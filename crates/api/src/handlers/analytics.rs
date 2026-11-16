//! `apps/builder/src/features/analytics/api/public.ts`: the ported
//! `analytics.*` routes (Q.D11 slice 8). No response cache (`withCache`
//! 120 s): every read is a fresh query.

use chatbotx_business::analytics::messages::ResponseAverages;
use chatbotx_business::analytics::{
    CountPoint, Filter, TimeRange, bot_messages, contacts, conversations, messages,
};
use chatbotx_db::repo;
use serde_json::{Value, json};

use super::input::{Input, Seg, Str, key};
use super::{ApiState, Call, Reply, internal_error};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "analytics.filterOptions",
    "analytics.contactCountsPerDay",
    "analytics.newContactCountsPerDay",
    "analytics.blockedContactsPerDay",
    "analytics.blockedContactsCount",
    "analytics.newContactsCount",
    "analytics.contactsCount",
    "analytics.activeContactsCount",
    "analytics.contactsByDimension",
    "analytics.macActiveContactCount",
    "analytics.messagesByAdmin",
    "analytics.humanAgentStats",
    "analytics.agentResponseSeries",
    "analytics.conversationHandoffs",
    "analytics.conversationFollowUps",
    "analytics.conversationArchived",
    "analytics.conversationAssigned",
    "analytics.conversationAssignedByAdmin",
    "analytics.uniqueConversationsByAdmin",
    "analytics.botMessagesByResult",
    "analytics.botMessagesWithResponse",
    "analytics.botMessagesNoResponse",
    "analytics.botMessagesAiProviders",
    "analytics.messagesBySender",
];

/// `MAX_ANALYTICS_FILTER_VALUES`.
const MAX_FILTER_VALUES: usize = 200;

/// Every value of a query key (`z.preprocess`: one value = a one-item
/// array; repeated keys build the array). `None` = absent.
fn query_list(call: &Call<'_>, name: &str) -> Option<Vec<String>> {
    let query = call.query?;
    let values: Vec<String> = url::form_urlencoded::parse(query.as_bytes())
        .filter(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .collect();
    (!values.is_empty()).then_some(values)
}

/// `analyticsFilterFields`.
fn filter(input: &mut Input, call: &Call<'_>) -> Filter {
    let mut channels = Vec::new();
    if let Some(values) = query_list(call, "channels") {
        let options = chatbotx_business::connection::providers::CHANNEL_TYPES;
        for (index, value) in values.iter().enumerate() {
            let path = [Seg::Key("channels".into()), Seg::Index(index)];
            if let Some(channel) =
                input.enum_at(&path, Some(&Value::String(value.clone())), options, true)
            {
                channels.push(channel);
            }
        }
        if values.len() > options.len() {
            input.issue(
                &key("channels"),
                "too_big",
                &format!("Too big: expected array to have <={} items", options.len()),
            );
        }
    }
    let mut inbox_ids = Vec::new();
    if let Some(values) = query_list(call, "inboxIds") {
        for (index, value) in values.iter().enumerate() {
            let path = [Seg::Key("inboxIds".into()), Seg::Index(index)];
            if let Some(id) = input.id_at(&path, Some(&Value::String(value.clone())), true) {
                inbox_ids.push(id);
            }
        }
        if values.len() > MAX_FILTER_VALUES {
            input.issue(
                &key("inboxIds"),
                "too_big",
                &format!("Too big: expected array to have <={MAX_FILTER_VALUES} items"),
            );
        }
    }
    Filter::new(channels, inbox_ids)
}

/// `timeRangePublicRequest`: `from`, `to`, `timezone` (default UTC) and the
/// filter. `new Date(text)` of an unparsable value is an Invalid Date.
fn time_range(input: &mut Input, call: &Call<'_>) -> Option<TimeRange> {
    let from = input.string("from", Str::default());
    let to = input.string("to", Str::default());
    let timezone = input.opt_string("timezone", Str::default());
    let filter = filter(input, call);
    Some(TimeRange::new(
        call.workspace_id,
        chatbotx_core::js::parse_date(&from?),
        chatbotx_core::js::parse_date(&to?),
        timezone.as_deref().unwrap_or("UTC"),
        filter,
    ))
}

fn series(points: Vec<CountPoint>) -> Value {
    json!({
        "data": points
            .into_iter()
            .map(|p| json!({ "date": p.date, "count": p.count }))
            .collect::<Vec<_>>()
    })
}

async fn read<R: Send + 'static>(
    state: &ApiState,
    f: impl FnOnce(&chatbotx_db::rusqlite::Connection) -> repo::Result<R> + Send + 'static,
) -> Result<R, crate::orpc::OrpcError> {
    state
        .db
        .read(move |conn| f(conn).map_err(|e| chatbotx_db::DbError::other(e.to_string())))
        .await
        .map_err(|e| internal_error(&e.to_string()))
}

