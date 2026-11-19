//! Broadcasts: `features/broadcasts/api/private.ts` and
//! `features/broadcasts/actions/*` (Q.D5). The batch stats and contacts reads
//! need the delivery analytics (Q.D11); the contacts list is ported (Q.W2).

use chatbotx_business::BusinessError;
use chatbotx_business::broadcast::{self as service, history, recurrence, write};
use chatbotx_db::repo::now_ms;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid};
use crate::handlers::broadcasts::{
    coerce_date, parse_action_payload, recurrence_rule, resolve_schedule_time, run_date,
    schedule_request, send_rate, submitted_rate,
};
use crate::orpc::OrpcError;
use chatbotx_core::flow_config::z;

/// The ported actions.
pub const ACTIONS: &[&str] = &[
    "cloneBroadcastAction",
    "createBroadcastAction",
    "deleteBroadcastAction",
    "deleteBroadcastsAction",
    "moveBroadcastToDraftAction",
    "resendBroadcastAction",
    "resumeBroadcastAction",
    "scheduleBroadcastAction",
    "setBroadcastRunSkippedAction",
    "stopBroadcastAction",
    "updateBroadcastAction",
    "updateDraftBroadcastAction",
    // Server-component queries of the broadcast pages (P4 spec §5.1).
    "listBroadcasts",
    "listBroadcastsForCalendar",
    "findDraftBroadcast",
    "findRecurringBroadcast",
    "findIntegrationWhatsappInboxId",
];

/// The page queries (no next-safe-action wrapper: a plain value or error).
const QUERIES: &[&str] = &[
    "listBroadcasts",
    "listBroadcastsForCalendar",
    "findDraftBroadcast",
    "findRecurringBroadcast",
    "findIntegrationWhatsappInboxId",
];

/// `TEMPLATE_BORROWED_STATUSES` of `features/broadcasts/queries`.
const TEMPLATE_BORROWED_STATUSES: &[&str] = &["cancelled", "sent"];

/// A nuqs enum param: an unknown value parses as `null`.
fn known(input: &In<'_>, key: &str, values: &[&str]) -> Option<String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| values.contains(v))
        .map(str::to_owned)
}

/// `listBroadcasts` (`features/broadcasts/queries/index.ts`): one-off
/// broadcasts and repeating series, never the runs of a series.
async fn list_page(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    use chatbotx_db::repo::partials::broadcast::BROADCAST_STATUSES;
    let input = In(input);
    let repeating = input.get("kind").and_then(Value::as_str) == Some("repeating");
    let status = known(&input, "status", BROADCAST_STATUSES);
    let exclude_templates =
        !repeating && TEMPLATE_BORROWED_STATUSES.contains(&status.as_deref().unwrap_or(""));
    let list = chatbotx_db::repo::broadcast::BroadcastListInput {
        workspace_id: ctx.workspace_id,
        name: input.opt_str("name")?.filter(|n| !n.is_empty()),
        status,
        page: input.opt_positive("page")?,
        per_page: input.opt_positive("perPage")?,
        sort: input.sort(),
        channel: None,
        scheduled_from: None,
        scheduled_to: None,
        include_runs: Some(false),
        exclude_templates: Some(exclude_templates),
        parent_broadcast_id: None,
        schedules_type: repeating.then(|| "recurring".to_owned()),
    };
    service::list(&ctx.api.db, list).await.map_err(fail)
}

