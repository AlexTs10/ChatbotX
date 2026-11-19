//! The queries and actions of the tool screens (Q.D13a): inbound webhooks
//! (`features/inbound-webhooks/{queries,actions}`), outbound webhooks, magic
//! links and Google Sheets. The inbound lists and writes run the ported
//! public `/v1` routes ([`public`]), the same service calls with the same
//! input schemas; the other reads call the services the builder pages call.
//!
//! The inbound webhook and webhook pages are gated with
//! `requireWorkspacePermission(…, "superAdmin")` (every inbound action
//! re-checks it, `assertCanManageInboundWebhooks`). The SPA loads a page's
//! data through these queries, so the queries carry the page guard too. The
//! magic link and Google Sheets pages need a member only.

use chatbotx_business::{BusinessError, inbound_webhook, tx};
use chatbotx_db::repo::models::MagicLink;
use serde_json::{Value, json};

use super::tools::list_query;
use super::{Ctx, In, Outcome, action_fail, arg, fail, public, schema_errors};
use crate::orpc::OrpcError;

pub(super) const ACTIONS: &[&str] = &[
    "listInboundWebhooks",
    "findInboundWebhookForEditor",
    "listInboundWebhookRequests",
    "createInboundWebhookAction",
    "deleteInboundWebhooksAction",
    "getInboundWebhookRequestAction",
    "getInboundWebhookTokenAction",
    "regenerateInboundWebhookTokenAction",
    "updateInboundWebhookSettingsAction",
    "updateInboundWebhookAction",
    // `(has-folder)/webhooks/page.tsx`, `webhooks/[id]/edit/page.tsx`:
    // `getWebhooks`, `findWebhook` (`null` when not in the workspace) and the
    // edit page's `ecommerceSettingsService.storeSource`.
    "getWebhooks",
    "findWebhook",
    "webhookStoreSource",
    // `magic-links/page.tsx`, `magic-links/[magicLinkId]/analytics/page.tsx`,
    // `(integrations)/google-sheets/page.tsx` and
    // `settings/integrations/google-sheets/page.tsx` (any member).
    "listMagicLinks",
    "findMagicLink",
    "listSpreadsheets",
    "findIntegrationGoogleSheet",
];

/// The pages of these reads need a member only.
const MEMBER_READS: &[&str] = &[
    "listMagicLinks",
    "findMagicLink",
    "listSpreadsheets",
    "findIntegrationGoogleSheet",
];

/// `resolveGuardedWorkspaceId(…, "superAdmin")`: a 404 for other members.
const WEBHOOK_READS: &[&str] = &["getWebhooks", "findWebhook", "webhookStoreSource"];

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    if MEMBER_READS.contains(&name) {
        return Some(run(ctx, name, args).await);
    }
    if WEBHOOK_READS.contains(&name) && !ctx.has("superAdmin") {
        return Some(Err(fail(BusinessError::not_found("Not found"))));
    }
    if !ctx.has("superAdmin") {
        return Some(Err(action_fail(BusinessError::exception(
            "You need to be a super admin to manage inbound webhooks",
            "systemError",
            400,
        ))));
    }
    Some(run(ctx, name, args).await)
}