fn ok(body: Value) -> Reply {
    Ok(super::json(200, body))
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let ws = call.workspace_id;
    match route_id {
        "analytics.filterOptions" => {
            let rows = read(state, move |conn| {
                repo::inbox::list_analytics_options_by_workspace(conn, ws)
            })
            .await?;
            // Only known channels are offered (`Inbox.channel` is plain text).
            let channels = chatbotx_business::connection::providers::CHANNEL_TYPES;
            let data: Vec<Value> = rows
                .into_iter()
                .filter(|row| channels.contains(&row.channel.as_str()))
                .map(|row| json!({ "id": row.id.to_string(), "name": row.name, "channel": row.channel }))
                .collect();
            ok(json!({ "data": data }))
        }
        "analytics.macActiveContactCount" => {
            let now = repo::now_ms();
            let mac = read(state, move |conn| {
                contacts::current_mac_count(conn, ws, now)
            })
            .await?;
            ok(json!({ "data": { "macCount": mac } }))
        }
        "analytics.contactsByDimension" => {
            let range = time_range(&mut input, &call);
            let dimension = {
                let value = input.get("dimension").cloned();
                input.enum_at(
                    &key("dimension"),
                    value.as_ref(),
                    contacts::DIMENSIONS,
                    true,
                )
            };
            let metric = {
                let value = input.get("metric").cloned();
                input.enum_at(&key("metric"), value.as_ref(), contacts::METRICS, false)
            }
            .unwrap_or_else(|| "new".into());
            if !input.failed()
                && let (Some(range), Some(dimension)) = (&range, &dimension)
                && !contacts::is_supported_dimension_metric(dimension, &metric, &range.filter)
            {
                input.issue(
                    &key("metric"),
                    "custom",
                    "`metric: total` is only supported for the channel and source dimensions, and only for channel with `inboxIds`",
                );
            }
            input.finish()?;
            let (Some(range), Some(dimension)) = (range, dimension) else {
                return Err(internal_error("validated input missing"));
            };
            let rows = read(state, move |conn| {
                contacts::contacts_by_dimension(conn, &range, &dimension, &metric)
            })
            .await?;
            let data: Vec<Value> = rows
                .into_iter()
                .map(|row| {
                    json!({ "dimension": row.dimension, "count": row.count, "uniqueContacts": row.unique_contacts })
                })
                .collect();
            ok(json!({ "data": data }))
        }
        "analytics.botMessagesByResult"
        | "analytics.botMessagesWithResponse"
        | "analytics.botMessagesNoResponse"
        | "analytics.messagesBySender" => {
            let range = time_range(&mut input, &call);
            let options: &[&str] = if route_id == "analytics.messagesBySender" {
                &["day", "month"]
            } else {
                bot_messages::GRANULARITIES
            };
            let granularity = {
                let value = input.get("granularity").cloned();
                input.enum_at(&key("granularity"), value.as_ref(), options, false)
            }
            .unwrap_or_else(|| "day".into());
            input.finish()?;
            let Some(range) = range else {
                return Err(internal_error("validated input missing"));
            };
            if route_id == "analytics.messagesBySender" {
                let month = granularity == "month";
                let rows = read(state, move |conn| {
                    messages::messages_by_sender(conn, &range, month)
                })
                .await?;
                let data: Vec<Value> = rows
                    .into_iter()
                    .map(|r| {
                        json!({ "timestamp": r.timestamp, "channel": r.channel, "senderType": r.sender_type, "count": r.count })
                    })
                    .collect();
                return ok(json!({ "data": data }));
            }
            let route = route_id.to_string();
            let rows = read(state, move |conn| match route.as_str() {
                "analytics.botMessagesByResult" => {
                    bot_messages::by_result(conn, &range, &granularity)
                }
                "analytics.botMessagesWithResponse" => {
                    bot_messages::with_response(conn, &range, &granularity)
                }
                _ => bot_messages::no_response(conn, &range, &granularity),
            })
            .await?;
            let data: Vec<Value> = rows
                .into_iter()
                .map(|r| {
                    let mut row = json!({
                        "timestamp": r.timestamp,
                        "hasResponse": r.has_response,
                        "responseType": r.response_type,
                        "aiProvider": r.ai_provider,
                        "count": r.count,
                    });
                    if let Some(result) = r.result {
                        row["result"] = json!(result);
                    }
                    row
                })
                .collect();
            ok(json!({ "data": data }))
        }
        _ => {
            let range = time_range(&mut input, &call);
            input.finish()?;
            let Some(range) = range else {
                return Err(internal_error("validated input missing"));
            };
            match route_id {
                "analytics.contactCountsPerDay" => {
                    let points = read(state, move |conn| {
                        contacts::contact_counts_series(conn, &range)
                    })
                    .await?;
                    ok(series(points))
                }
                "analytics.newContactCountsPerDay" | "analytics.blockedContactsPerDay" => {
                    let event = if route_id == "analytics.newContactCountsPerDay" {
                        "contact_created"
                    } else {
                        "contact_blocked"
                    };
                    let points = read(state, move |conn| {
                        contacts::distinct_contacts_series(conn, &range, event)
                    })
                    .await?;
                    ok(series(points))
                }
                "analytics.newContactsCount" | "analytics.blockedContactsCount" => {
                    let event = if route_id == "analytics.newContactsCount" {
                        "contact_created"
                    } else {
                        "contact_blocked"
                    };
                    let count = read(state, move |conn| {
                        contacts::distinct_contacts_count(conn, &range, event)
                    })
                    .await?;
                    ok(json!({ "data": { "count": count } }))
                }
                "analytics.contactsCount" => {
                    let count = read(state, move |conn| {
                        contacts::contacts_count(conn, range.workspace_id, &range.filter)
                    })
                    .await?;
                    ok(json!({ "data": { "count": count } }))
                }
                "analytics.activeContactsCount" => {
                    let count = read(state, move |conn| {
                        contacts::active_contacts_count(conn, &range)
                    })
                    .await?;
                    ok(json!({ "data": { "count": count } }))
                }
                "analytics.conversationHandoffs"
                | "analytics.conversationFollowUps"
                | "analytics.conversationArchived"
                | "analytics.conversationAssigned" => {
                    let route = route_id.to_string();
                    let rows = read(state, move |conn| match route.as_str() {
                        "analytics.conversationHandoffs" => {
                            conversations::handoffs_by_day(conn, &range)
                        }
                        "analytics.conversationFollowUps" => {
                            conversations::events_by_day(conn, &range, "conversation_followed")
                        }
                        "analytics.conversationArchived" => {
                            conversations::events_by_day(conn, &range, "conversation_archived")
                        }
                        _ => conversations::events_by_day(conn, &range, "conversation_assigned"),
                    })
                    .await?;
                    let data: Vec<Value> = rows
                        .into_iter()
                        .map(|r| {
                            let mut row = json!({ "timestamp": r.timestamp, "count": r.count });
                            if let Some(direction) = r.direction {
                                row["direction"] = json!(direction);
                            }
                            row
                        })
                        .collect();
                    ok(json!({ "data": data }))
                }
                "analytics.conversationAssignedByAdmin"
                | "analytics.uniqueConversationsByAdmin" => {
                    let distinct = route_id == "analytics.uniqueConversationsByAdmin";
                    let rows = read(state, move |conn| {
                        conversations::assigned_by_admin(conn, &range, distinct)
                    })
                    .await?;
                    let data: Vec<Value> = rows
                        .into_iter()
                        .map(|r| {
                            member(
                                json!({ "toAssignee": r.user_id.to_string(), "count": r.count }),
                                r.user_name,
                                r.user_email,
                            )
                        })
                        .collect();
                    ok(json!({ "data": data }))
                }
                "analytics.messagesByAdmin" => {
                    let rows =
                        read(state, move |conn| messages::messages_by_admin(conn, &range)).await?;
                    let data: Vec<Value> = rows
                        .into_iter()
                        .map(|r| {
                            member(
                                json!({ "adminId": r.admin_id.to_string(), "count": r.count }),
                                r.user_name,
                                r.user_email,
                            )
                        })
                        .collect();
                    ok(json!({ "data": data }))
                }
                "analytics.humanAgentStats" => {
                    let rows =
                        read(state, move |conn| messages::human_agent_stats(conn, &range)).await?;
                    let data: Vec<Value> = rows
                        .into_iter()
                        .map(|r| {
                            let mut row = json!({
                                "adminId": r.admin_id.to_string(),
                                "assignedConversations": r.assigned_conversations,
                                "messagesSent": r.messages_sent,
                                "uniqueContacts": r.unique_contacts,
                            });
                            averages(&mut row, &r.response);
                            member(row, r.user_name, r.user_email)
                        })
                        .collect();
                    ok(json!({ "data": data }))
                }
                "analytics.agentResponseSeries" => {
                    let rows = read(state, move |conn| {
                        messages::agent_response_series(conn, &range)
                    })
                    .await?;
                    let data: Vec<Value> = rows
                        .into_iter()
                        .map(|r| {
                            let mut row =
                                json!({ "adminId": r.admin_id.to_string(), "date": r.date });
                            averages(&mut row, &r.response);
                            member(row, r.user_name, r.user_email)
                        })
                        .collect();
                    ok(json!({ "data": data }))
                }
                "analytics.botMessagesAiProviders" => {
                    let rows =
                        read(state, move |conn| bot_messages::ai_providers(conn, &range)).await?;
                    let data: Vec<Value> = rows
                        .into_iter()
                        .map(|(provider, count, share)| {
                            json!({ "aiProvider": provider, "count": count, "percentage": share })
                        })
                        .collect();
                    ok(json!({ "data": data }))
                }
                _ => Err(crate::routes::not_ported_error()),
            }
        }
    }
}