/// `listBroadcastsForCalendar`: the SPA sends the UTC window it computed
/// with the builder's `getCalendarQueryRange` (`from` / `to` ISO strings).
async fn calendar(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    use chatbotx_db::repo::partials::broadcast::BROADCAST_STATUSES;
    let at = |key: &str| {
        input
            .get(key)
            .filter(|v| v.is_string())
            .and_then(coerce_date)
            .ok_or_else(invalid)
    };
    let input_ref = In(input);
    let rows = service::audience::list_for_calendar(
        &ctx.api.db,
        service::audience::CalendarInput {
            workspace_id: ctx.workspace_id,
            from: at("from")?,
            to: at("to")?,
            status: known(&input_ref, "status", BROADCAST_STATUSES),
            schedules_type: (input_ref.get("kind").and_then(Value::as_str) == Some("repeating"))
                .then(|| "recurring".to_owned()),
            name: input_ref.opt_str("name")?,
        },
    )
    .await
    .map_err(fail)?;
    Ok(Value::Array(rows))
}

/// `broadcastService.findDraft`: a `draft` row of the workspace with its
/// targets, repeat rule and the page of its integration (`null` = none).
async fn find_draft(ctx: &Ctx<'_>, id: i64) -> Result<Value, OrpcError> {
    use chatbotx_db::repo::models::{Broadcast, BroadcastRecurrence, BroadcastTarget};
    use chatbotx_db::repo::{Error, find_many, find_one};
    use chatbotx_db::rusqlite::OptionalExtension;
    let ws = ctx.workspace_id;
    chatbotx_business::tx::read(&ctx.api.db, move |conn| {
        let Some(row) = find_one::<Broadcast>(
            conn,
            r#""id" = ?1 AND "workspaceId" = ?2 AND "status" = 'draft' AND "deletedAt" IS NULL"#,
            &[&id, &ws],
        )?
        else {
            return Ok(Value::Null);
        };
        let targets = find_many::<BroadcastTarget>(conn, r#"WHERE "broadcastId" = ?1"#, &[&id])?;
        let recurrence = find_one::<BroadcastRecurrence>(conn, r#""broadcastId" = ?1"#, &[&id])?;
        let inbox_of = |table: &str, integration: Option<i64>| -> Result<Value, Error> {
            let Some(integration) = integration else {
                return Ok(Value::Null);
            };
            let inbox: Option<i64> = conn
                .query_row(
                    &format!(r#"SELECT "inboxId" FROM "{table}" WHERE "id" = ?1"#),
                    [integration],
                    |r| r.get(0),
                )
                .optional()?;
            Ok(inbox.map_or(Value::Null, |i| json!({ "inboxId": i.to_string() })))
        };
        let mut out = row.to_json();
        out["targets"] = Value::Array(targets.iter().map(|t| t.to_json()).collect());
        out["recurrence"] = recurrence.map_or(
            Value::Null,
            |r| json!({ "rule": r.to_json()["rule"], "occurrenceCount": r.occurrence_count }),
        );
        out["integrationWhatsapp"] = inbox_of("IntegrationWhatsapp", row.integration_whatsapp_id)?;
        out["integrationMessenger"] =
            inbox_of("IntegrationMessenger", row.integration_messenger_id)?;
        Ok(out)
    })
    .await
    .map_err(fail)
}

/// The broadcast page queries ([`QUERIES`]); `[workspaceId, input]`.
async fn query(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let input = arg(args, 1);
    match name {
        "listBroadcasts" => list_page(ctx, input).await,
        "listBroadcastsForCalendar" => calendar(ctx, input).await,
        "findDraftBroadcast" => {
            let Ok(id) = In(input).id("broadcastId") else {
                return Ok(Value::Null);
            };
            find_draft(ctx, id).await
        }
        // `findByIdOrName` of the runs page: only a repeating series
        // resolves (`null` otherwise, the page's 404).
        "findRecurringBroadcast" => {
            let id = In(input).str("broadcastId", false, 1, usize::MAX)?;
            match service::find_by_id_or_name(&ctx.api.db, ctx.workspace_id, id).await {
                Ok(found) if !found["recurrence"].is_null() => Ok(found),
                Ok(_) => Ok(Value::Null),
                Err(BusinessError::Exception { code, .. }) if code == "notFound" => Ok(Value::Null),
                Err(error) => Err(fail(error)),
            }
        }
        // `integrationWhatsappService.findByIdForWorkspace(...).inboxId` of
        // the create page's Ads deep link (`null` = preselect nothing).
        _ => {
            let Ok(id) = In(input).id("integrationWhatsappId") else {
                return Ok(Value::Null);
            };
            let ws = ctx.workspace_id;
            chatbotx_business::tx::read(&ctx.api.db, move |conn| {
                Ok(
                    chatbotx_db::repo::integration_whatsapp::find_by_id_for_workspace(
                        conn, id, ws,
                    )?
                    .map_or(Value::Null, |i| Value::String(i.inbox_id.to_string())),
                )
            })
            .await
            .map_err(fail)
        }
    }
}

const CHANNELS: &[&str] = &[
    "omnichannel",
    "webchat",
    "messenger",
    "whatsapp",
    "zalo",
    "smtp",
    "telegram",
    "instagram",
    "threads",
    "tiktok",
    "api",
];

/// A `zodBigintAsString` bind arg.
fn bound_id(value: &Value) -> Result<i64, OrpcError> {
    match value {
        Value::String(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(text.parse::<i64>().unwrap_or(-1))
        }
        _ => Err(invalid()),
    }
}

/// `withBroadcastPlanLimitOutcome`: a plan limit is `{ outcome: "planLimit", limit }`.
fn plan_limit_outcome(result: Result<Value, BusinessError>) -> Result<Value, OrpcError> {
    match result {
        Err(BusinessError::Exception {
            code,
            data: Some(data),
            ..
        }) if code == "broadcastPlanLimit" => Ok(json!({ "outcome": "planLimit", "limit": data })),
        other => other.map_err(action_fail),
    }
}

fn parsed(schema: z::Schema, value: &Value) -> Result<Value, OrpcError> {
    let parsed = schema.safe_parse(value);
    parsed.data.filter(|_| parsed.success).ok_or_else(invalid)
}

/// `previewRecurrencePublicRequest` (`count` defaults to 3).
fn preview(input: &Value) -> Result<Value, OrpcError> {
    let data = parsed(
        z::object(vec![
            ("recurrence", recurrence_rule()),
            ("count", z::number().int().gte(1.0).lte(10.0).optional()),
        ]),
        input,
    )?;
    let count = data["count"].as_u64().map_or(3, |c| c as usize);
    recurrence::preview_upcoming(&data["recurrence"], count, now_ms()).map_err(fail)
}

fn opt_str(data: &Value, key: &str) -> Option<String> {
    data[key].as_str().map(str::to_owned)
}

/// `listBroadcastRunGroupsPublicRequest`.
async fn run_groups(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let data = parsed(
        z::object(vec![
            ("idOrName", z::string()),
            ("groupBy", z::enum_(&["day", "week", "month"]).optional()),
            ("timezone", z::string().optional()),
            (
                "limit",
                z::number().coerce().int().gte(1.0).lte(31.0).optional(),
            ),
            ("cursor", z::string().optional()),
        ]),
        input,
    )?;
    let granularity =
        history::granularity_of(data["groupBy"].as_str().unwrap_or("day")).ok_or_else(invalid)?;
    history::list_run_groups(
        &ctx.api.db,
        ctx.workspace_id,
        opt_str(&data, "idOrName").unwrap_or_default(),
        granularity,
        opt_str(&data, "timezone"),
        data["limit"].as_i64().unwrap_or(31),
        opt_str(&data, "cursor"),
    )
    .await
    .map_err(fail)
}

/// `listBroadcastOccurrencesPublicRequest`.
async fn occurrences(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let data = parsed(
        z::object(vec![
            ("idOrName", z::string()),
            (
                "limit",
                z::number().coerce().int().gte(1.0).lte(100.0).optional(),
            ),
            ("cursor", z::string().optional()),
        ]),
        input,
    )?;
    let date = |key: &str| match input.get(key) {
        None => Ok(None),
        Some(value) => coerce_date(value).map(Some).ok_or_else(invalid),
    };
    history::list_occurrences(
        &ctx.api.db,
        ctx.workspace_id,
        opt_str(&data, "idOrName").unwrap_or_default(),
        date("from")?,
        date("to")?,
        opt_str(&data, "cursor"),
        data["limit"].as_i64().unwrap_or(50),
    )
    .await
    .map_err(fail)
}

/// `privateListBroadcastContactsAPI` (`listBroadcastContactsRequest`): no
/// `eventType` is an empty page of a live broadcast.
async fn contacts(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let data = parsed(
        z::object(vec![
            ("broadcastId", z::string()),
            (
                "eventType",
                z::enum_(crate::handlers::broadcasts::BROADCAST_EVENT_TYPES).optional(),
            ),
            ("page", z::number().optional()),
            ("perPage", z::number().optional()),
        ]),
        input,
    )?;
    let id = bound_id(&data["broadcastId"])
        .map_err(|_| fail(BusinessError::not_found("Broadcast not found")))?;
    let page = data["page"].as_f64().unwrap_or(1.0) as i64;
    let per_page = data["perPage"].as_f64().unwrap_or(20.0) as i64;
    let Some(event_type) = opt_str(&data, "eventType") else {
        let ws = ctx.workspace_id;
        let live = chatbotx_business::tx::read(&ctx.api.db, move |conn| {
            service::list_existing_ids(conn, ws, &[id])
        })
        .await
        .map_err(fail)?;
        if live.is_empty() {
            return Err(fail(BusinessError::not_found("Broadcast not found")));
        }
        return Ok(json!({ "data": [], "total": 0, "page": page, "pageCount": 0 }));
    };
    let mut out = service::list_contacts_page(
        &ctx.api.db,
        ctx.workspace_id,
        id,
        event_type,
        page,
        per_page.max(1),
    )
    .await
    .map_err(fail)?;
    out["page"] = json!(page);
    Ok(out)
}

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let input_ref = In(input);
    Some(match path {
        "broadcastAPIs.privatePreviewBroadcastRecurrenceAPI" => preview(input),
        "broadcastAPIs.privateListBroadcastRunGroupsAPI" => run_groups(ctx, input).await,
        "broadcastAPIs.privateListBroadcastOccurrencesAPI" => occurrences(ctx, input).await,
        "broadcastAPIs.privateListBroadcastContactsAPI" => contacts(ctx, input).await,
        // A soft-deleted broadcast's id is dropped (`listExistingIds`).
        "broadcastAPIs.privateGetBatchBroadcastStatsAPI" => {
            async {
                let ids = input_ref.ids("broadcastIds")?;
                let stats = chatbotx_business::tx::read(db, move |conn| {
                    let live = service::list_existing_ids(conn, ws, &ids)?;
                    history::batch_stats(conn, ws, &live)
                })
                .await
                .map_err(fail)?;
                Ok(Value::Object(stats))
            }
            .await
        }
        "broadcastAPIs.privateListBroadcastOptionsAPI" => {
            async {
                let channel = input_ref
                    .get("channel")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !CHANNELS.contains(&channel) {
                    return Err(invalid());
                }
                let data = service::list_options(db, ws, channel.to_owned())
                    .await
                    .map_err(fail)?;
                Ok(json!({ "data": data }))
            }
            .await
        }
        "broadcastAPIs.privateListBroadcastTemplateDetailsAPI" => {
            async {
                let id = input_ref.id("broadcastId")?;
                Ok(Value::Array(
                    write::list_template_details(db, ws, id)
                        .await
                        .map_err(fail)?,
                ))
            }
            .await
        }
        _ => return None,
    })
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    let api = ctx.api;
    let (db, audit, jobs) = (&api.db, &api.audit, api.jobs.as_ref());
    let ws = ctx.workspace_id;
    let can_view = ctx.has("emailAndPhone");
    if QUERIES.contains(&name) {
        return Some(query(ctx, name, args).await);
    }
    Some(
        async {
            match name {
                // [workspaceId], input
                "createBroadcastAction" => {
                    let data = parse_action_payload(arg(args, 1)).ok_or_else(invalid)?;
                    plan_limit_outcome(
                        write::create(db, audit, ws, data, can_view)
                            .await
                            .map(|b| b.to_json()),
                    )
                }
                // [workspaceId], { ids }
                "deleteBroadcastsAction" => {
                    let ids = In(arg(args, 1)).ids("ids")?;
                    let (deleted, requested) = service::soft_delete_broadcasts(db, audit, ws, ids)
                        .await
                        .map_err(action_fail)?;
                    Ok(json!({ "deletedCount": deleted, "requestedCount": requested }))
                }
                // [workspaceId, id], input?
                _ => {
                    let id = bound_id(arg(args, 1))?;
                    let input = arg(args, 2);
                    match name {
                        "cloneBroadcastAction" => {
                            write::clone_broadcast(db, audit, ws, id, can_view)
                                .await
                                .map(|b| b.to_json())
                                .map_err(action_fail)
                        }
                        "deleteBroadcastAction" => {
                            let (deleted, requested) =
                                service::soft_delete_broadcasts(db, audit, ws, vec![id])
                                    .await
                                    .map_err(action_fail)?;
                            Ok(json!({ "deletedCount": deleted, "requestedCount": requested }))
                        }
                        "moveBroadcastToDraftAction" => service::move_to_draft(db, audit, ws, id)
                            .await
                            .map_err(action_fail),
                        "resendBroadcastAction" => plan_limit_outcome(
                            write::resend_with_pruning(db, audit, ws, id, can_view)
                                .await
                                .map(|b| b.to_json()),
                        ),
                        "resumeBroadcastAction" => {
                            let data =
                                parsed(z::object(vec![("sendRatePerMinute", send_rate())]), input)?;
                            plan_limit_outcome(
                                service::resume_sending(db, audit, ws, id, submitted_rate(&data))
                                    .await,
                            )
                        }
                        "scheduleBroadcastAction" => {
                            let data = parsed(schedule_request(), input)?;
                            let recurrence =
                                data.get("recurrence").filter(|r| !r.is_null()).cloned();
                            plan_limit_outcome(
                                write::schedule_draft(
                                    db,
                                    audit,
                                    jobs,
                                    ws,
                                    id,
                                    data["schedulesType"]
                                        .as_str()
                                        .unwrap_or_default()
                                        .to_owned(),
                                    resolve_schedule_time(&data),
                                    submitted_rate(&data),
                                    recurrence,
                                )
                                .await,
                            )
                        }
                        "setBroadcastRunSkippedAction" => {
                            let data = parsed(
                                z::object(vec![("date", run_date()), ("skipped", z::boolean())]),
                                input,
                            )?;
                            history::set_occurrence_excluded(
                                db,
                                audit,
                                ws,
                                id.to_string(),
                                vec![data["date"].as_str().unwrap_or_default().to_owned()],
                                Vec::new(),
                                data["skipped"].as_bool().unwrap_or_default(),
                            )
                            .await
                            .map_err(action_fail)
                        }
                        "stopBroadcastAction" => service::stop_sending(db, audit, ws, id)
                            .await
                            .map_err(action_fail),
                        "updateBroadcastAction" => {
                            let name = In(input).str("name", true, 1, 255)?;
                            service::update(db, audit, ws, id, name)
                                .await
                                .map_err(action_fail)?;
                            Ok(Value::Null)
                        }
                        // updateDraftBroadcastAction
                        _ => {
                            let data = parse_action_payload(input).ok_or_else(invalid)?;
                            plan_limit_outcome(
                                write::update_draft(db, audit, jobs, ws, id, data, can_view).await,
                            )
                        }
                    }
                }
            }
        }
        .await,
    )
}
