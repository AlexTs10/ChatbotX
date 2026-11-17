//! The Q.D13 rest domains: saved replies, response SLA, persistent menus
//! (actions), webhooks, spreadsheets, dynamic images, decisions, magic links,
//! leaderboards (`features/<domain>/api/{private,authorized}.ts` and
//! `features/<domain>/actions/*`). Request checks are the public handlers'
//! ones; a schema failure is the action's `validationErrors`.

use chatbotx_business::saved_reply::{self, Update};
use chatbotx_business::tx;
use chatbotx_db::repo::small_repos;
use serde_json::{Map, Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, public, schema_errors};
use crate::handlers::input::Input;
use crate::handlers::input::{Str, key};
use crate::handlers::working_hours::{self as wh, audited};
use crate::handlers::{response_sla, saved_replies, webhooks};
use crate::orpc::OrpcError;
use chatbotx_business::working_hours as wh_service;

pub(super) const ACTIONS: &[&str] = &[
    "listSavedRepliesAction",
    "createSavedReplyAction",
    "editSavedReplyAction",
    "deleteSavedReplyAction",
    "updateResponseSlaAction",
    "createUserPersistentMenuAction",
    "updateUserPersistentMenuAction",
    "deleteUserPersistentMenuAction",
    "createWebhookAction",
    "deleteWebhooksAction",
    "updateWebhookAction",
    "updateWebhookSettingsAction",
    "createWorkingHoursAction",
    "updateWorkingHoursAction",
    "deleteWorkingHoursAction",
    "replaceWorkingHoursMembersAction",
    "createHolidayAction",
    "updateHolidayAction",
    "deleteHolidayAction",
];

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, _input: &Value) -> Outcome {
    let api = ctx.api;
    let ws = ctx.workspace_id;
    Some(match path {
        "savedRepliesAPI.listSavedRepliesAuthorizedAPI" => saved_replies::list(api, ws)
            .await
            .map(|data| json!({ "data": data }))
            .map_err(fail),
        "responseSlaAPI.getResponseSlaStatusWorkspaceAuthAPI" => response_sla::enabled(api, ws)
            .await
            .map(|enabled| json!({ "enabled": enabled }))
            .map_err(fail),
        "workingHoursAPI.listWorkingHoursOptionsWorkspaceAuthAPI" => {
            wh::options(api, ws).await.map_err(fail)
        }
        _ => return None,
    })
}

/// The action input object at `index` (a non-object is an empty input).
pub(crate) fn input_at(args: &[Value], index: usize) -> Input {
    Input::from_object(
        arg(args, index)
            .as_object()
            .cloned()
            .unwrap_or_else(Map::new),
    )
}

/// `input.finish()` as the action's `validationErrors`.
pub(crate) fn checked(input: Input) -> Result<(), OrpcError> {
    input.finish().map_err(schema_errors)
}

