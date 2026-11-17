//! `packages/business/src/ai-agent/service.ts`: the AI agents of a workspace
//! (`"AIAgent"`). No list / default caches and no template edit locks,
//! delete guards or `assertReferencable` (P3.6-D1 rule).

use std::collections::BTreeSet;

use chatbotx_db::Db;
use chatbotx_db::repo::models::AiAgent;
use chatbotx_db::repo::partials::ai::{AiAgentAction, parse_ai_agent_action_rules};
use chatbotx_db::repo::utils::{
    PaginationInput, SortItem, get_pagination_with_defaults, order_by_sql, parse_order_by,
    with_id_tie_break,
};
use chatbotx_db::repo::{
    Json, Model, find_many, find_one, like_contains, new_id, now_ms, returning,
};
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::audit::AuditService;
use crate::errors::{BusinessError, Result};
use crate::tx;

fn repo_err(error: impl Into<chatbotx_db::repo::Error>) -> BusinessError {
    BusinessError::Repo(error.into())
}

const FILE_TOOL_PREFIX: &str = "file:";

/// `CreateAIAgentRequest` / `UpdateAIAgentRequest`: `None` = not given.
/// Create fills a missing field with the schema default.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AiAgentInput {
    pub name: Option<String>,
    pub prompt: Option<String>,
    pub messages: Option<Value>,
    pub models: Option<Value>,
    pub temperature: Option<f64>,
    pub max_output_tokens: Option<i64>,
    pub tools: Option<Vec<String>>,
    /// The raw `{ value }` strings (normalized by the service).
    pub web_search_authorized_domains: Option<Vec<String>>,
    pub is_default: Option<bool>,
    pub is_rich_response: Option<bool>,
    pub action_prompt: Option<Option<String>>,
    pub action_rules: Option<Value>,
}

impl AiAgentInput {
    fn is_empty(&self) -> bool {
        *self == AiAgentInput::default()
    }

    /// `Object.keys(data)` is exactly `["isDefault"]`.
    fn only_is_default(&self) -> bool {
        self.is_default.is_some()
            && AiAgentInput {
                is_default: None,
                ..self.clone()
            }
            .is_empty()
    }
}

/// `listAIAgents` input.
#[derive(Debug, Clone, Default)]
pub struct ListAiAgents {
    pub paging: PaginationInput,
    pub sort: Vec<SortItem>,
    pub name: Option<String>,
}

/// `listAIAgents` result.
#[derive(Debug, Clone, PartialEq)]
pub struct AiAgentPage {
    pub data: Vec<AiAgent>,
    pub page_count: i64,
}

/// `normalizeWebSearchDomains`: trimmed, lower case, unique, in order.
pub fn normalize_web_search_domains(domains: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for domain in domains {
        let normalized = domain.trim().to_lowercase();
        if !normalized.is_empty() && !out.contains(&normalized) {
            out.push(normalized);
        }
    }
    out
}

fn filter_tools(tools: &[String], files: bool) -> Vec<String> {
    tools
        .iter()
        .filter(|tool| tool.starts_with(FILE_TOOL_PREFIX) == files)
        .cloned()
        .collect()
}

fn stored_strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// `isSameStringSet`: sorted lists are equal.
fn same_string_set(a: &[String], b: &[String]) -> bool {
    let mut left = a.to_vec();
    let mut right = b.to_vec();
    left.sort();
    right.sort();
    left == right
}

fn same_tools(a: &[String], b: &[String], files: bool) -> bool {
    same_string_set(&filter_tools(a, files), &filter_tools(b, files))
}

