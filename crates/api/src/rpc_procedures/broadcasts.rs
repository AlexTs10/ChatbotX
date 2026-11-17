//! Broadcasts: `features/broadcasts/api/private.ts` and
//! `features/broadcasts/actions/*` (Q.D5). The stats and contacts reads need
//! the delivery analytics (Q.D11); run groups, occurrences and
//! `setBroadcastRunSkippedAction` the recurrence history (Q.D5 slice 4b).

use chatbotx_business::BusinessError;
use chatbotx_business::broadcast::{self as service, recurrence, write};
use chatbotx_db::repo::now_ms;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid};
use crate::handlers::broadcasts::{
    parse_action_payload, recurrence_rule, resolve_schedule_time, schedule_request, send_rate,
    submitted_rate,
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
    "stopBroadcastAction",
    "updateBroadcastAction",
    "updateDraftBroadcastAction",
];

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

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let input_ref = In(input);
    Some(match path {
        "broadcastAPIs.privatePreviewBroadcastRecurrenceAPI" => preview(input),
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
