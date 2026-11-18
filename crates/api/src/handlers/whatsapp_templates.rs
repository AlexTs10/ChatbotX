//! `features/integration-whatsapp/message-templates/api/public.ts`
//! (`whatsappTemplatesPublicRouter` + the deprecated `templateMessagesPublicRouter`,
//! scope `broadcasts`), with `whatsappMessageTemplateService` and
//! `lib/sync-whatsapp-templates.ts` (`listMessageTemplates` of `api/waba.ts`).
//! `whatsappTemplates.searchCatalogProducts` is served by `meta_catalog` (same service).

use std::time::Duration;

use chatbotx_business::{BusinessError, tx};
use chatbotx_core::flow_config::steps::describe_wa_template_parameters;
use chatbotx_core::utils::graph_error::format_graph_error;
use chatbotx_core::utils::graph_pagination::{DEFAULT_GRAPH_PAGE_LIMIT, GraphPage};
use chatbotx_db::repo::models::{IntegrationWhatsapp, WhatsappMessageTemplate};
use chatbotx_db::repo::{find_many, iso, new_id, now_ms};
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};

use super::input::{Input, key};
use super::messenger_templates::{Fail, reply_error};
use super::{ApiState, Call, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "whatsappTemplates.list",
    "templateMessages.list",
    "whatsappTemplates.get",
    "whatsappTemplates.sync",
];

const DEFAULT_API_VERSION: &str = "v23.0";
/// `MESSAGE_TEMPLATE_MAX_PAGES`.
const MAX_PAGES: usize = 70;
/// `whatsappTemplateStatusSchema`.
const STATUSES: &[&str] = &["APPROVED", "PENDING", "REJECTED"];

fn template_json(t: &WhatsappMessageTemplate) -> Value {
    json!({
        "id": t.id.to_string(),
        "name": t.name,
        "language": t.language,
        "category": t.category,
        "status": t.status,
        "components": t.components.0,
        "integrationWhatsappId": t.integration_whatsapp_id.to_string(),
    })
}

/// `integrationWhatsappResource`.
pub(super) fn integration_json(i: &IntegrationWhatsapp) -> Value {
    json!({
        "id": i.id.to_string(),
        "name": i.name,
        "inboxId": i.inbox_id.to_string(),
        "displayPhoneNumber": i.display_phone_number,
        "tokenRefreshError": i.token_refresh_error,
        "phoneNumberId": i.phone_number_id,
        "wabaId": i.waba_id,
        "hasCapiScope": i.has_capi_scope,
        "capiScopeCheckedAt": i.capi_scope_checked_at.map(iso),
        "datasetId": i.dataset_id,
    })
}

/// `whatsappMessageTemplateService.list`: oldest first, with the integration.
pub async fn list(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    inbox_id: Option<i64>,
    integration_id: Option<i64>,
    status: Option<String>,
) -> Result<Vec<Value>, BusinessError> {
    tx::read(db, move |conn| {
        let integrations: Vec<IntegrationWhatsapp> = find_many(
            conn,
            r#"WHERE "workspaceId" = ?1 AND (?2 IS NULL OR "inboxId" = ?2)"#,
            &[&workspace_id, &inbox_id],
        )?;
        let ids: Vec<i64> = integrations.iter().map(|i| i.id).collect();
        let templates: Vec<WhatsappMessageTemplate> = find_many(
            conn,
            r#"WHERE "integrationWhatsappId" IN (SELECT value FROM json_each(?1))
               AND (?2 IS NULL OR "integrationWhatsappId" = ?2) AND (?3 IS NULL OR "status" = ?3)
               ORDER BY "createdAt", "id""#,
            &[
                &serde_json::to_string(&ids).unwrap_or_default(),
                &integration_id,
                &status,
            ],
        )?;
        Ok(templates
            .iter()
            .filter_map(|t| {
                let integration = integrations
                    .iter()
                    .find(|i| i.id == t.integration_whatsapp_id)?;
                let mut out = template_json(t);
                out["integrationWhatsapp"] = integration_json(integration);
                Some(out)
            })
            .collect())
    })
    .await
}

