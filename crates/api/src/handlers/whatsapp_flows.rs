//! `features/integration-whatsapp/flows/api/public.ts` (`whatsappFlowsPublicRouter`,
//! scope `broadcasts`), with `whatsappFlowService` and `lib/whatsapp-flow-operations.ts`
//! (`listFlows` / `getFlowAssets` of `integrations/whatsapp/src/api/flow.ts`).

use std::time::Duration;

use chatbotx_business::net::ssrf_guard::assert_public_url;
use chatbotx_business::whatsapp_flow::parse_flow_screens;
use chatbotx_business::{BusinessError, tx};
use chatbotx_db::repo::models::{IntegrationWhatsapp, WhatsappFlow};
use chatbotx_db::repo::{find_many, new_id, now_ms};
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};

use super::input::{Input, key};
use super::messenger_templates::{Fail, reply_error};
use super::whatsapp_templates::integration_json;
use super::{ApiState, Call, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "whatsappFlows.list",
    "whatsappFlows.getScreens",
    "whatsappFlows.sync",
];

const DEFAULT_API_VERSION: &str = "v23.0";
const FLOW_NOT_FOUND: &str = "WhatsApp Flow not found";

/// `whatsappFlowResource` (without `integrationWhatsapp`).
fn flow_json(f: &WhatsappFlow) -> Value {
    json!({
        "id": f.id.to_string(),
        "name": f.name,
        "sourceId": f.source_id,
        "status": f.status,
        "categories": f.categories.0,
        "validationErrors": f.validation_errors.0,
        "completedCount": f.completed_count.to_string(),
        "integrationWhatsappId": f.integration_whatsapp_id.to_string(),
        "screens": f.screens.0,
    })
}

/// `whatsappFlowService.list`: oldest first; `with_integration` for the inbox list.
pub async fn list(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    inbox_id: Option<i64>,
    integration_id: Option<i64>,
    with_integration: bool,
) -> Result<Vec<Value>, BusinessError> {
    tx::read(db, move |conn| {
        let integrations: Vec<IntegrationWhatsapp> = find_many(
            conn,
            r#"WHERE "workspaceId" = ?1 AND (?2 IS NULL OR "inboxId" = ?2)
               AND (?3 IS NULL OR "id" = ?3)"#,
            &[&workspace_id, &inbox_id, &integration_id],
        )?;
        let ids: Vec<i64> = integrations.iter().map(|i| i.id).collect();
        let flows: Vec<WhatsappFlow> = find_many(
            conn,
            r#"WHERE "integrationWhatsappId" IN (SELECT value FROM json_each(?1))
               ORDER BY "createdAt", "id""#,
            &[&serde_json::to_string(&ids).unwrap_or_default()],
        )?;
        Ok(flows
            .iter()
            .filter_map(|f| {
                let integration = integrations
                    .iter()
                    .find(|i| i.id == f.integration_whatsapp_id)?;
                let mut out = flow_json(f);
                if with_integration {
                    out["integrationWhatsapp"] = integration_json(integration);
                }
                Some(out)
            })
            .collect())
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

fn auth_text<'a>(auth: &'a Value, pointer: &str) -> &'a str {
    auth.pointer(pointer).and_then(Value::as_str).unwrap_or("")
}

fn version(auth: &Value) -> &str {
    Some(auth_text(auth, "/version"))
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_API_VERSION)
}