/// `userName` / `userEmail` (`?? undefined`: absent when null).
fn member(mut row: Value, name: Option<String>, email: Option<String>) -> Value {
    if let Some(name) = name {
        row["userName"] = json!(name);
    }
    if let Some(email) = email {
        row["userEmail"] = json!(email);
    }
    row
}

/// The `AgentResponseAverages` fields of a row.
fn averages(row: &mut Value, r: &ResponseAverages) {
    for (key, value) in [
        ("avgResponseSeconds", json!(r.avg_response_seconds)),
        (
            "avgWorkingResponseSeconds",
            json!(r.avg_working_response_seconds),
        ),
        (
            "avgFirstResponseSeconds",
            json!(r.avg_first_response_seconds),
        ),
        (
            "avgWorkingFirstResponseSeconds",
            json!(r.avg_working_first_response_seconds),
        ),
        ("responseCount", json!(r.response_count)),
        ("firstResponseCount", json!(r.first_response_count)),
        (
            "responseSlaEligibleCount",
            json!(r.response_sla_eligible_count),
        ),
        ("responseSlaMetCount", json!(r.response_sla_met_count)),
        ("responseSlaMetPercent", json!(r.response_sla_met_percent())),
        (
            "firstResponseSlaEligibleCount",
            json!(r.first_response_sla_eligible_count),
        ),
        (
            "firstResponseSlaMetCount",
            json!(r.first_response_sla_met_count),
        ),
        (
            "firstResponseSlaMetPercent",
            json!(r.first_response_sla_met_percent()),
        ),
    ] {
        row[key] = value;
    }
}
