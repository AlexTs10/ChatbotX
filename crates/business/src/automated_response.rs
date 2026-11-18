//! `automated-response/service.ts` (`automatedResponseService`): keyword
//! automations (`"AutomatedResponse"` rows). Inbound rules always store
//! `conditions` (plain keywords become one `contains` condition) and mirror
//! the trigger words in `keywords`; outbound rules keep exact `keywords`.
//!
//! No read cache (`invalidateCache` has nothing to clear) and no template
//! locks / guards (P3.6-D1 decision); a missing reply flow on create is the
//! `notFound` "Flow not found" of `assertReferencable`. The services do not
//! write audit records: they return what the caller needs to decide
//! (`Updated::changed`, `StatusChange`, the deleted ids) and the `*_audit`
//! functions build the original detail texts.

use chatbotx_db::Db;
use chatbotx_db::repo::models::AutomatedResponse;
use chatbotx_db::repo::partials::automated_response::{
    KeywordCondition, flatten_keyword_condition_values,
};
use chatbotx_db::repo::partials::keyword_condition_schema::parse_keyword_conditions;
use chatbotx_db::repo::{Model, flow, like_contains, new_id, now_ms};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params, params_from_iter};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `AutomatedResponseType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResponseType {
    /// Contact messages (the default).
    #[default]
    Inbound,
    /// Page / agent messages.
    Outbound,
}

impl ResponseType {
    /// The stored `type` text.
    pub fn as_str(self) -> &'static str {
        match self {
            ResponseType::Inbound => "inbound",
            ResponseType::Outbound => "outbound",
        }
    }

    /// `automatedResponseFolderTypeByType`.
    pub fn folder_type(self) -> &'static str {
        match self {
            ResponseType::Inbound => "automatedResponse",
            ResponseType::Outbound => "outboundAutomatedResponse",
        }
    }
}

/// `KeywordRuleMatch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeywordRuleMatch {
    pub keywords: Vec<String>,
    pub conditions: Option<Vec<KeywordCondition>>,
}

fn parsed(field: &str, fallback: &str, value: &Value) -> Result<KeywordRuleMatch> {
    match parse_keyword_conditions(value) {
        Ok(conditions) => Ok(KeywordRuleMatch {
            keywords: flatten_keyword_condition_values(&conditions),
            conditions: Some(conditions),
        }),
        Err(error) => Err(BusinessError::validation(
            field,
            error
                .issues
                .into_iter()
                .next()
                .map(|issue| issue.message)
                .unwrap_or_else(|| fallback.to_string()),
        )),
    }
}

/// `resolveKeywordRuleMatch`: what a rule matches on (`conditions` = the
/// raw JSON sent, checked with `keywordConditionsSchema`).
pub fn resolve_keyword_rule_match(
    kind: ResponseType,
    keywords: Option<&[String]>,
    conditions: Option<&Value>,
) -> Result<KeywordRuleMatch> {
    let conditions = conditions.filter(|c| !c.is_null());
    if conditions.is_some() && keywords.is_some() {
        return Err(BusinessError::validation(
            "conditions",
            "Send either keywords or conditions, not both",
        ));
    }
    if let Some(conditions) = conditions {
        if kind != ResponseType::Inbound {
            return Err(BusinessError::validation(
                "conditions",
                "Conditions are only supported for contact (inbound) keywords",
            ));
        }
        return parsed("conditions", "Invalid keyword conditions", conditions);
    }
    let keywords: Vec<String> = keywords
        .unwrap_or_default()
        .iter()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();
    if keywords.is_empty() {
        return Err(BusinessError::validation(
            "keywords",
            "At least one keyword is required",
        ));
    }
    if kind != ResponseType::Inbound {
        return Ok(KeywordRuleMatch {
            keywords,
            conditions: None,
        });
    }
    parsed(
        "keywords",
        "Invalid keywords",
        &json!([{ "operator": "contains", "values": keywords }]),
    )
}

/// The `create` values.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreateAutomatedResponse {
    pub kind: ResponseType,
    pub text: Option<String>,
    /// A bigint id text; blank = not set (the form sends `""`).
    pub flow_id: Option<String>,
    pub folder_id: Option<i64>,
    pub keywords: Option<Vec<String>>,
    pub conditions: Option<Value>,
}

/// `UpdateAutomatedResponseRequest` (`None` = not sent; `Some(None)` = null).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateAutomatedResponse {
    pub folder_id: Option<Option<i64>>,
    pub keywords: Option<Vec<String>>,
    pub conditions: Option<Value>,
    pub text: Option<Option<String>>,
    pub flow_id: Option<Option<i64>>,
}