/// `whatsappMessageTemplateService.findByIdForWorkspace`.
async fn find_for_workspace(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    id: i64,
) -> Result<Option<WhatsappMessageTemplate>, BusinessError> {
    tx::read(db, move |conn| {
        let rows: Vec<WhatsappMessageTemplate> = find_many(
            conn,
            r#"WHERE "id" = ?1 AND "integrationWhatsappId" IN
               (SELECT "id" FROM "IntegrationWhatsapp" WHERE "workspaceId" = ?2) LIMIT 1"#,
            &[&id, &workspace_id],
        )?;
        Ok(rows.into_iter().next())
    })
    .await
}

fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default()
    })
}

/// One Graph page; a failure is `rescue`'s `WhatsappException` (Meta's status).
async fn get_page(url: String, token: String) -> Result<GraphPage<Value>, Fail> {
    let response = http()
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| Fail::Sdk(e.to_string(), 400))?;
    let status = response.status().as_u16();
    let bytes = response.bytes().await.unwrap_or_default();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let message = format_graph_error(body.get("error"))
            .unwrap_or_else(|| "WhatsApp API call failed".to_owned());
        return Err(Fail::Sdk(message, status));
    }
    Ok(GraphPage {
        data: body["data"].as_array().cloned().unwrap_or_default(),
        after: body
            .pointer("/paging/cursors/after")
            .and_then(Value::as_str)
            .map(str::to_owned),
        next: body
            .pointer("/paging/next")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// `listMessageTemplates(auth)`: every page of `/{wabaId}/message_templates`.
async fn list_meta_templates(graph_url: &str, auth: &Value) -> Result<Vec<Value>, Fail> {
    let text = |p: &str| auth.pointer(p).and_then(Value::as_str).unwrap_or("");
    let version = Some(text("/version"))
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_API_VERSION);
    let first = format!(
        "{graph_url}/{version}/{}/message_templates?limit={DEFAULT_GRAPH_PAGE_LIMIT}",
        text("/metadata/wabaId")
    );
    let token = text("/tokens/accessToken").to_owned();
    // `fetchAllNextPages` (same-origin, unvisited `paging.next`, at most 70 pages).
    // ponytail: inline loop: the core helper's `&mut dyn FnMut` callback is not
    // `Send`, so its future cannot run in an axum handler.
    let origin = |u: &str| {
        url::Url::parse(u)
            .ok()
            .map(|u| u.origin().ascii_serialization())
    };
    let first_origin = origin(&first);
    let mut results = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut next = Some(first);
    for _ in 0..MAX_PAGES {
        let Some(url) = next.take() else {
            return Ok(results);
        };
        visited.insert(url.clone());
        let page = get_page(url, token.clone()).await?;
        results.extend(page.data);
        next = page
            .next
            .filter(|n| !n.is_empty() && !visited.contains(n))
            .filter(|n| first_origin.is_some() && origin(n) == first_origin);
    }
    match next {
        Some(_) => Err(Fail::Sdk(
            format!("Graph pagination exceeded {MAX_PAGES} pages"),
            400,
        )),
        None => Ok(results),
    }
}

/// `syncFromMeta`: full sync (rows Meta no longer has are deleted).
async fn sync_from_meta(
    db: &chatbotx_db::Db,
    integration_id: i64,
    templates: Vec<Value>,
) -> Result<(), BusinessError> {
    tx::write(db, move |conn| {
        let incoming: Vec<String> = templates
            .iter()
            .filter_map(|t| t["id"].as_str().map(str::to_owned))
            .collect();
        conn.execute(
            r#"DELETE FROM "WhatsappMessageTemplate" WHERE "integrationWhatsappId" = ?1
               AND "sourceId" NOT IN (SELECT value FROM json_each(?2))"#,
            params![integration_id, serde_json::to_string(&incoming).unwrap_or_default()],
        )?;
        let text = |t: &Value, k: &str| t[k].as_str().unwrap_or_default().to_owned();
        for t in &templates {
            conn.execute(
                r#"INSERT INTO "WhatsappMessageTemplate" ("id", "name", "integrationWhatsappId", "language",
                     "category", "status", "sourceId", "components")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                   ON CONFLICT ("integrationWhatsappId", "sourceId") DO UPDATE SET
                     "name" = excluded."name", "language" = excluded."language",
                     "category" = excluded."category", "status" = excluded."status",
                     "components" = excluded."components", "updatedAt" = ?9"#,
                params![
                    new_id(),
                    text(t, "name"),
                    integration_id,
                    text(t, "language"),
                    text(t, "category"),
                    text(t, "status"),
                    text(t, "id"),
                    t.get("components").cloned().unwrap_or(Value::Null).to_string(),
                    now_ms(),
                ],
            )?;
        }
        Ok(())
    })
    .await
}