/// The bound id at `index` (`zodBigintAsString`).
pub(crate) fn bound(args: &[Value], index: usize) -> Result<i64, OrpcError> {
    In(&json!({ "id": arg(args, index) })).id("id")
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let (api, ws) = (ctx.api, ctx.workspace_id);
    let storage = crate::handlers::contacts::tenant_urls(api).storage_url;
    match name {
        "listSavedRepliesAction" => saved_replies::list(api, ws)
            .await
            .map(|data| json!({ "data": data }))
            .map_err(action_fail),
        "createSavedReplyAction" => {
            let mut input = input_at(args, 1);
            let (shortcut, text, images) = saved_replies::fields(&mut input, true);
            checked(input)?;
            saved_reply::create(
                &api.db,
                &storage,
                ws,
                shortcut,
                text,
                images.unwrap_or_default(),
            )
            .await
            .map(|row| row.to_json())
            .map_err(action_fail)
        }
        "editSavedReplyAction" => {
            let id = bound(args, 1)?;
            let mut input = input_at(args, 2);
            let (shortcut, text, image_file_ids) = saved_replies::fields(&mut input, false);
            checked(input)?;
            let data = Update {
                shortcut,
                text,
                image_file_ids,
            };
            saved_reply::update(&api.db, &storage, ws, id, data)
                .await
                .map(|row| row.to_json())
                .map_err(action_fail)
        }
        "deleteSavedReplyAction" => {
            let id = bound(args, 1)?;
            saved_reply::delete(&api.db, ws, id)
                .await
                .map(|()| Value::Null)
                .map_err(action_fail)
        }
        "updateResponseSlaAction" => {
            let mut input = input_at(args, 1);
            let data = response_sla::request(&mut input);
            checked(input)?;
            response_sla::upsert(api, ws, data)
                .await
                .map_err(action_fail)
        }
        "createUserPersistentMenuAction" => public(
            ctx,
            "userPersistentMenus.create",
            &[],
            None,
            Some(arg(args, 1)),
        )
        .await
        .map(|_| Value::Null)
        .map_err(schema_errors),
        "updateUserPersistentMenuAction" => {
            let id = bound(args, 1)?.to_string();
            public(
                ctx,
                "userPersistentMenus.update",
                &[("id", id)],
                None,
                Some(arg(args, 2)),
            )
            .await
            .map(|_| Value::Null)
            .map_err(schema_errors)
        }
        "deleteUserPersistentMenuAction" => {
            let ids = In(arg(args, 1)).ids("ids")?;
            tx::write(&api.db, move |c| {
                let deleted = small_repos::delete_user_persistent_menus(c, ws, &ids)?;
                if ids.len() == 1 && deleted.is_empty() {
                    return Err(chatbotx_business::BusinessError::not_found(
                        "User persistent menu not found",
                    ));
                }
                Ok(Value::Null)
            })
            .await
            .map_err(action_fail)
        }
        "createWebhookAction" => {
            let mut input = input_at(args, 1);
            let name = input.string(
                "name",
                Str {
                    min: Some(1),
                    ..Str::default()
                },
            );
            // `zodBigintAsString().nullable()`: absent is an issue.
            let folder = match input.get("folderId").cloned() {
                Some(Value::Null) => None,
                other => crate::handlers::tags::digits_at(
                    &mut input,
                    &key("folderId"),
                    other.as_ref(),
                    true,
                ),
            };
            checked(input)?;
            match chatbotx_business::webhook::create(&api.db, ws, name.unwrap_or_default(), folder)
                .await
            {
                Ok((row, detail)) => {
                    api.record_audit("create", &detail).await;
                    Ok(webhooks::row_json(&row))
                }
                // The builder translates `validation.maxItemsReached` (en).
                Err(chatbotx_business::BusinessError::Exception { code, data, .. })
                    if code == "validation" =>
                {
                    let max = data.as_ref().map_or(Value::Null, |d| d["max"].clone());
                    Err(fail(chatbotx_business::BusinessError::exception(
                        format!("Maximum {max} webhooks allowed per workspace"),
                        "businessError",
                        400,
                    )))
                }
                Err(error) => Err(action_fail(error)),
            }
        }
        "deleteWebhooksAction" => {
            let ids = In(arg(args, 1)).ids("ids")?;
            webhooks::delete_many(api, ws, ids)
                .await
                .map(|()| Value::Null)
                .map_err(action_fail)
        }
        "updateWebhookAction" => {
            let id = bound(args, 1)?;
            let mut input = input_at(args, 2);
            let conditions = webhooks::conditions(&mut input, 0);
            let url = webhooks::url(&mut input, false);
            checked(input)?;
            webhooks::update(
                api,
                ws,
                id,
                url.unwrap_or_default(),
                conditions.unwrap_or_default(),
            )
            .await
            .map(|row| row.map_or(Value::Null, |w| webhooks::row_json(&w)))
            .map_err(action_fail)
        }
        "updateWebhookSettingsAction" => {
            let id = bound(args, 1)?;
            let mut input = input_at(args, 2);
            let (name, active) = webhooks::settings(&mut input);
            checked(input)?;
            webhooks::update_settings(api, ws, id, name, active)
                .await
                .map(|()| Value::Null)
                .map_err(action_fail)
        }
        "createWorkingHoursAction" => {
            let mut input = input_at(args, 1);
            let (name, intervals, users) = wh::create_request(&mut input);
            checked(input)?;
            let outcome = wh_service::create(&api.db, ws, name, intervals, users).await;
            audited(api, "create", outcome).await.map_err(action_fail)
        }
        "updateWorkingHoursAction" => {
            let id = bound(args, 1)?;
            let mut input = input_at(args, 2);
            let data = wh::update_request(&mut input);
            checked(input)?;
            let outcome = wh_service::update(&api.db, ws, id, data).await;
            audited(api, "update", outcome).await.map_err(action_fail)
        }
        "deleteWorkingHoursAction" => {
            let id = bound(args, 1)?;
            let outcome = wh_service::delete(&api.db, ws, id).await.map(|d| ((), d));
            audited(api, "delete", outcome)
                .await
                .map(|()| Value::Null)
                .map_err(action_fail)
        }
        "replaceWorkingHoursMembersAction" => {
            let id = bound(args, 1)?;
            let mut input = input_at(args, 2);
            let users = wh::members_request(&mut input);
            checked(input)?;
            let outcome = wh_service::replace_members(&api.db, ws, id, users).await;
            audited(api, "update", outcome).await.map_err(action_fail)
        }
        "createHolidayAction" => {
            let mut input = input_at(args, 1);
            let data = wh::create_holiday_request(&mut input);
            checked(input)?;
            let outcome = wh_service::add_holiday(&api.db, ws, data).await;
            audited(api, "create", outcome)
                .await
                .map(|row| wh_service::holiday_json(&row))
                .map_err(action_fail)
        }
        "updateHolidayAction" => {
            let id = bound(args, 1)?;
            let mut input = input_at(args, 2);
            let (name, ranges) = wh::update_holiday_request(&mut input);
            checked(input)?;
            let outcome = wh_service::update_holiday(&api.db, ws, id, name, ranges).await;
            audited(api, "update", outcome)
                .await
                .map(|row| wh_service::holiday_json(&row))
                .map_err(action_fail)
        }
        "deleteHolidayAction" => {
            let id = bound(args, 1)?;
            let outcome = wh_service::delete_holiday(&api.db, ws, id)
                .await
                .map(|d| ((), d));
            audited(api, "delete", outcome)
                .await
                .map(|()| Value::Null)
                .map_err(action_fail)
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