async fn get_json(url: &str, token: Option<&str>) -> Result<Value, String> {
    let mut request = http().get(url);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| e.without_url().to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

/// `listFlows`: one page of `/{wabaId}/flows` (the original reads no next page).
async fn list_meta_flows(graph_url: &str, auth: &Value) -> Result<Vec<Value>, Fail> {
    let url = format!(
        "{graph_url}/{}/{}/flows",
        version(auth),
        auth_text(auth, "/metadata/wabaId")
    );
    let body = get_json(&url, Some(auth_text(auth, "/tokens/accessToken")))
        .await
        .map_err(|error| {
            tracing::error!(%error, "Failed to list flows");
            Fail::Sdk("Failed to list flows".to_owned(), 400)
        })?;
    Ok(body["data"].as_array().cloned().unwrap_or_default())
}

/// `downloadFlowJson`. The asset's `download_url` comes from Meta, so it must be public.
async fn download_flow_json(
    graph_url: &str,
    auth: &Value,
    flow_source_id: &str,
) -> Result<Value, String> {
    let url = format!("{graph_url}/{}/{flow_source_id}/assets", version(auth));
    let assets = get_json(&url, Some(auth_text(auth, "/tokens/accessToken"))).await?;
    let download = assets["data"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| {
            a["asset_type"] == "FLOW_JSON"
                || a["name"].as_str().is_some_and(|n| n.ends_with(".json"))
        })
        .and_then(|a| a["download_url"].as_str());
    let Some(download) = download else {
        return Ok(Value::Null);
    };
    assert_public_url(download, "WhatsApp Flow asset").await?;
    get_json(download, None).await
}

/// `getFlowAssets`: `[]` when Meta cannot serve the Flow JSON.
async fn flow_screens(graph_url: &str, auth: &Value, flow_source_id: &str) -> Vec<Value> {
    match download_flow_json(graph_url, auth, flow_source_id).await {
        Ok(flow) => parse_flow_screens(&flow),
        Err(error) => {
            tracing::error!(%error, "Failed to load flow assets");
            Vec::new()
        }
    }
}

/// `getWhatsappFlowScreens`: a missing Flow and a Flow of another workspace read the same.
pub async fn get_screens(
    db: &chatbotx_db::Db,
    graph_url: &str,
    workspace_id: i64,
    flow_id: i64,
) -> Result<Vec<Value>, BusinessError> {
    let found = tx::read(db, move |conn| {
        let flows: Vec<WhatsappFlow> = find_many(conn, r#"WHERE "id" = ?1 LIMIT 1"#, &[&flow_id])?;
        let Some(flow) = flows.into_iter().next() else {
            return Ok(None);
        };
        let integration = chatbotx_db::repo::integration_whatsapp::find_by_id_for_workspace(
            conn,
            flow.integration_whatsapp_id,
            workspace_id,
        )?;
        Ok(integration.map(|i| (flow, i)))
    })
    .await?;
    let (flow, integration) = found.ok_or_else(|| BusinessError::not_found(FLOW_NOT_FOUND))?;
    Ok(flow_screens(graph_url, &integration.auth.0, &flow.source_id).await)
}

/// `whatsappFlowService.syncFromMeta`: full sync (rows Meta no longer has are deleted).
async fn sync_from_meta(
    db: &chatbotx_db::Db,
    integration_id: i64,
    flows: Vec<Value>,
) -> Result<(), BusinessError> {
    tx::write(db, move |conn| {
        let incoming: Vec<String> = flows
            .iter()
            .filter_map(|f| f["id"].as_str().map(str::to_owned))
            .collect();
        conn.execute(
            r#"DELETE FROM "WhatsappFlow" WHERE "integrationWhatsappId" = ?1
               AND "sourceId" NOT IN (SELECT value FROM json_each(?2))"#,
            params![
                integration_id,
                serde_json::to_string(&incoming).unwrap_or_default()
            ],
        )?;
        for f in &flows {
            // A missing list keeps the column default (`[]`), as Drizzle skips `undefined`.
            let field = |k: &str| {
                f.get(k)
                    .filter(|v| !v.is_null())
                    .cloned()
                    .unwrap_or(json!([]))
                    .to_string()
            };
            conn.execute(
                r#"INSERT INTO "WhatsappFlow" ("id", "name", "integrationWhatsappId", "sourceId",
                     "status", "categories", "validationErrors")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                   ON CONFLICT ("integrationWhatsappId", "sourceId") DO UPDATE SET
                     "name" = excluded."name", "status" = excluded."status",
                     "categories" = excluded."categories",
                     "validationErrors" = excluded."validationErrors", "updatedAt" = ?8"#,
                params![
                    new_id(),
                    f["name"].as_str().unwrap_or_default(),
                    integration_id,
                    f["id"].as_str().unwrap_or_default(),
                    f["status"].as_str().unwrap_or_default(),
                    field("categories"),
                    field("validation_errors"),
                    now_ms(),
                ],
            )?;
        }
        Ok(())
    })
    .await
}

/// `syncWhatsappFlows` (shared with the `/rpc` sync action).
pub(crate) async fn sync_integration(
    db: &chatbotx_db::Db,
    graph_url: &str,
    integration: &IntegrationWhatsapp,
) -> Result<(), Fail> {
    let flows = list_meta_flows(graph_url, &integration.auth.0).await?;
    Ok(sync_from_meta(db, integration.id, flows).await?)
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "whatsappFlows.list" => {
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
            input.finish()?;
            let rows = list(db, ws, inbox, integration, false)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            Ok(json(200, Value::Array(rows)))
        }
        "whatsappFlows.getScreens" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.digits_id("flowId");
            input.finish()?;
            let screens = get_screens(db, &state.graph_url, ws, id.unwrap_or(-1))
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, json!({ "screens": screens })))
        }
        _ => {
            // whatsappFlows.sync
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