/// The id bound at `index`.
fn bound(args: &[Value], index: usize) -> Result<i64, OrpcError> {
    In(&json!({ "id": arg(args, index) })).id("id")
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let search = arg(args, 0);
    let id_param = |id: i64| [("id", id.to_string())];
    match name {
        "listInboundWebhooks" => {
            let query = list_query(search, &[]);
            public(ctx, "inboundWebhooks.list", &[], Some(&query), None).await
        }
        "findInboundWebhookForEditor" => {
            let id = In(search).id("id")?;
            let row = inbound_webhook::find_or_fail(&ctx.api.db, ctx.workspace_id, id)
                .await
                .map_err(fail)?;
            // The editor builds the endpoint URL from the token.
            let mut out = crate::handlers::inbound_webhooks::resource(&row, None);
            out["token"] = Value::String(row.token.clone());
            Ok(out)
        }
        "listInboundWebhookRequests" => {
            let id = In(search).id("inboundWebhookId")?;
            let query = list_query(search, &[]);
            public(
                ctx,
                "inboundWebhooks.listRequests",
                &id_param(id),
                Some(&query),
                None,
            )
            .await
        }
        "createInboundWebhookAction" => {
            let created = public(ctx, "inboundWebhooks.create", &[], None, Some(arg(args, 1)))
                .await
                .map_err(schema_errors)?;
            Ok(json!({ "id": created.get("id").cloned().unwrap_or(Value::Null) }))
        }
        "deleteInboundWebhooksAction" => public(
            ctx,
            "inboundWebhooks.deleteMany",
            &[],
            None,
            Some(arg(args, 1)),
        )
        .await
        .map(|_| Value::Null)
        .map_err(schema_errors),
        "getInboundWebhookRequestAction" => {
            let id = bound(args, 1)?;
            let request_id = bound(args, 2)?;
            let params = [
                ("id", id.to_string()),
                ("requestId", request_id.to_string()),
            ];
            public(
                ctx,
                "inboundWebhooks.getRequest",
                &params,
                Some("includePayload=true"),
                None,
            )
            .await
        }
        "getInboundWebhookTokenAction" => {
            let id = bound(args, 1)?;
            let row = inbound_webhook::find_or_fail(&ctx.api.db, ctx.workspace_id, id)
                .await
                .map_err(action_fail)?;
            Ok(json!({ "token": row.token }))
        }
        "regenerateInboundWebhookTokenAction" => {
            let id = bound(args, 1)?;
            public(
                ctx,
                "inboundWebhooks.regenerateToken",
                &id_param(id),
                None,
                Some(&json!({})),
            )
            .await
            .map(|_| Value::Null)
        }
        "updateInboundWebhookSettingsAction" | "updateInboundWebhookAction" => {
            let id = bound(args, 1)?;
            let route = match name {
                "updateInboundWebhookAction" => "inboundWebhooks.update",
                _ => "inboundWebhooks.updateSettings",
            };
            public(ctx, route, &id_param(id), None, Some(arg(args, 2)))
                .await
                .map(|_| Value::Null)
                .map_err(schema_errors)
        }
        "getWebhooks" => {
            let page = search["page"].as_i64().unwrap_or(1).max(1);
            let per_page = search["perPage"].as_i64().unwrap_or(10).max(1);
            // `folderId`: absent = every folder; null or "0" = the root.
            let folder_id = search.get("folderId").map(|v| {
                v.as_str()
                    .and_then(|s| s.parse::<i64>().ok())
                    .filter(|id| *id != 0)
            });
            let name = search["name"]
                .as_str()
                .filter(|n| !n.is_empty())
                .map(str::to_owned);
            let input = chatbotx_db::repo::webhook::ListWebhooksInput {
                workspace_id: ctx.workspace_id,
                folder_id,
                name,
                limit: per_page,
                offset: (page - 1) * per_page,
            };
            let (rows, page_count) = chatbotx_business::webhook::list(&ctx.api.db, input)
                .await
                .map_err(fail)?;
            let data: Vec<Value> = rows.iter().map(webhook_json).collect();
            Ok(json!({ "data": data, "pageCount": page_count }))
        }
        "findWebhook" => {
            let id = In(search).id("id")?;
            let ws = ctx.workspace_id;
            let row = tx::read(&ctx.api.db, move |c| {
                Ok(chatbotx_db::repo::webhook::find_webhook_with_conditions(
                    c,
                    Some(id),
                    Some(ws),
                )?)
            })
            .await
            .map_err(fail)?;
            Ok(row.as_ref().map_or(Value::Null, webhook_json))
        }
        "webhookStoreSource" => {
            chatbotx_business::external_store::store_source(&ctx.api.db, ctx.workspace_id)
                .await
                .map(Value::String)
                .map_err(fail)
        }
        "listMagicLinks" => {
            let sort = search["sort"].as_array().map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some(chatbotx_db::repo::SortItem {
                            id: item["id"].as_str()?.to_owned(),
                            desc: item["desc"].as_bool().unwrap_or(false),
                        })
                    })
                    .collect()
            });
            let (rows, page_count) = chatbotx_business::magic_link::list(
                &ctx.api.db,
                ctx.workspace_id,
                search["keyword"].as_str().map(str::to_owned),
                sort,
                search["page"].as_i64().unwrap_or(1),
                search["perPage"].as_i64().unwrap_or(10),
            )
            .await
            .map_err(fail)?;
            let data: Vec<Value> = rows.iter().map(|r| r.to_json()).collect();
            Ok(json!({ "data": data, "pageCount": page_count }))
        }
        "findMagicLink" => {
            let id = In(search).id("id")?;
            let ws = ctx.workspace_id;
            let row = tx::read(&ctx.api.db, move |c| {
                Ok(chatbotx_db::repo::find_one::<MagicLink>(
                    c,
                    r#""id" = ?1 AND "workspaceId" = ?2"#,
                    &[&id, &ws],
                )?)
            })
            .await
            .map_err(fail)?;
            Ok(row.map_or(
                Value::Null,
                |r| json!({ "id": r.id.to_string(), "name": r.name }),
            ))
        }
        "listSpreadsheets" => {
            let (rows, page_count) = chatbotx_business::spreadsheet::list(
                &ctx.api.db,
                ctx.workspace_id,
                search["name"].as_str().map(str::to_owned),
                search["page"].as_i64().unwrap_or(1),
                search["perPage"].as_i64(),
            )
            .await
            .map_err(fail)?;
            let data: Vec<Value> = rows.iter().map(|r| r.to_json()).collect();
            Ok(json!({ "data": data, "pageCount": page_count }))
        }
        "findIntegrationGoogleSheet" => {
            let row =
                chatbotx_business::google_sheets::find_by_workspace(&ctx.api.db, ctx.workspace_id)
                    .await
                    .map_err(fail)?;
            Ok(row.map_or(Value::Null, |r| {
                json!({
                    "id": r.id.to_string(),
                    "workspaceId": ctx.workspace_id.to_string(),
                    "integrationId": r.integration_id.to_string(),
                })
            }))
        }
        _ => Err(fail(BusinessError::not_found("Action not found"))),
    }
}

/// `{ ...webhook, conditions }` (the builder's `webhookService.list` rows).
fn webhook_json(row: &chatbotx_db::repo::webhook::WebhookWithConditions) -> Value {
    let mut out = row.webhook.to_json();
    out["conditions"] = row.conditions.iter().map(|c| c.to_json()).collect();
    out
}
