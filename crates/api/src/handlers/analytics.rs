//! `apps/builder/src/features/analytics/api/public.ts`: the ported
//! `analytics.*` routes (Q.D11 slice 8). No response cache (`withCache`
//! 120 s): every read is a fresh query.

use chatbotx_business::analytics::messages::ResponseAverages;
use chatbotx_business::analytics::{
    CountPoint, Filter, TimeRange, bot_messages, campaigns, comment_automation, contacts,
    conversations, flows, links, messages,
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
    "analytics.magicLinkStats",
    "analytics.magicLinkContacts",
    "analytics.refLinkStats",
    "analytics.refLinkContacts",
    "analytics.flowStats",
    "analytics.resetFlowStats",
    "analytics.flowSmartDelayStats",
    "analytics.broadcastStats",
    "analytics.sequenceStepStats",
    "analytics.commentAutomationReplyStats",
    "analytics.commentAutomationUserComments",
    "analytics.commentAutomationBotReplies",
    "analytics.commentAutomationErrors",
    "analytics.commentAutomationContacts",
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

/// `withLegacyDateRangeAliases`: `startDate` / `endDate` stand in for a
/// missing `from` / `to`.
fn legacy_range(input: &mut Input) {
    for (name, legacy) in [("from", "startDate"), ("to", "endDate")] {
        if let Some(value) = input.obj.remove(legacy) {
            input.obj.entry(name).or_insert(value);
        }
    }
}

/// A link id or date the original hands to Postgres as is: a value that
/// does not cast fails the query (500).
fn cast<T>(value: Option<T>) -> Result<T, crate::orpc::OrpcError> {
    value.ok_or_else(|| internal_error("invalid input syntax"))
}

fn link_kind(route_id: &str) -> links::Link {
    if route_id.starts_with("analytics.magicLink") {
        links::Link::Magic
    } else {
        links::Link::Ref
    }
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

/// `commentAutomationRangePublicRequest`: the automation id
/// (`zodBigintAsString`), the `from` / `to` ISO datetimes and the IANA
/// timezone. `None` once an issue is recorded.
fn comment_range(input: &mut Input, ws: i64) -> Option<(TimeRange, i64)> {
    let automation_id = automation_id(input);
    let from = super::polls::datetime_at(input, "from", true);
    let to = super::polls::datetime_at(input, "to", true);
    let path = key("timezone");
    let value = input.get("timezone").cloned();
    let timezone = input.string_at(&path, value.as_ref(), Str::trimmed(1, 255), true);
    if let Some(zone) = &timezone
        && !chatbotx_core::utils::tz::is_intl_time_zone(zone)
    {
        input.issue(&path, "custom", "Invalid IANA timezone");
    }
    // `startDate` / `endDate` as given: no floor clamp on these reads.
    Some((
        TimeRange {
            workspace_id: ws,
            from: Some(from?),
            to: Some(to?),
            tz: chatbotx_business::analytics::zone(&timezone?),
            filter: Filter::default(),
        },
        automation_id?,
    ))
}

/// `zodBigintAsString()` at `automationId`: digits only. A value past `i64`
/// is `None` with no issue: the original's bigint cast fails (500).
fn automation_id(input: &mut Input) -> Option<i64> {
    let path = key("automationId");
    let value = input.get("automationId").cloned();
    let text = input.string_at(&path, value.as_ref(), Str::default(), true)?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        input.issue(
            &path,
            "invalid_format",
            "Invalid string: must match pattern /^\\d+$/",
        );
        return None;
    }
    text.parse().ok()
}

/// `commentAutomationListPublicRequest`.
fn comment_list(input: &mut Input, ws: i64) -> Option<comment_automation::ListInput> {
    let range = comment_range(input, ws);
    let (page, per_page) = super::qr_codes::public_paging(input);
    let keyword = input.opt_string("keyword", Str::default());
    let (range, automation_id) = range?;
    let (from, to) = range.bounds()?;
    Some(comment_automation::ListInput {
        workspace_id: ws,
        automation_id,
        from,
        to,
        page,
        per_page,
        keyword,
    })
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
        "analytics.broadcastStats" | "analytics.sequenceStepStats" => {
            let ids: Vec<Option<String>> = if route_id == "analytics.broadcastStats" {
                vec![input.string("broadcastId", Str::default())]
            } else {
                vec![
                    input.string("sequenceId", Str::default()),
                    input.string("stepId", Str::default()),
                ]
            };
            input.finish()?;
            let ids = ids
                .into_iter()
                .map(|id| cast(id.and_then(|id| id.parse::<i64>().ok())))
                .collect::<Result<Vec<i64>, _>>()?;
            let stats = read(state, move |conn| match ids[..] {
                [broadcast] => campaigns::broadcast_stats(conn, ws, broadcast),
                [sequence, step, ..] => campaigns::sequence_step_stats(conn, ws, sequence, step),
                _ => Ok(campaigns::DeliveryStats::default()),
            })
            .await?;
            ok(json!({
                "message:sent": stats.sent(),
                "message:delivered": stats.delivered,
                "message:seen": stats.seen,
                "flow:clicked": stats.clicked,
                "message:failed": stats.failed,
            }))
        }
        "analytics.flowStats" | "analytics.resetFlowStats" | "analytics.flowSmartDelayStats" => {
            let flow_id = input.string("flowId", Str::default());
            input.finish()?;
            let flow_id = cast(flow_id.and_then(|id| id.parse::<i64>().ok()))?;
            match route_id {
                "analytics.flowStats" => {
                    let nodes =
                        read(state, move |conn| flows::flow_stats(conn, ws, flow_id)).await?;
                    let mut out = serde_json::Map::new();
                    for (id, n) in nodes {
                        let buttons: serde_json::Map<String, Value> = n
                            .buttons
                            .into_iter()
                            .map(|(b, clicks)| {
                                (b.clone(), json!({ "buttonId": b, "clicks": clicks }))
                            })
                            .collect();
                        out.insert(
                            id,
                            json!({
                                "node": {
                                    "message:sent": n.sent,
                                    "message:seen": n.seen,
                                    "message:delivered": n.delivered,
                                    "flow:clicked": { "clicked": n.clicked, "totalUsers": n.delivered },
                                    "message:failed": n.failed,
                                },
                                "buttons": buttons,
                            }),
                        );
                    }
                    ok(Value::Object(out))
                }
                "analytics.resetFlowStats" => {
                    let now = repo::now_ms();
                    let id = chatbotx_core::utils::id::create_id()
                        .parse::<i64>()
                        .unwrap_or_default();
                    state
                        .db
                        .write(move |tx| {
                            flows::reset_stats_session(tx, ws, flow_id, now, id)
                                .map_err(|e| chatbotx_db::DbError::other(e.to_string()))
                        })
                        .await
                        .map_err(|e| internal_error(&e.to_string()))?;
                    Ok(super::no_content())
                }
                _ => {
                    let flow = chatbotx_business::flow::find_by_id(&state.db, ws, flow_id)
                        .await
                        .map_err(|e| super::business_error(e, super::FINDING))?;
                    let Some(draft) = flow.flow_versions.into_iter().find(|v| v.is_draft) else {
                        return Err(super::business_error(
                            chatbotx_business::BusinessError::not_found(
                                "Flow has no draft version",
                            ),
                            super::FINDING,
                        ));
                    };
                    let nodes = draft.nodes.0.as_array().cloned().unwrap_or_default();
                    let rows = read(state, move |conn| {
                        flows::smart_delay_stats(conn, ws, flow_id, &nodes)
                    })
                    .await?;
                    let out: serde_json::Map<String, Value> = rows
                        .into_iter()
                        .map(|(id, c)| (id, json!({ "waiting": c.waiting, "sent": c.sent })))
                        .collect();
                    ok(Value::Object(out))
                }
            }
        }
        "analytics.magicLinkStats" | "analytics.refLinkStats" => {
            legacy_range(&mut input);
            let from = input.string("from", Str::default());
            let to = input.string("to", Str::default());
            let link_id = input.string("linkId", Str::default());
            let timezone = input.string("timezone", Str::default());
            input.finish()?;
            let (Some(from), Some(to), Some(link_id), Some(timezone)) =
                (from, to, link_id, timezone)
            else {
                return Err(internal_error("validated input missing"));
            };
            let bounds = (
                cast(chatbotx_core::js::parse_date(&from))?,
                cast(chatbotx_core::js::parse_date(&to))?,
            );
            let link_id = cast(link_id.parse::<i64>().ok())?;
            let link = link_kind(route_id);
            let rows = read(state, move |conn| {
                links::stats_by_day(conn, link, ws, link_id, bounds, &timezone)
            })
            .await?;
            let data: Vec<Value> = rows
                .into_iter()
                .map(|(day, count)| json!({ "dateReport": day, "count": count }))
                .collect();
            ok(json!({ "data": data }))
        }
        "analytics.magicLinkContacts" | "analytics.refLinkContacts" => {
            legacy_range(&mut input);
            let link_id = input.string("linkId", Str::default());
            let from = input.opt_string("from", Str::default());
            let to = input.opt_string("to", Str::default());
            // Accepted for compatibility; the contact list does not use it.
            input.opt_string("timezone", Str::default());
            let (page, per_page) = super::qr_codes::public_paging(&mut input);
            input.finish()?;
            let Some(link_id) = link_id else {
                return Err(internal_error("validated input missing"));
            };
            // `startDate && endDate`: the range needs both, non-empty.
            let range = match (from.filter(|v| !v.is_empty()), to.filter(|v| !v.is_empty())) {
                (Some(from), Some(to)) => Some((
                    cast(chatbotx_core::js::parse_date(&from))?,
                    cast(chatbotx_core::js::parse_date(&to))?,
                )),
                _ => None,
            };
            // `if (!linkId)`: an empty id is an empty first page.
            let link_id = if link_id.is_empty() {
                None
            } else {
                Some(cast(link_id.parse::<i64>().ok())?)
            };
            let link = link_kind(route_id);
            let list = read(state, move |conn| {
                links::contacts(conn, link, ws, link_id, page, per_page, range)
            })
            .await?;
            let data: Vec<Value> = list
                .data
                .into_iter()
                .map(|r| {
                    json!({
                        "contactId": r.contact_id,
                        "contactInboxId": r.contact_inbox_id,
                        "sourceId": r.source_id,
                        "channel": r.channel,
                        "conversationId": r.conversation_id,
                        "occurredAt": r.occurred_at,
                    })
                })
                .collect();
            ok(
                json!({ "data": data, "total": list.total, "page": list.page, "pageCount": list.page_count }),
            )
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
        "analytics.commentAutomationReplyStats" => {
            let range = comment_range(&mut input, ws);
            input.finish()?;
            let Some((range, automation_id)) = range else {
                return Err(internal_error("validated input missing"));
            };
            let rows = read(state, move |conn| {
                comment_automation::replies_by_date(conn, &range, automation_id)
            })
            .await?;
            let data: Vec<Value> = rows
                .into_iter()
                .map(|(day, count)| json!({ "dateReport": day, "count": count }))
                .collect();
            ok(json!({ "data": data }))
        }
        "analytics.commentAutomationUserComments" | "analytics.commentAutomationBotReplies" => {
            let list = comment_list(&mut input, ws);
            input.finish()?;
            let Some(list) = list else {
                return Err(internal_error("validated input missing"));
            };
            let user = route_id == "analytics.commentAutomationUserComments";
            let page = read(state, move |conn| {
                if user {
                    comment_automation::user_comments(conn, &list)
                } else {
                    comment_automation::bot_replies(conn, &list)
                }
            })
            .await?;
            let data: Vec<Value> = page
                .data
                .into_iter()
                .map(|r| json!({ "text": r.text, "total": r.total }))
                .collect();
            ok(
                json!({ "data": data, "total": page.total, "page": page.page, "pageCount": page.page_count }),
            )
        }
        "analytics.commentAutomationErrors" => {
            let list = comment_list(&mut input, ws);
            input.finish()?;
            let Some(list) = list else {
                return Err(internal_error("validated input missing"));
            };
            let page = read(state, move |conn| comment_automation::errors(conn, &list)).await?;
            let data: Vec<Value> = page
                .data
                .into_iter()
                .map(|r| {
                    json!({
                        "id": r.id.to_string(),
                        "replyChannel": r.reply_channel,
                        "replyType": r.reply_type,
                        "errorDetail": r.error_detail,
                        "httpCode": r.http_code,
                        "commentText": r.comment_text,
                        "contact": r.contact.map(|(first, last, avatar)| {
                            json!({ "firstName": first, "lastName": last, "avatar": avatar })
                        }),
                        "occurredAt": r.occurred_at,
                    })
                })
                .collect();
            ok(
                json!({ "data": data, "total": page.total, "page": page.page, "pageCount": page.page_count }),
            )
        }
        "analytics.commentAutomationContacts" => {
            let automation_id = automation_id(&mut input);
            let value = input.get("eventType").cloned();
            let event_type = input.enum_at(
                &key("eventType"),
                value.as_ref(),
                comment_automation::EVENT_TYPES,
                true,
            );
            let (page, per_page) = super::qr_codes::public_paging(&mut input);
            input.finish()?;
            let (Some(automation_id), Some(event_type)) = (automation_id, event_type) else {
                return Err(internal_error("validated input missing"));
            };
            let list = read(state, move |conn| {
                let Some((kind, total)) =
                    comment_automation::find(conn, ws, automation_id, &event_type)?
                else {
                    return Ok(None);
                };
                comment_automation::contacts(
                    conn,
                    ws,
                    automation_id,
                    &kind,
                    &event_type,
                    total,
                    page,
                    per_page,
                )
                .map(Some)
            })
            .await?;
            // `commentAutomationService.findOrFail`: 404 for an unknown or
            // foreign id.
            let Some(list) = list else {
                return Err(super::business_error(
                    chatbotx_business::BusinessError::not_found("Comment Automation not found"),
                    super::FINDING,
                ));
            };
            let data: Vec<Value> = list
                .data
                .into_iter()
                .map(|r| {
                    json!({
                        "rowKey": r.row_key,
                        "contactId": r.contact_id,
                        "contactInboxId": r.contact_inbox_id,
                        "sourceId": r.source_id,
                        "channel": r.channel,
                        "conversationId": r.conversation_id,
                        "errorContent": r.error_content,
                        "occurredAt": r.occurred_at,
                        "commentText": r.comment_text,
                        "missReason": r.miss_reason,
                    })
                })
                .collect();
            ok(json!({
                "data": data,
                "total": list.total,
                "contactTotal": list.contact_total,
                "page": list.page,
                "pageCount": list.page_count,
            }))
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
