//! The Q.D13 rest domains: saved replies, response SLA, webhooks, working
//! hours, spreadsheets, magic links (persistent menus: `user_persistent_menus`) (`features/<domain>/api/{private,authorized}.ts` and
//! `features/<domain>/actions/*`). Request checks are the public handlers'
//! ones; a schema failure is the action's `validationErrors`.

use chatbotx_business::saved_reply::{self, Update};
use serde_json::{Map, Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, schema_errors};
use crate::handlers::input::Input;
use crate::handlers::input::{Str, key};
use crate::handlers::{response_sla, saved_replies, webhooks};
use crate::orpc::OrpcError;

mod flow_reads;

pub(super) const ACTIONS: &[&str] = &[
    "listSavedRepliesAction",
    "createSavedReplyAction",
    "editSavedReplyAction",
    "deleteSavedReplyAction",
    "updateResponseSlaAction",
    "createWebhookAction",
    "deleteWebhooksAction",
    "updateWebhookAction",
    "updateWebhookSettingsAction",
    "createSpreadsheetAction",
    "updateSpreadsheetAction",
    "deleteSpreadsheetAction",
    "createMagicLinkAction",
    "updateMagicLinkAction",
    "deleteMagicLinksAction",
];

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
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
        "spreadsheetsAPI.listSpreadsheetsAuthenticatedAPI" => {
            let page = input["page"].as_f64().map_or(1, |n| n as i64);
            let per_page = input["perPage"].as_f64().map(|n| n as i64);
            let name = input["name"].as_str().map(str::to_owned);
            chatbotx_business::spreadsheet::list(&api.db, ws, name, page, per_page)
                .await
                .map(|(rows, page_count)| {
                    let data: Vec<Value> = rows.iter().map(|r| r.to_json()).collect();
                    json!({ "data": data, "pageCount": page_count })
                })
                .map_err(fail)
        }
        "spreadsheetsAPI.listWorksheetsAuthenticatedAPI" => {
            let id = match In(input).id("spreadsheetId") {
                Ok(id) => id,
                Err(error) => return Some(Err(error)),
            };
            let http = api.provider_http.as_ref();
            chatbotx_business::spreadsheet::list_worksheets(&api.db, http, ws, id)
                .await
                .map(|data| json!({ "data": data }))
                .map_err(fail)
        }
        "spreadsheetsAPI.listWorksheetHeadersAuthenticatedAPI" => {
            let (id, sheet) = match (
                In(input).id("spreadsheetId"),
                In(input).opt_str("sheetName"),
            ) {
                (Ok(id), Ok(Some(sheet))) => (id, sheet),
                (Err(error), _) | (_, Err(error)) => return Some(Err(error)),
                (_, Ok(None)) => return Some(Err(super::invalid())),
            };
            let http = api.provider_http.as_ref();
            chatbotx_business::spreadsheet::list_worksheet_headers(&api.db, http, ws, id, &sheet)
                .await
                .map(|data| json!({ "data": data }))
                .map_err(fail)
        }
        "decisionProfilesAPI.listActiveForFlow" => {
            if !ctx.has("flows") {
                return Some(Err(OrpcError::new(
                    "FORBIDDEN",
                    Some(403),
                    Some("Forbidden"),
                )));
            }
            flow_reads::decision_profiles_for_flow(api, ws)
                .await
                .map_err(fail)
        }
        "dynamicImagesAPI.getDynamicImageAPI" => {
            let id = input["id"]
                .as_str()
                .and_then(|t| t.parse::<i64>().ok())
                .unwrap_or(-1);
            flow_reads::dynamic_image(api, ws, id).await.map_err(fail)
        }
        "leaderboardsAPI.listLeaderboardOptionsAPI" => {
            flow_reads::leaderboard_options(api, ws).await.map_err(fail)
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
        "createSpreadsheetAction" => {
            let mut input = input_at(args, 1);
            let (name, url) = crate::handlers::spreadsheets::request(&mut input);
            checked(input)?;
            chatbotx_business::spreadsheet::create(
                &api.db,
                api.provider_http.as_ref(),
                ws,
                name,
                url,
            )
            .await
            .map(|id| json!({ "id": id.to_string() }))
            .map_err(action_fail)
        }
        "updateSpreadsheetAction" => {
            let id = bound(args, 1)?;
            let mut input = input_at(args, 2);
            let (name, url) = crate::handlers::spreadsheets::request(&mut input);
            checked(input)?;
            chatbotx_business::spreadsheet::update(
                &api.db,
                api.provider_http.as_ref(),
                ws,
                id,
                name,
                url,
            )
            .await
            .map(|row| row.to_json())
            .map_err(action_fail)
        }
        "deleteSpreadsheetAction" => {
            let ids = In(arg(args, 1)).ids("ids")?;
            chatbotx_business::spreadsheet::delete_many(&api.db, ws, ids)
                .await
                .map(|()| Value::Null)
                .map_err(action_fail)
        }
        "createMagicLinkAction" | "updateMagicLinkAction" => {
            let update = name == "updateMagicLinkAction";
            let id = if update { Some(bound(args, 1)?) } else { None };
            let mut input = input_at(args, if update { 2 } else { 1 });
            let (link_name, url) = crate::handlers::magic_links::request(&mut input, update);
            checked(input)?;
            let db = &api.db;
            let outcome = match id {
                Some(id) => chatbotx_business::magic_link::update(db, ws, id, link_name, url).await,
                None => {
                    chatbotx_business::magic_link::create(
                        db,
                        ws,
                        link_name.unwrap_or_default(),
                        url.unwrap_or_default(),
                    )
                    .await
                }
            };
            outcome.map(|_| Value::Null).map_err(action_fail)
        }
        "deleteMagicLinksAction" => {
            let ids = In(arg(args, 1)).ids("ids")?;
            chatbotx_business::magic_link::delete_many(&api.db, ws, ids)
                .await
                .map(|()| Value::Null)
                .map_err(action_fail)
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
