//! `features/leaderboards/{queries,actions}`: the leaderboard list, wizard
//! and participants screens of the SPA (`leaderboardService`,
//! `leaderboardParticipantService`). Membership is the `/rpc` check; the
//! inputs are the builder zod schemas (the public API parsers).

use chatbotx_business::BusinessError;
use chatbotx_business::leaderboard::service::{self, Deps, ListInput, ParticipantsInput};
use chatbotx_business::leaderboard::{PLATFORMS, default_rules, platform_channel};
use chatbotx_business::minigame::{self, ListMinigames};
use chatbotx_db::repo::SortItem;
use serde_json::{Map, Value, json};

use super::{Ctx, In, Outcome, action_fail, arg};
use crate::handlers::input::Input;
use crate::handlers::leaderboards::{adjust_fields, config, reason, resource, view_url_template};
use crate::orpc::OrpcError;

const ACTIONS: &[&str] = &[
    "listLeaderboards",
    "findLeaderboard",
    "listLeaderboardParticipants",
    "getLeaderboardWizardData",
    "createLeaderboardAction",
    "updateLeaderboardAction",
    "changeLeaderboardStatusAction",
    "deleteLeaderboardAction",
    "resetLeaderboardPointsAction",
    "triggerLeaderboardRewardsAction",
    "exportLeaderboardAction",
    "getLeaderboardExportAction",
    "listTelegramChatsAction",
    "findParticipantConversationAction",
    "adjustParticipantScoreAction",
    "setParticipantBannedAction",
    "resetParticipantScoreAction",
    "getParticipantHistoryAction",
    "listLeaderboardAuditLogAction",
];

const MINIGAME_OPTIONS_LIMIT: i64 = 200;
const AUDIT_LOG_PAGE_SIZE: i64 = 20;
const DOWNLOAD_URL_TTL_SECONDS: u64 = 5 * 60;

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

/// The action input object as a parser [`Input`].
fn input_of(value: &Value) -> Input {
    Input::from_body(&[], value.to_string().as_bytes())
}

fn bound_id(args: &[Value], index: usize) -> Result<i64, OrpcError> {
    In(&json!({ "id": arg(args, index) })).id("id")
}