/// The `update` result: the stored row and whether a field really changed
/// (the audit condition).
#[derive(Debug, Clone, PartialEq)]
pub struct Updated {
    pub row: AutomatedResponse,
    pub changed: bool,
}

/// The `setStatus` result: the row (`None` = no row of that id and type)
/// and whether the status changed (the audit condition).
#[derive(Debug, Clone, PartialEq)]
pub struct StatusChange {
    pub row: Option<AutomatedResponse>,
    pub changed: bool,
}

/// `ListAutomatedResponsesRequest` (sort: `createdAt` desc, `id` desc).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListAutomatedResponses {
    pub workspace_id: i64,
    pub kind: ResponseType,
    /// `"0"` = the top level (no folder).
    pub folder_id: Option<String>,
    pub keyword: Option<String>,
    pub page: i64,
    pub per_page: i64,
}

/// `PaginatedResult<AutomatedResponseModel>`.
#[derive(Debug, Clone, PartialEq)]
pub struct AutomatedResponsePage {
    pub data: Vec<AutomatedResponse>,
    pub page_count: i64,
}

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Automated response not found")
}

fn find_in(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
    kind: ResponseType,
) -> Result<Option<AutomatedResponse>> {
    conn.query_row(
        &format!(
            r#"{} WHERE "workspaceId" = ?1 AND "id" = ?2 AND "type" = ?3"#,
            AutomatedResponse::select_sql()
        ),
        params![workspace_id, id, kind.as_str()],
        AutomatedResponse::from_row,
    )
    .optional()
    .map_err(repo_err)
}

fn stored_conditions(row: &AutomatedResponse) -> Option<Vec<KeywordCondition>> {
    row.conditions
        .as_ref()
        .and_then(|c| serde_json::from_value(c.0.clone()).ok())
}

fn stored_keywords(row: &AutomatedResponse) -> Vec<String> {
    serde_json::from_value(row.keywords.0.clone()).unwrap_or_default()
}

fn conditions_json(conditions: &Option<Vec<KeywordCondition>>) -> Option<String> {
    conditions
        .as_ref()
        .map(|c| serde_json::to_string(c).unwrap_or_else(|_| "[]".into()))
}

/// `findBy`: the row of that id and type in the workspace.
pub async fn find_by(
    db: &Db,
    workspace_id: i64,
    id: i64,
    kind: ResponseType,
) -> Result<Option<AutomatedResponse>> {
    tx::read(db, move |conn| find_in(conn, workspace_id, id, kind)).await
}

/// `findOrFail`: 404 "Automated response not found".
pub async fn find_or_fail(
    db: &Db,
    workspace_id: i64,
    id: i64,
    kind: ResponseType,
) -> Result<AutomatedResponse> {
    find_by(db, workspace_id, id, kind)
        .await?
        .ok_or_else(not_found)
}

/// `findByInboundKeyword`: an inbound rule whose `keywords` hold `keyword`
/// exactly (`@> ARRAY[keyword]`), any status.
pub async fn find_by_inbound_keyword(
    db: &Db,
    workspace_id: i64,
    keyword: &str,
) -> Result<Option<AutomatedResponse>> {
    let keyword = keyword.to_string();
    tx::read(db, move |conn| {
        // ponytail: the lowest id wins (the original has no order).
        conn.query_row(
            &format!(
                r#"{} WHERE "workspaceId" = ?1 AND "type" = 'inbound'
                   AND EXISTS (SELECT 1 FROM json_each("keywords") k WHERE k.value = ?2)
                   ORDER BY "id" LIMIT 1"#,
                AutomatedResponse::select_sql()
            ),
            params![workspace_id, keyword],
            AutomatedResponse::from_row,
        )
        .optional()
        .map_err(repo_err)
    })
    .await
}

