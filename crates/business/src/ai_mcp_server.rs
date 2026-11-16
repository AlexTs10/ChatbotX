//! `packages/business/src/ai-mcp-server` (`service.ts`, `endpoint-policy.ts`)
//! and the builder's `validateAIMcpServer` action. No template edit locks /
//! delete guards (P3.6-D1 rule).

use std::sync::Arc;

use chatbotx_core::variables::bot_field::{
    BotFieldVariableTextParseResult, BotFieldVariableTextResolution, parse_bot_field_variable_text,
};
use chatbotx_db::Db;
use chatbotx_db::repo::models::{AiMcpServer, BotField};
use chatbotx_db::repo::utils::{PaginationInput, get_pagination_with_defaults};
use chatbotx_db::repo::{Json, Model, find_many, find_one, new_id, now_ms, returning};
use chatbotx_db::rusqlite::Connection;
use serde_json::{Map, Value};

use crate::ai::mcp_client::{BoxFuture, EndpointGuard, McpAuth, McpClient};
use crate::audit::AuditService;
use crate::errors::{BusinessError, Result};
use crate::net::ssrf_guard::{DohFetch, HttpDoh, SsrfCheck, check_ssrf_safety_with};
use crate::tx;

fn repo_err(error: impl Into<chatbotx_db::repo::Error>) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `CreateAIMcpServerRequest` / `UpdateAIMcpServerRequest` (`auth` is the
/// stored JSON shape).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AiMcpServerInput {
    pub name: String,
    pub url: String,
    pub auth: Value,
    pub available_tools: Map<String, Value>,
    pub selected_tools: Vec<String>,
}

impl AiMcpServerInput {
    fn same_as(&self, row: &AiMcpServer) -> bool {
        row.name == self.name
            && row.url == self.url
            && row.auth.0 == self.auth
            && row.available_tools.0 == Value::Object(self.available_tools.clone())
            && row.selected_tools.0
                == Value::Array(
                    self.selected_tools
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                )
    }
}

/// `listAIMcpServers` result.
#[derive(Debug, Clone, PartialEq)]
pub struct AiMcpServerPage {
    pub data: Vec<AiMcpServer>,
    pub page_count: i64,
}

/// `findBy({ id, workspaceId })`.
pub fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<AiMcpServer>> {
    find_one(
        conn,
        r#""id" = ?1 AND "workspaceId" = ?2"#,
        &[&id, &workspace_id],
    )
    .map_err(repo_err)
}

/// `findBy`.
pub async fn find(db: &Db, workspace_id: i64, id: i64) -> Result<Option<AiMcpServer>> {
    tx::read(db, move |conn| find_in(conn, workspace_id, id)).await
}

fn find_by_name(conn: &Connection, workspace_id: i64, name: &str) -> Result<Option<AiMcpServer>> {
    find_one(
        conn,
        r#""workspaceId" = ?1 AND "name" = ?2"#,
        &[&workspace_id, &name],
    )
    .map_err(repo_err)
}