fn reason_fields(value: &Value) -> Result<(i64, i64, String), OrpcError> {
    let ids = In(value);
    let (id, contact) = (ids.id("id")?, ids.id("contactId")?);
    let mut input = input_of(value);
    let reason = reason(&mut input);
    input.finish().map_err(super::schema_errors)?;
    Ok((id, contact, reason))
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let (db, ws, app) = (&ctx.api.db, ctx.workspace_id, ctx.api.app_url.as_str());
    let deps = Deps {
        db,
        jobs: ctx.api.jobs.clone(),
    };
    let user = Some(ctx.user_id);
    match name {
        "listLeaderboards" => {
            let search = In(arg(args, 0));
            let out = service::list(
                db,
                ws,
                ListInput {
                    status: search.opt_str("status")?,
                    platform: search.opt_str("platform")?,
                    keyword: search.opt_str("keyword")?.filter(|k| !k.trim().is_empty()),
                    page: search.opt_positive("page")?,
                    per_page: search.opt_positive("perPage")?,
                },
            )
            .await
            .map_err(action_fail)?;
            let data: Vec<Value> = out
                .rows
                .iter()
                .map(|(row, stats)| resource(app, &row.leaderboard, *stats))
                .collect();
            Ok(
                json!({ "data": data, "pageCount": out.page_count, "total": out.total, "summary": out.summary }),
            )
        }
        "findLeaderboard" => match service::find(db, ws, In(arg(args, 0)).id("id")?).await {
            Ok((row, stats)) => Ok(resource(app, &row, stats)),
            Err(error) if error.code() == Some("notFound") => Ok(Value::Null),
            Err(error) => Err(action_fail(error)),
        },
        "listLeaderboardParticipants" => {
            let search = In(arg(args, 0));
            service::list_participants(
                db,
                ws,
                search.id("leaderboardId")?,
                ParticipantsInput {
                    keyword: search.opt_str("keyword")?.filter(|k| !k.trim().is_empty()),
                    filter: search.opt_str("filter")?.unwrap_or_else(|| "all".into()),
                    page: search.opt_positive("page")?,
                    per_page: search.opt_positive("perPage")?,
                },
            )
            .await
            .map_err(action_fail)
        }
        "getLeaderboardWizardData" => {
            let games = minigame::list(
                db,
                ListMinigames {
                    workspace_id: ws,
                    page: Some(1),
                    per_page: Some(MINIGAME_OPTIONS_LIMIT),
                    name: None,
                    sort: Some(vec![SortItem {
                        id: "createdAt".into(),
                        desc: true,
                    }]),
                },
            )
            .await
            .map_err(action_fail)?;
            let options: Vec<Value> = games
                .data
                .iter()
                .map(|m| json!({ "label": m.name, "value": m.id.to_string() }))
                .collect();
            let mut rules = Map::new();
            let mut channels = Map::new();
            for platform in PLATFORMS {
                rules.insert((*platform).into(), default_rules(platform));
                channels.insert((*platform).into(), json!(platform_channel(platform)));
            }
            Ok(
                json!({ "minigameOptions": options, "defaultRules": rules, "platformChannels": channels }),
            )
        }
        "createLeaderboardAction" | "updateLeaderboardAction" => {
            let create = name == "createLeaderboardAction";
            let body = arg(args, if create { 1 } else { 2 });
            let mut input = input_of(body);
            let cfg = config(&mut input);
            input.finish().map_err(super::schema_errors)?;
            let publish = body["publish"] == true;
            if create {
                let row = service::create(db, ws, cfg, Some(view_url_template(app)))
                    .await
                    .map_err(action_fail)?;
                let id = row.id.to_string();
                if !publish {
                    return Ok(json!({ "id": id, "published": false, "publishError": null }));
                }
                return Ok(match service::publish(db, ws, row.id).await {
                    Ok(_) => json!({ "id": id, "published": true, "publishError": null }),
                    Err(error @ BusinessError::Exception { .. }) => {
                        json!({ "id": id, "published": false, "publishError": error.message() })
                    }
                    Err(error) => return Err(action_fail(error)),
                });
            }
            let id = bound_id(args, 1)?;
            let row = service::update(db, ws, id, cfg)
                .await
                .map_err(action_fail)?;
            if publish && row.status == "draft" {
                service::publish(db, ws, id).await.map_err(action_fail)?;
                return Ok(json!({ "id": id.to_string(), "published": true }));
            }
            Ok(json!({ "id": id.to_string(), "published": false }))
        }
        "changeLeaderboardStatusAction" => {
            let id = bound_id(args, 1)?;
            let row = match In(arg(args, 2)).opt_str("action")?.as_deref() {
                Some("publish") => service::publish(db, ws, id).await,
                Some("pause") => service::pause(db, ws, id).await,
                Some("start") => service::start(db, ws, id).await,
                Some("finish") => service::finish(&deps, ws, id).await,
                _ => return Err(super::invalid()),
            }
            .map_err(action_fail)?;
            Ok(json!({ "id": row.id.to_string(), "status": row.status }))
        }
        "deleteLeaderboardAction" => {
            let deleted = service::delete(db, ws, bound_id(args, 1)?)
                .await
                .map_err(action_fail)?;
            Ok(json!(deleted.to_string()))
        }
        "resetLeaderboardPointsAction" => {
            let id = bound_id(args, 1)?;
            let mut input = input_of(arg(args, 2));
            let reason = reason(&mut input);
            input.finish().map_err(super::schema_errors)?;
            let reset = service::reset_points(db, ws, id, user, reason)
                .await
                .map_err(action_fail)?;
            Ok(json!({ "reset": reset }))
        }
        "triggerLeaderboardRewardsAction" => {
            let out = service::trigger_rewards(&deps, ws, bound_id(args, 1)?, true)
                .await
                .map_err(action_fail)?;
            let count = |key: &str| out[key].as_array().map_or(0, Vec::len);
            Ok(
                json!({ "winners": count("winners"), "sent": out["sent"], "skipped": count("skipped") }),
            )
        }
        "exportLeaderboardAction" => {
            // Phone/email columns follow the member's contact permission.
            let include = ctx
                .contact_scope()
                .is_some_and(|s| s.can_view_email_and_phone);
            service::create_export(&deps, ws, bound_id(args, 1)?, user, include)
                .await
                .map_err(action_fail)
        }
        "getLeaderboardExportAction" => {
            let file_id = In(arg(args, 1)).id("fileId")?;
            let file = service::get_export(db, ws, file_id)
                .await
                .map_err(action_fail)?;
            let download = match (&ctx.api.files, file.status == "uploaded") {
                (Some(files), true) => Some(
                    files
                        .get_presigned_download(&file.path, Some(DOWNLOAD_URL_TTL_SECONDS))
                        .await
                        .map_err(|e| {
                            action_fail(BusinessError::exception(e.to_string(), "systemError", 500))
                        })?,
                ),
                _ => None,
            };
            let total = file
                .meta
                .as_ref()
                .and_then(|m| m.0["totalRecords"].as_i64())
                .unwrap_or(0);
            Ok(
                json!({ "status": file.status, "fileName": file.file_name, "totalRecords": total, "downloadUrl": download }),
            )
        }
        "listTelegramChatsAction" => {
            let inbox = In(arg(args, 1)).id("inboxId")?;
            service::telegram_chats(db, ws, inbox)
                .await
                .map_err(action_fail)
        }
        "findParticipantConversationAction" => {
            let contact = In(arg(args, 1)).id("contactId")?;
            let id = super::dm_conversation_id(ctx, contact).await?;
            Ok(json!({ "conversationId": id.map(|id| id.to_string()) }))
        }
        "adjustParticipantScoreAction" => {
            let value = arg(args, 1);
            let ids = In(value);
            let (id, contact) = (ids.id("id")?, ids.id("contactId")?);
            let mut input = input_of(value);
            let (delta, reason) = adjust_fields(&mut input);
            input.finish().map_err(super::schema_errors)?;
            let score = service::adjust(db, ws, id, contact, delta, &reason, user)
                .await
                .map_err(action_fail)?;
            Ok(json!({ "score": score }))
        }
        "setParticipantBannedAction" => {
            let value = arg(args, 1);
            let (id, contact, reason) = reason_fields(value)?;
            let banned = In(value).opt_bool("isBanned")?.ok_or_else(super::invalid)?;
            service::set_banned(db, ws, id, contact, banned, &reason, user)
                .await
                .map_err(action_fail)?;
            Ok(json!({ "ok": true }))
        }
        "resetParticipantScoreAction" => {
            let (id, contact, reason) = reason_fields(arg(args, 1))?;
            let score = service::reset_one(db, ws, id, contact, &reason, user)
                .await
                .map_err(action_fail)?;
            Ok(json!({ "score": score }))
        }
        "getParticipantHistoryAction" => {
            let ids = In(arg(args, 1));
            service::participant_history(db, ws, ids.id("id")?, ids.id("contactId")?)
                .await
                .map_err(action_fail)
        }
        "listLeaderboardAuditLogAction" => {
            let search = In(arg(args, 1));
            let page = search.opt_positive("page")?.unwrap_or(1);
            service::audit_log(
                db,
                ws,
                search.id("id")?,
                Some(page),
                Some(AUDIT_LOG_PAGE_SIZE),
            )
            .await
            .map_err(action_fail)
        }
        _ => Err(super::invalid()),
    }
}