/// `syncWhatsappMessageTemplates` (shared with the `/rpc` sync action).
pub(crate) async fn sync_integration(
    db: &chatbotx_db::Db,
    graph_url: &str,
    integration: &IntegrationWhatsapp,
) -> Result<(), Fail> {
    let templates = list_meta_templates(graph_url, &integration.auth.0).await?;
    Ok(sync_from_meta(db, integration.id, templates).await?)
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "whatsappTemplates.list" | "templateMessages.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let inbox = input
                .obj
                .get("inboxId")
                .cloned()
                .and_then(|v| input.id_at(&key("inboxId"), Some(&v), false));
            let integration = input
                .obj
                .get("integrationWhatsappId")
                .cloned()
                .and_then(|v| input.id_at(&key("integrationWhatsappId"), Some(&v), false));
            let value = input.get("status").cloned();
            let status = input.enum_at(&key("status"), value.as_ref(), STATUSES, false);
            input.finish()?;
            let rows = list(db, ws, inbox, integration, status)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            Ok(json(200, Value::Array(rows)))
        }
        "whatsappTemplates.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("id");
            input.finish()?;
            let template = find_for_workspace(db, ws, id.unwrap_or(-1))
                .await
                .map_err(|e| business_error(e, FINDING))?
                .ok_or_else(|| {
                    business_error(BusinessError::not_found("Template not found"), FINDING)
                })?;
            let components = template
                .components
                .0
                .as_array()
                .cloned()
                .unwrap_or_default();
            let parameters: Vec<Value> = describe_wa_template_parameters(&components)
                .into_iter()
                .map(|spec| {
                    let mut out = json!({
                        "key": spec.key,
                        "component": spec.component,
                        "kind": spec.kind,
                        "required": spec.required,
                    });
                    if let Some(placeholder) = spec.placeholder {
                        out["placeholder"] = json!(placeholder);
                    }
                    out
                })
                .collect();
            let mut out = template_json(&template);
            out["parameters"] = Value::Array(parameters);
            Ok(json(200, out))
        }
        _ => {
            // whatsappTemplates.sync
            let mut input = Input::from_body(call.params, call.body);
            let id = input.digits_id("id");
            input.finish()?;
            let id = id.unwrap_or(-1);
            let integration = tx::read(db, move |conn| {
                Ok(
                    chatbotx_db::repo::integration_whatsapp::find_by_id_for_workspace(
                        conn, id, ws,
                    )?,
                )
            })
            .await
            .map_err(|e| business_error(e, MUTATING))?
            .ok_or_else(|| {
                business_error(
                    BusinessError::not_found("WhatsApp channel not found"),
                    MUTATING,
                )
            })?;
            sync_integration(db, &state.graph_url, &integration)
                .await
                .map_err(|f| reply_error(f, MUTATING))?;
            Ok(no_content())
        }
    }
}