/// `listAIMcpServers`: newest first; without `page` / `perPage` every row
/// and `pageCount: 1`.
pub async fn list(db: &Db, workspace_id: i64, paging: PaginationInput) -> Result<AiMcpServerPage> {
    tx::read(db, move |conn| {
        let order = r#"WHERE "workspaceId" = ?1 ORDER BY "createdAt" DESC, "id" DESC"#;
        if paging.page.is_none() && paging.per_page.is_none() {
            let data = find_many(conn, order, &[&workspace_id]).map_err(repo_err)?;
            return Ok(AiMcpServerPage {
                data,
                page_count: 1,
            });
        }
        let p = get_pagination_with_defaults(paging);
        let data = find_many(
            conn,
            &format!("{order} LIMIT ?2 OFFSET ?3"),
            &[&workspace_id, &p.limit, &p.offset],
        )
        .map_err(repo_err)?;
        let total: i64 = conn
            .query_row(
                r#"SELECT count(*) FROM "AIMCPServer" WHERE "workspaceId" = ?1"#,
                [workspace_id],
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        Ok(AiMcpServerPage {
            data,
            page_count: (total + p.limit - 1) / p.limit,
        })
    })
    .await
}

fn tools_json(input: &AiMcpServerInput) -> (Json, Json) {
    (
        Json(Value::Object(input.available_tools.clone())),
        Json(Value::Array(
            input
                .selected_tools
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        )),
    )
}

/// `create`: 422 `Name is already taken`, insert, audit.
pub async fn create(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    input: AiMcpServerInput,
) -> Result<AiMcpServer> {
    let created = tx::write(db, move |tx| {
        if find_by_name(tx, workspace_id, &input.name)?.is_some() {
            return Err(BusinessError::validation("name", "Name is already taken"));
        }
        let (available, selected) = tools_json(&input);
        let sql = format!(
            r#"INSERT INTO {} ("id", "createdAt", "updatedAt", "name", "url", "auth", "availableTools", "selectedTools", "workspaceId")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
            AiMcpServer::TABLE
        );
        returning::<AiMcpServer>(
            tx,
            &sql,
            &[
                &new_id(),
                &now_ms(),
                &input.name,
                &input.url,
                &Json(input.auth.clone()),
                &available,
                &selected,
                &workspace_id,
            ],
        )
        .map_err(repo_err)?
        .pop()
        .ok_or_else(|| BusinessError::not_found("AI MCP server not found"))
    })
    .await?;
    audit
        .audit(
            "create",
            &format!("created a new MCP Server (#{})", created.id),
        )
        .await;
    Ok(created)
}

/// `update`: 422 `Name is already taken` (another row of the name), write;
/// `None` = no row of the workspace (no audit). Audits when a field changed.
pub async fn update(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    id: i64,
    input: AiMcpServerInput,
) -> Result<Option<AiMcpServer>> {
    let outcome = tx::write(db, move |tx| {
        if find_by_name(tx, workspace_id, &input.name)?.is_some_and(|row| row.id != id) {
            return Err(BusinessError::validation("name", "Name is already taken"));
        }
        let Some(existing) = find_in(tx, workspace_id, id)? else {
            return Ok(None);
        };
        let changed = !input.same_as(&existing);
        let (available, selected) = tools_json(&input);
        let sql = format!(
            r#"UPDATE {} SET "name" = ?1, "url" = ?2, "auth" = ?3, "availableTools" = ?4,
                 "selectedTools" = ?5, "updatedAt" = ?6
               WHERE "id" = ?7 AND "workspaceId" = ?8"#,
            AiMcpServer::TABLE
        );
        let row = returning::<AiMcpServer>(
            tx,
            &sql,
            &[
                &input.name,
                &input.url,
                &Json(input.auth.clone()),
                &available,
                &selected,
                &now_ms(),
                &id,
                &workspace_id,
            ],
        )
        .map_err(repo_err)?
        .pop();
        Ok(row.map(|row| (row, changed)))
    })
    .await?;
    let Some((row, changed)) = outcome else {
        return Ok(None);
    };
    if changed {
        audit
            .audit("update", &format!("updated an MCP Server (#{id})"))
            .await;
    }
    Ok(Some(row))
}

/// `delete`: the deleted ids (empty = no row; no audit).
pub async fn delete(db: &Db, audit: &AuditService, workspace_id: i64, id: i64) -> Result<Vec<i64>> {
    let deleted = tx::write(db, move |tx| delete_in(tx, workspace_id, &[id])).await?;
    if !deleted.is_empty() {
        audit
            .audit("delete", &format!("deleted an MCP Server (#{id})"))
            .await;
    }
    Ok(deleted)
}

/// `deleteInTx`.
pub fn delete_in(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    for id in ids {
        let n = conn
            .execute(
                r#"DELETE FROM "AIMCPServer" WHERE "workspaceId" = ?1 AND "id" = ?2"#,
                [workspace_id, *id],
            )
            .map_err(repo_err)?;
        if n > 0 {
            out.push(*id);
        }
    }
    Ok(out)
}

// ---- endpoint-policy.ts ----

fn reject_endpoint() -> BusinessError {
    BusinessError::exception(
        "MCP server endpoint is not permitted.",
        "invalidMcpServerEndpoint",
        400,
    )
}

/// `assertSafeMcpEndpoint` with an explicit edition and DoH client. Self-hosted:
/// any http(s) URL. Cloud: https, no metadata / `*.localhost` host, and the
/// shared SSRF guard must call it public.
pub async fn assert_safe_mcp_endpoint_with(
    url_value: &str,
    cloud: bool,
    doh: &impl DohFetch,
) -> Result<()> {
    let url = url::Url::parse(url_value).map_err(|_| reject_endpoint())?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(reject_endpoint());
    }
    if !cloud {
        return Ok(());
    }
    if url.scheme() != "https" {
        return Err(reject_endpoint());
    }
    let hostname = url.host_str().unwrap_or_default().to_lowercase();
    if hostname == "metadata.google.internal"
        || hostname == "metadata.google.internal."
        || hostname.ends_with(".localhost")
    {
        return Err(reject_endpoint());
    }
    match check_ssrf_safety_with(url_value, doh).await {
        SsrfCheck::Unsafe => Err(reject_endpoint()),
        SsrfCheck::Safe { .. } => Ok(()),
    }
}