/// `hasOtherFieldChanges`.
fn has_other_field_changes(agent: &AiAgent, data: &AiAgentInput) -> bool {
    let tools = stored_strings(&agent.tools.0);
    data.name.as_ref().is_some_and(|v| *v != agent.name)
        || data.temperature.is_some_and(|v| v != agent.temperature)
        || data
            .max_output_tokens
            .is_some_and(|v| v != agent.max_output_tokens)
        || data
            .is_rich_response
            .is_some_and(|v| v != agent.is_rich_response)
        || data
            .messages
            .as_ref()
            .is_some_and(|v| *v != agent.messages.0)
        || data
            .web_search_authorized_domains
            .as_ref()
            .is_some_and(|v| {
                !same_string_set(
                    &normalize_web_search_domains(v),
                    &stored_strings(&agent.web_search_authorized_domains.0),
                )
            })
        || data
            .tools
            .as_ref()
            .is_some_and(|v| !same_tools(v, &tools, false))
        || data
            .action_prompt
            .as_ref()
            .is_some_and(|v| *v != agent.action_prompt)
        || data
            .action_rules
            .as_ref()
            .is_some_and(|v| *v != agent.action_rules.0)
}

fn change_group_message(groups: &[&str]) -> String {
    match groups {
        [one] => format!("updated the AI Agent {one}"),
        [a, b] => format!("updated the AI Agent {a} and {b}"),
        [rest @ .., last] => format!("updated the AI Agent {} and {last}", rest.join(", ")),
        [] => String::new(),
    }
}

/// The audit detail of an update of `before` with `data` (`None`: no audit).
fn update_audit_detail(before: &AiAgent, data: &AiAgentInput) -> Option<String> {
    let id = before.id;
    if data.only_is_default() {
        let wanted = data.is_default?;
        return (wanted != before.is_default).then(|| {
            if wanted {
                format!("set as default an AI Agent (#{id})")
            } else {
                format!("unset default an AI Agent (#{id})")
            }
        });
    }
    let mut groups = Vec::new();
    if data.models.as_ref().is_some_and(|v| *v != before.models.0) {
        groups.push("model");
    }
    if data
        .prompt
        .as_ref()
        .is_some_and(|v| Some(v) != before.prompt.as_ref())
    {
        groups.push("instructions");
    }
    if data
        .tools
        .as_ref()
        .is_some_and(|v| !same_tools(v, &stored_strings(&before.tools.0), true))
    {
        groups.push("knowledge base");
    }
    if !groups.is_empty() {
        return Some(format!("{} (#{id})", change_group_message(&groups)));
    }
    has_other_field_changes(before, data).then(|| format!("updated an AI Agent (#{id})"))
}

fn id_of(text: &str) -> Option<i64> {
    text.parse().ok()
}

fn count_in(conn: &Connection, sql: &str, workspace_id: i64, ids: &BTreeSet<i64>) -> Result<i64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let list = Value::Array(ids.iter().map(|id| json!(id)).collect()).to_string();
    conn.query_row(sql, params![workspace_id, list], |r| r.get(0))
        .map_err(repo_err)
}

/// Collects ids; a text that is not a number never matches a row.
fn ids_or_missing(texts: &[&str]) -> (BTreeSet<i64>, usize) {
    let unique: BTreeSet<&str> = texts.iter().copied().collect();
    let ids: BTreeSet<i64> = unique.iter().filter_map(|t| id_of(t)).collect();
    (ids, unique.len())
}

/// `isWorkspaceAdminMember`: the owner, or a member with `superAdmin`.
fn is_admin(role: &str, permissions: &Value) -> bool {
    role == "owner" || permissions.get("superAdmin") == Some(&Value::Bool(true))
}