/// `list`: one type, paged.
pub async fn list(db: &Db, input: ListAutomatedResponses) -> Result<AutomatedResponsePage> {
    tx::read(db, move |conn| {
        let mut filter = String::from(r#""workspaceId" = ? AND "type" = ?"#);
        let mut binds = vec![
            Sql::Integer(input.workspace_id),
            Sql::Text(input.kind.as_str().into()),
        ];
        if let Some(keyword) = input.keyword.as_deref().filter(|k| !k.is_empty()) {
            filter.push_str(r#" AND ilike("keywords", ?)"#);
            binds.push(Sql::Text(like_contains(keyword)));
        }
        match input.folder_id.as_deref().filter(|f| !f.is_empty()) {
            Some("0") => filter.push_str(r#" AND "folderId" IS NULL"#),
            Some(folder) => {
                filter.push_str(r#" AND "folderId" = ?"#);
                binds.push(Sql::Integer(folder.parse().unwrap_or(-1)));
            }
            None => {}
        }
        let total: i64 = conn
            .query_row(
                &format!(r#"SELECT count(*) FROM "AutomatedResponse" WHERE {filter}"#),
                params_from_iter(binds.iter()),
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        let per_page = input.per_page.max(1);
        binds.push(Sql::Integer(per_page));
        binds.push(Sql::Integer((input.page.max(1) - 1) * per_page));
        let mut stmt = conn
            .prepare(&format!(
                r#"{} WHERE {filter} ORDER BY "createdAt" DESC, "id" DESC LIMIT ? OFFSET ?"#,
                AutomatedResponse::select_sql()
            ))
            .map_err(repo_err)?;
        let data = stmt
            .query_map(params_from_iter(binds.iter()), AutomatedResponse::from_row)
            .map_err(repo_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok(AutomatedResponsePage {
            data,
            page_count: (total + per_page - 1) / per_page,
        })
    })
    .await
}

/// `create`: a new enabled rule.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    values: CreateAutomatedResponse,
) -> Result<AutomatedResponse> {
    let matched = resolve_keyword_rule_match(
        values.kind,
        values.keywords.as_deref(),
        values.conditions.as_ref(),
    )?;
    tx::write(db, move |conn| {
        let flow_id = values.flow_id.as_deref().filter(|f| !f.is_empty());
        let text = values.text.filter(|t| !t.is_empty());
        let flow_id = match flow_id {
            // `assertReferencable`: an unknown / foreign flow is 404.
            Some(raw) => {
                let id = raw.parse::<i64>().ok().filter(|id| {
                    flow::exists_in_workspace(conn, workspace_id, *id).unwrap_or(false)
                });
                Some(id.ok_or_else(|| BusinessError::not_found("Flow not found"))?)
            }
            None => None,
        };
        if flow_id.is_some() && text.is_some() {
            return Err(BusinessError::validation(
                "flowId",
                "A keyword replies with either text or a flow, not both",
            ));
        }
        if let Some(folder) = values.folder_id {
            conn.query_row(
                r#"SELECT 1 FROM "Folder" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "folderType" = ?3"#,
                params![folder, workspace_id, values.kind.folder_type()],
                |_| Ok(()),
            )
            .optional()
            .map_err(repo_err)?
            .ok_or_else(|| BusinessError::not_found("Folder not found"))?;
        }
        let id = new_id();
        let now = now_ms();
        conn.execute(
            r#"INSERT INTO "AutomatedResponse" ("id", "createdAt", "updatedAt", "workspaceId", "folderId", "keywords", "conditions", "status", "text", "flowId", "type")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, ?9)"#,
            params![
                id,
                now,
                workspace_id,
                values.folder_id,
                serde_json::to_string(&matched.keywords).unwrap_or_else(|_| "[]".into()),
                conditions_json(&matched.conditions),
                text,
                flow_id,
                values.kind.as_str()
            ],
        )
        .map_err(repo_err)?;
        find_in(conn, workspace_id, id, values.kind)?.ok_or_else(not_found)
    })
    .await
}

/// `update`: text and flow replace each other; keywords / conditions are
/// only touched when sent. 404 "Automated response not found".
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    kind: ResponseType,
    data: UpdateAutomatedResponse,
) -> Result<Updated> {
    tx::write(db, move |conn| {
        crate::template::guard::assert_content_editable(
            conn,
            workspace_id,
            "automatedResponse",
            &[id],
        )?;
        let existing = find_in(conn, workspace_id, id, kind)?;
        let mut next_flow = data.flow_id;
        let mut next_text = data.text.clone();
        if data
            .text
            .as_ref()
            .is_some_and(|t| t.as_ref().is_some_and(|t| !t.is_empty()))
        {
            next_flow = Some(None);
        } else if let Some(Some(flow_id)) = data.flow_id {
            if !flow::exists_in_workspace(conn, workspace_id, flow_id)? {
                return Err(BusinessError::validation("flowId", "Flow not found"));
            }
            next_text = Some(None);
        }
        let next_match = if data.keywords.is_some() || data.conditions.is_some() {
            Some(resolve_keyword_rule_match(
                kind,
                data.keywords.as_deref(),
                data.conditions.as_ref(),
            )?)
        } else {
            None
        };
        let Some(existing) = existing else {
            return Err(not_found());
        };
        let mut sets = vec![r#""updatedAt" = ?"#.to_string()];
        let mut binds = vec![Sql::Integer(now_ms())];
        let opt_int = |v: Option<i64>| v.map_or(Sql::Null, Sql::Integer);
        if let Some(folder) = data.folder_id {
            sets.push(r#""folderId" = ?"#.into());
            binds.push(opt_int(folder));
        }
        if let Some(text) = &next_text {
            sets.push(r#""text" = ?"#.into());
            binds.push(text.clone().map_or(Sql::Null, Sql::Text));
        }
        if let Some(flow) = next_flow {
            sets.push(r#""flowId" = ?"#.into());
            binds.push(opt_int(flow));
        }
        if let Some(m) = &next_match {
            sets.push(r#""keywords" = ?"#.into());
            binds.push(Sql::Text(
                serde_json::to_string(&m.keywords).unwrap_or_else(|_| "[]".into()),
            ));
            sets.push(r#""conditions" = ?"#.into());
            binds.push(conditions_json(&m.conditions).map_or(Sql::Null, Sql::Text));
        }
        binds.push(Sql::Integer(id));
        conn.execute(
            &format!(
                r#"UPDATE "AutomatedResponse" SET {} WHERE "id" = ?"#,
                sets.join(", ")
            ),
            params_from_iter(binds.iter()),
        )
        .map_err(repo_err)?;
        let row = find_in(conn, workspace_id, id, kind)?.ok_or_else(not_found)?;
        let changed = next_match.as_ref().is_some_and(|m| {
            m.keywords != stored_keywords(&existing)
                || m.conditions.clone().unwrap_or_default()
                    != stored_conditions(&existing).unwrap_or_default()
        }) || data.folder_id.is_some_and(|f| f != existing.folder_id)
            || next_text.as_ref().is_some_and(|t| *t != existing.text)
            || next_flow.is_some_and(|f| f != existing.flow_id);
        Ok(Updated { row, changed })
    })
    .await
}

/// `setStatus`.
pub async fn set_status(
    db: &Db,
    workspace_id: i64,
    id: i64,
    kind: ResponseType,
    status: bool,
) -> Result<StatusChange> {
    tx::write(db, move |conn| {
        let Some(existing) = find_in(conn, workspace_id, id, kind)? else {
            return Ok(StatusChange {
                row: None,
                changed: false,
            });
        };
        conn.execute(
            r#"UPDATE "AutomatedResponse" SET "status" = ?1, "updatedAt" = ?2 WHERE "id" = ?3"#,
            params![status, now_ms(), id],
        )
        .map_err(repo_err)?;
        Ok(StatusChange {
            row: find_in(conn, workspace_id, id, kind)?,
            changed: existing.status != status,
        })
    })
    .await
}

/// `deleteMany` / `deleteInTx`: the ids actually deleted.
pub async fn delete_many(
    db: &Db,
    workspace_id: i64,
    ids: Vec<i64>,
    kind: ResponseType,
) -> Result<Vec<i64>> {
    tx::write(db, move |conn| {
        crate::template::guard::assert_deletable(conn, workspace_id, "automatedResponse", &ids)?;
        let list = serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into());
        let mut stmt = conn
            .prepare(
                r#"DELETE FROM "AutomatedResponse" WHERE "workspaceId" = ?1 AND "type" = ?2
                   AND "id" IN (SELECT value FROM json_each(?3)) RETURNING "id""#,
            )
            .map_err(repo_err)?;
        let mut deleted = stmt
            .query_map(params![workspace_id, kind.as_str(), list], |r| r.get(0))
            .map_err(repo_err)?
            .collect::<Result<Vec<i64>, _>>()
            .map_err(repo_err)?;
        deleted.sort_unstable();
        Ok(deleted)
    })
    .await
}

/// The `create` audit detail.
pub fn create_audit(id: i64) -> String {
    format!("created a new keyword automation (#{id})")
}

/// The `update` audit detail.
pub fn update_audit(id: i64) -> String {
    format!("updated a keyword automation (#{id})")
}

/// The `setStatus` audit detail.
pub fn status_audit(id: i64, status: bool) -> String {
    let verb = if status { "enabled" } else { "disabled" };
    format!("{verb} a keyword automation (#{id})")
}

/// The `deleteMany` audit detail (`None` = nothing deleted, no record).
pub fn delete_audit(ids: &[i64]) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let plural = if ids.len() > 1 { "s" } else { "" };
    let list: Vec<String> = ids.iter().map(|id| format!("#{id}")).collect();
    Some(format!(
        "deleted keyword automation{plural} ({})",
        list.join(", ")
    ))
}