/// `isCloud()` (`NEXT_PUBLIC_EDITION=cloud`).
pub fn is_cloud() -> bool {
    std::env::var("NEXT_PUBLIC_EDITION").is_ok_and(|edition| edition == "cloud")
}

/// `assertSafeMcpEndpoint` as an [`EndpointGuard`] (the real DoH client).
#[derive(Debug, Clone, Copy)]
pub struct McpEndpointPolicy {
    pub cloud: bool,
}

impl McpEndpointPolicy {
    /// The policy of this process's edition.
    pub fn from_env() -> McpEndpointPolicy {
        McpEndpointPolicy { cloud: is_cloud() }
    }
}

impl EndpointGuard for McpEndpointPolicy {
    fn check<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(assert_safe_mcp_endpoint_with(url, self.cloud, &HttpDoh))
    }
}

// ---- bot field token templates ----

/// `resolveBotFieldVariableText` (no cache): `{{bot_field:<id>}}` replaced
/// by the workspace's Bot Field values.
pub async fn resolve_bot_field_variable_text(
    db: &Db,
    workspace_id: i64,
    text: &str,
) -> Result<BotFieldVariableTextResolution> {
    let field_ids = match parse_bot_field_variable_text(text) {
        BotFieldVariableTextParseResult::Malformed => {
            return Ok(BotFieldVariableTextResolution::Malformed);
        }
        BotFieldVariableTextParseResult::Valid { field_ids } => field_ids,
    };
    if field_ids.is_empty() {
        return Ok(BotFieldVariableTextResolution::Resolved {
            value: text.to_string(),
        });
    }
    let fields: Vec<BotField> = tx::read(db, move |conn| {
        find_many(conn, r#"WHERE "workspaceId" = ?1"#, &[&workspace_id]).map_err(repo_err)
    })
    .await?;
    let mut value = text.to_string();
    for field_id in field_ids {
        let Some(field) = fields.iter().find(|f| f.id.to_string() == field_id) else {
            return Ok(BotFieldVariableTextResolution::Missing { field_id });
        };
        let Some(field_value) = field.value.as_deref().filter(|v| !v.is_empty()) else {
            return Ok(BotFieldVariableTextResolution::Empty { field_id });
        };
        value = value.replace(&format!("{{{{bot_field:{field_id}}}}}"), field_value);
    }
    Ok(BotFieldVariableTextResolution::Resolved { value })
}

// ---- validate-ai-mcp-server.action.ts ----

/// `validateAIMcpServer`: the endpoint guard first (nothing is resolved or
/// sent for a refused endpoint), then the auth headers (a token template
/// that does not resolve is `400 invalidMcpTokenTemplate`), then the server's
/// tools as `@ai-sdk/mcp` JSON.
pub async fn validate(
    db: &Db,
    guard: Arc<dyn EndpointGuard>,
    workspace_id: i64,
    url: &str,
    auth: &Value,
) -> Result<Map<String, Value>> {
    guard.check(url).await?;
    let headers = match McpAuth::from_json(auth) {
        McpAuth::Token(token) => {
            match resolve_bot_field_variable_text(db, workspace_id, &token).await? {
                BotFieldVariableTextResolution::Resolved { value } => {
                    vec![("Authorization".to_string(), format!("Bearer {value}"))]
                }
                _ => {
                    return Err(BusinessError::exception(
                        "Unable to validate MCP server.",
                        "invalidMcpTokenTemplate",
                        400,
                    ));
                }
            }
        }
        other => other.headers(),
    };
    McpClient::sdk(url, headers).sdk_tools().await
}