/// `validateActionRules`: the schema, then every referenced flow (active and
/// published), tag, custom field, admin member and inbox team.
pub fn validate_action_rules(conn: &Connection, workspace_id: i64, rules: &Value) -> Result<Value> {
    let parsed = parse_ai_agent_action_rules(rules).map_err(|error| {
        BusinessError::validation(
            "actionRules",
            error
                .issues
                .first()
                .map(|i| i.message.clone())
                .unwrap_or_else(|| "Invalid input".into()),
        )
    })?;
    let (mut flows, mut tags, mut fields, mut assigned) = (vec![], vec![], vec![], vec![]);
    for action in parsed.iter().flat_map(|rule| &rule.actions) {
        match action {
            AiAgentAction::SendFlow { flow_id, .. } => flows.push(flow_id.as_str()),
            AiAgentAction::AddTag { tag_id, .. } | AiAgentAction::RemoveTag { tag_id, .. } => {
                tags.push(tag_id.as_str())
            }
            AiAgentAction::SetCustomField {
                custom_field_id, ..
            }
            | AiAgentAction::ClearCustomField {
                custom_field_id, ..
            } => fields.push(custom_field_id.as_str()),
            AiAgentAction::AssignConversation {
                assigned_id,
                admin_id,
                ..
            } => assigned.push(match (assigned_id, admin_id) {
                (Some(id), _) => id.clone(),
                (None, Some(admin)) => format!("u_{admin}"),
                (None, None) => String::new(),
            }),
            _ => {}
        }
    }
    let fail = |message: &str| Err(BusinessError::validation("actionRules", message));

    for flow in flows.iter().collect::<BTreeSet<_>>() {
        let published = match id_of(flow) {
            Some(id) => conn
                .query_row(
                    r#"SELECT "currentVersionId" IS NOT NULL FROM "Flow"
                       WHERE "id" = ?1 AND "workspaceId" = ?2 AND "active" = 1"#,
                    params![id, workspace_id],
                    |r| r.get::<_, bool>(0),
                )
                .unwrap_or(false),
            None => false,
        };
        if !published {
            return fail("A selected flow must be active and published.");
        }
    }
    let (ids, wanted) = ids_or_missing(&tags);
    let found = count_in(
        conn,
        r#"SELECT count(*) FROM "Tag" WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL AND "id" IN (SELECT value FROM json_each(?2))"#,
        workspace_id,
        &ids,
    )?;
    if found as usize != wanted {
        return fail("A selected tag is unavailable.");
    }
    let (ids, wanted) = ids_or_missing(&fields);
    let found = count_in(
        conn,
        r#"SELECT count(*) FROM "CustomField" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2))"#,
        workspace_id,
        &ids,
    )?;
    if found as usize != wanted {
        return fail("A selected custom field is unavailable.");
    }
    let admins: BTreeSet<&str> = assigned
        .iter()
        .filter_map(|a| a.strip_prefix("u_"))
        .collect();
    for user in admins {
        let member: Option<(String, String)> = id_of(user).and_then(|user_id| {
            conn.query_row(
                r#"SELECT "role", "permissions" FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
                params![workspace_id, user_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
        });
        let admin = member.is_some_and(|(role, permissions)| {
            is_admin(
                &role,
                &serde_json::from_str(&permissions).unwrap_or(Value::Null),
            )
        });
        if !admin {
            return fail("Conversation assignees must be workspace administrators.");
        }
    }
    let teams: Vec<&str> = assigned
        .iter()
        .filter_map(|a| a.strip_prefix("t_"))
        .collect();
    let (ids, wanted) = ids_or_missing(&teams);
    let found = count_in(
        conn,
        r#"SELECT count(*) FROM "InboxTeam" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2))"#,
        workspace_id,
        &ids,
    )?;
    if found as usize != wanted {
        return fail("A selected inbox team is unavailable.");
    }
    serde_json::to_value(parsed)
        .map_err(|e| BusinessError::exception(e.to_string(), "systemError", 500))
}

/// `findBy({ id, workspaceId })`.
pub fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<AiAgent>> {
    find_one(
        conn,
        r#""id" = ?1 AND "workspaceId" = ?2"#,
        &[&id, &workspace_id],
    )
    .map_err(repo_err)
}

/// `findByIdForWorkspace`.
pub async fn find(db: &Db, workspace_id: i64, id: i64) -> Result<Option<AiAgent>> {
    tx::read(db, move |conn| find_in(conn, workspace_id, id)).await
}

/// `findDefault` (no cache).
pub async fn find_default(db: &Db, workspace_id: i64) -> Result<Option<AiAgent>> {
    tx::read(db, move |conn| {
        find_one(
            conn,
            r#""workspaceId" = ?1 AND "isDefault" = 1"#,
            &[&workspace_id],
        )
        .map_err(repo_err)
    })
    .await
}

/// `listAIAgents`: `sort` columns (then `id`), `name` contains (ILIKE).
pub async fn list(db: &Db, workspace_id: i64, input: ListAiAgents) -> Result<AiAgentPage> {
    tx::read(db, move |conn| {
        let p = get_pagination_with_defaults(input.paging);
        let pattern = input
            .name
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(like_contains);
        let filter = r#"WHERE "workspaceId" = ?1 AND (?2 IS NULL OR ilike("name", ?2))"#;
        let order = order_by_sql(
            &with_id_tie_break(&parse_order_by(AiAgent::COLUMNS, Some(&input.sort), None)),
            None,
        );
        let data = find_many(
            conn,
            &format!("{filter} {order} LIMIT ?3 OFFSET ?4"),
            &[&workspace_id, &pattern, &p.limit, &p.offset],
        )
        .map_err(repo_err)?;
        let total: i64 = conn
            .query_row(
                &format!(r#"SELECT count(*) FROM "AIAgent" {filter}"#),
                params![workspace_id, pattern],
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        Ok(AiAgentPage {
            data,
            page_count: (total + p.limit - 1) / p.limit,
        })
    })
    .await
}

fn clear_default(conn: &Connection, workspace_id: i64) -> Result<()> {
    conn.execute(
        r#"UPDATE "AIAgent" SET "isDefault" = 0 WHERE "workspaceId" = ?1"#,
        [workspace_id],
    )
    .map_err(repo_err)?;
    Ok(())
}

/// `createAndReturn(workspaceId, data, tx)`: the insert inside a caller's
/// transaction (no audit).
pub fn create_in(conn: &Connection, workspace_id: i64, data: &AiAgentInput) -> Result<AiAgent> {
    let action_rules = validate_action_rules(
        conn,
        workspace_id,
        data.action_rules.as_ref().unwrap_or(&json!([])),
    )?;
    let is_default = data.is_default.unwrap_or(false);
    if is_default {
        clear_default(conn, workspace_id)?;
    }
    let domains =
        normalize_web_search_domains(data.web_search_authorized_domains.as_deref().unwrap_or(&[]));
    let now = now_ms();
    let sql = format!(
        r#"INSERT INTO {} ("id", "createdAt", "updatedAt", "workspaceId", "name", "prompt", "actionPrompt", "actionRules",
             "messages", "isDefault", "isRichResponse", "tools", "webSearchAuthorizedDomains", "models", "temperature", "maxOutputTokens")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)"#,
        AiAgent::TABLE
    );
    returning::<AiAgent>(
        conn,
        &sql,
        &[
            &new_id(),
            &now,
            &workspace_id,
            &data.name.clone().unwrap_or_default(),
            &data.prompt,
            &data.action_prompt.clone().flatten(),
            &Json(action_rules),
            &Json(data.messages.clone().unwrap_or_else(|| json!([]))),
            &is_default,
            &data.is_rich_response.unwrap_or(false),
            &Json(json!(data.tools.clone().unwrap_or_default())),
            &Json(json!(domains)),
            &Json(data.models.clone().unwrap_or_else(|| json!([]))),
            &data.temperature.unwrap_or_default(),
            &data.max_output_tokens.unwrap_or_default(),
        ],
    )
    .map_err(repo_err)?
    .pop()
    .ok_or_else(|| BusinessError::not_found("AI agent not found"))
}

/// `createAndReturn` (no transaction given): insert, then audit.
pub async fn create(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    data: AiAgentInput,
) -> Result<AiAgent> {
    let created = tx::write(db, move |tx| create_in(tx, workspace_id, &data)).await?;
    audit
        .audit(
            "create",
            &format!("created a new AI Agent (#{})", created.id),
        )
        .await;
    Ok(created)
}

/// `updateAIAgent`: 404 `AI agent not found`; an empty patch returns the
/// stored row; audits by change group.
pub async fn update(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    id: i64,
    data: AiAgentInput,
) -> Result<AiAgent> {
    let (updated, detail) = tx::write(db, move |tx| {
        let before = find_in(tx, workspace_id, id)?
            .ok_or_else(|| BusinessError::not_found("AI agent not found"))?;
        if data.is_empty() {
            return Ok((before, None));
        }
        let action_rules = data
            .action_rules
            .as_ref()
            .map(|rules| validate_action_rules(tx, workspace_id, rules))
            .transpose()?;
        if data.is_default == Some(true) {
            clear_default(tx, workspace_id)?;
        }
        let domains = data
            .web_search_authorized_domains
            .as_deref()
            .map(|d| Json(json!(normalize_web_search_domains(d))));
        let sql = r#"UPDATE "AIAgent" SET
              "name" = coalesce(?1, "name"),
              "prompt" = coalesce(?2, "prompt"),
              "messages" = coalesce(?3, "messages"),
              "models" = coalesce(?4, "models"),
              "temperature" = coalesce(?5, "temperature"),
              "maxOutputTokens" = coalesce(?6, "maxOutputTokens"),
              "tools" = coalesce(?7, "tools"),
              "webSearchAuthorizedDomains" = coalesce(?8, "webSearchAuthorizedDomains"),
              "isDefault" = coalesce(?9, "isDefault"),
              "isRichResponse" = coalesce(?10, "isRichResponse"),
              "actionPrompt" = CASE WHEN ?11 THEN ?12 ELSE "actionPrompt" END,
              "actionRules" = coalesce(?13, "actionRules"),
              "updatedAt" = ?14
            WHERE "id" = ?15 AND "workspaceId" = ?16"#;
        tx.execute(
            sql,
            params![
                data.name,
                data.prompt,
                data.messages.clone().map(Json),
                data.models.clone().map(Json),
                data.temperature,
                data.max_output_tokens,
                data.tools.as_ref().map(|t| Json(json!(t))),
                domains,
                data.is_default,
                data.is_rich_response,
                data.action_prompt.is_some(),
                data.action_prompt.clone().flatten(),
                action_rules.map(Json),
                now_ms(),
                id,
                workspace_id,
            ],
        )
        .map_err(repo_err)?;
        let detail = update_audit_detail(&before, &data);
        let after = find_in(tx, workspace_id, id)?
            .ok_or_else(|| BusinessError::not_found("AI agent not found"))?;
        Ok((after, detail))
    })
    .await?;
    if let Some(detail) = detail {
        audit.audit("update", &detail).await;
    }
    Ok(updated)
}

/// `delete`: deletes the agents of `ids` in the workspace (a missing id is
/// no error); one audit per deleted agent.
pub async fn delete(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    ids: Vec<i64>,
) -> Result<Vec<i64>> {
    let deleted = tx::write(db, move |tx| delete_in(tx, workspace_id, &ids)).await?;
    for id in &deleted {
        audit
            .audit("delete", &format!("deleted an AI Agent (#{id})"))
            .await;
    }
    Ok(deleted)
}

/// `deleteInTx`: the deleted ids.
pub fn delete_in(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    for id in ids {
        let n = conn
            .execute(
                r#"DELETE FROM "AIAgent" WHERE "workspaceId" = ?1 AND "id" = ?2"#,
                params![workspace_id, id],
            )
            .map_err(repo_err)?;
        if n > 0 {
            out.push(*id);
        }
    }
    Ok(out)
}
