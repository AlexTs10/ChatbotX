//! `packages/business/src/trigger/service.ts` (Q.D5): the CRUD part and the
//! datetime-sweep execution surface (`listActiveWithConditionsPage`,
//! `listExecutedPairs`, `recordExecution`, `purgeExecutionsOlderThan`). The
//! run limiter, contact history and daily stats live in the trigger queue
//! (`chatbotx_worker::trigger`, Q.W2).
//! No template locks / delete guards (P3.6-D1 rule). The trigger condition
//! cache is the 5 s RAM cache of `events::cache` (no explicit refresh).

use chatbotx_db::Db;
use chatbotx_db::repo::models::{Condition, Trigger};
use chatbotx_db::repo::trigger::{
    ListTriggersInput, TriggerWithConditions, find_with_conditions, list_paginated_with_conditions,
};
use chatbotx_db::repo::{self, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use crate::automation_action::{find_invalid_action_references, indexes_of_new_actions};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `MAX_TRIGGERS_PER_WORKSPACE`.
pub const MAX_TRIGGERS_PER_WORKSPACE: i64 = 50;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn load(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<TriggerWithConditions>> {
    Ok(find_with_conditions(conn, Some(id), Some(workspace_id))?)
}

/// `create`: at most 50 triggers per workspace; a `folderId` must be a
/// trigger folder of the workspace. `actions` starts empty.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    name: String,
    folder_id: Option<i64>,
) -> Result<Trigger> {
    tx::write(db, move |tx| {
        let count: i64 = tx
            .query_row(
                r#"SELECT count(*) FROM "Trigger" WHERE "workspaceId" = ?1"#,
                [workspace_id],
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        if count >= MAX_TRIGGERS_PER_WORKSPACE {
            return Err(BusinessError::Exception {
                message: "validation.maxItemsReached".to_string(),
                code: "validation".to_string(),
                status: 422,
                field: Some("_".to_string()),
                data: Some(json!({ "max": MAX_TRIGGERS_PER_WORKSPACE, "feature": "triggers" })),
            });
        }
        if let Some(folder_id) = folder_id {
            tx.query_row(
                r#"SELECT 1 FROM "Folder" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "folderType" = 'trigger'"#,
                params![folder_id, workspace_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(repo_err)?
            .ok_or_else(|| BusinessError::not_found("Folder not found"))?;
        }
        let id = new_id();
        let now = now_ms();
        tx.execute(
            r#"INSERT INTO "Trigger" ("id", "createdAt", "updatedAt", "name", "folderId", "actions", "workspaceId")
               VALUES (?1, ?2, ?2, ?3, ?4, '[]', ?5)"#,
            params![id, now, name, folder_id, workspace_id],
        )
        .map_err(repo_err)?;
        repo::find_one::<Trigger>(tx, r#""id" = ?1"#, &[&id])?
            .ok_or_else(|| BusinessError::not_found("Trigger not found"))
    })
    .await
}

/// `list`: newest first, with conditions; `pageCount = ceil(total / perPage)`.
pub async fn list(
    db: &Db,
    workspace_id: i64,
    folder_id: Option<Option<i64>>,
    name: Option<String>,
    page: i64,
    per_page: i64,
) -> Result<(Vec<TriggerWithConditions>, i64)> {
    tx::read(db, move |conn| {
        let out = list_paginated_with_conditions(
            conn,
            &ListTriggersInput {
                workspace_id,
                folder_id,
                name,
                limit: per_page,
                offset: (page - 1) * per_page,
            },
        )?;
        let pages = if per_page > 0 {
            (out.total + per_page - 1) / per_page
        } else {
            0
        };
        Ok((out.rows, pages))
    })
    .await
}

/// `findWithConditions` in the workspace.
pub async fn find(db: &Db, workspace_id: i64, id: i64) -> Result<Option<TriggerWithConditions>> {
    tx::read(db, move |conn| load(conn, workspace_id, id)).await
}

/// `deleteMany`: the ids actually deleted.
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<Vec<i64>> {
    tx::write(db, move |tx| {
        let mut deleted = Vec::new();
        for id in ids {
            if tx
                .execute(
                    r#"DELETE FROM "Trigger" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![id, workspace_id],
                )
                .map_err(repo_err)?
                > 0
            {
                deleted.push(id);
            }
        }
        Ok(deleted)
    })
    .await
}

/// The audit detail of `deleteMany` (`None` when nothing was deleted).
pub fn delete_audit_detail(deleted: &[i64]) -> Option<String> {
    if deleted.is_empty() {
        return None;
    }
    let ids: Vec<String> = deleted.iter().map(|id| format!("#{id}")).collect();
    let plural = if deleted.len() > 1 { "s" } else { "" };
    Some(format!("deleted trigger{plural} ({})", ids.join(", ")))
}

/// One submitted condition (`ConditionInput`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConditionInput {
    pub id: Option<String>,
    pub r#type: String,
    pub source_id: Option<String>,
    pub operator: Option<String>,
    pub value: Option<Value>,
}

impl ConditionInput {
    /// `toConditionColumnsShared`.
    fn columns(&self) -> (String, Option<String>, Option<String>, Value) {
        (
            self.r#type.clone(),
            self.source_id.clone(),
            self.operator.clone(),
            self.value.clone().unwrap_or(Value::Null),
        )
    }
}

/// `updateWithConditions`: replaces `actions` and diffs the conditions
/// (delete / update / create). `None` when the trigger is not in the
/// workspace. The bool is `hasRealChange` (audit only then). A new action
/// that names a missing record fails at `actions.<index>.<field>`.
pub async fn update_with_conditions(
    db: &Db,
    workspace_id: i64,
    id: i64,
    actions: Vec<Value>,
    conditions: Vec<ConditionInput>,
) -> Result<Option<(TriggerWithConditions, bool)>> {
    tx::write(db, move |tx| {
        let Some(existing) = load(tx, workspace_id, id)? else {
            return Ok(None);
        };
        let stored = existing.trigger.actions.0.as_array().cloned().unwrap_or_default();
        let fresh = indexes_of_new_actions(&actions, &stored);
        if !fresh.is_empty() {
            let problems = find_invalid_action_references(tx, workspace_id, &actions, Some(&fresh), true)?;
            if let Some((index, field)) = problems.first() {
                let field = format!("actions.{index}.{field}");
                return Err(BusinessError::Exception {
                    message: "validation.trigger.actionReferenceNotFound".to_string(),
                    code: "validation".to_string(),
                    status: 422,
                    field: Some(field.clone()),
                    data: Some(json!({ "field": field })),
                });
            }
        }
        let submitted: Vec<&str> = conditions.iter().filter_map(|c| c.id.as_deref()).collect();
        let to_delete: Vec<i64> = existing
            .conditions
            .iter()
            .filter(|c| !submitted.contains(&c.id.to_string().as_str()))
            .map(|c| c.id)
            .collect();
        let mut changed_updates = 0;
        for condition in &conditions {
            let Some(cid) = condition.id.as_deref() else { continue };
            let Some(row) = existing.conditions.iter().find(|c| c.id.to_string() == cid) else {
                continue;
            };
            let (kind, source, operator, value) = condition.columns();
            let same = kind == row.r#type
                && source == row.source_id
                && operator == row.operator
                && value == row.value.as_ref().map(|v| v.0.clone()).unwrap_or(Value::Null);
            if same {
                continue;
            }
            tx.execute(
                r#"UPDATE "Condition" SET "type" = ?1, "sourceId" = ?2, "operator" = ?3, "value" = ?4, "updatedAt" = ?5 WHERE "id" = ?6"#,
                params![kind, source, operator, json_or_null(&value), now_ms(), row.id],
            )
            .map_err(repo_err)?;
            changed_updates += 1;
        }
        let actions_changed = Value::Array(actions.clone()) != Value::Array(stored);
        if actions_changed {
            tx.execute(
                r#"UPDATE "Trigger" SET "actions" = ?1, "updatedAt" = ?2 WHERE "id" = ?3 AND "workspaceId" = ?4"#,
                params![Value::Array(actions).to_string(), now_ms(), id, workspace_id],
            )
            .map_err(repo_err)?;
        }
        for cid in &to_delete {
            tx.execute(r#"DELETE FROM "Condition" WHERE "id" = ?1"#, [cid])
                .map_err(repo_err)?;
        }
        let mut created = 0;
        for condition in conditions.iter().filter(|c| c.id.is_none()) {
            let (kind, source, operator, value) = condition.columns();
            let now = now_ms();
            tx.execute(
                r#"INSERT INTO "Condition" ("id", "createdAt", "updatedAt", "triggerId", "type", "sourceId", "operator", "value")
                   VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7)"#,
                params![new_id(), now, id, kind, source, operator, json_or_null(&value)],
            )
            .map_err(repo_err)?;
            created += 1;
        }
        let real = actions_changed || !to_delete.is_empty() || changed_updates > 0 || created > 0;
        Ok(load(tx, workspace_id, id)?.map(|t| (t, real)))
    })
    .await
}

/// A JSON column value (`NULL` for JSON `null`).
fn json_or_null(value: &Value) -> Option<String> {
    (!value.is_null()).then(|| value.to_string())
}

/// `updateSettings`: diff `name` / `active` against the row; the audit
/// detail is `enabled` / `disabled` when only `active` changed, else
/// `updated`; `None` when nothing changed. 404 "Trigger not found".
pub async fn update_settings(
    db: &Db,
    workspace_id: i64,
    id: i64,
    name: Option<String>,
    active: Option<bool>,
) -> Result<(TriggerWithConditions, Option<String>)> {
    tx::write(db, move |tx| {
        let current = load(tx, workspace_id, id)?.ok_or_else(|| BusinessError::not_found("Trigger not found"))?;
        let name = name.filter(|n| *n != current.trigger.name);
        let active = active.filter(|a| *a != current.trigger.active);
        if name.is_none() && active.is_none() {
            return Ok((current, None));
        }
        let updated = tx
            .execute(
                r#"UPDATE "Trigger" SET "name" = coalesce(?1, "name"), "active" = coalesce(?2, "active"), "updatedAt" = ?3 WHERE "id" = ?4"#,
                params![name, active, now_ms(), id],
            )
            .map_err(repo_err)?;
        let detail = (updated > 0).then(|| match (&name, active) {
            (None, Some(true)) => format!("enabled a trigger (#{id})"),
            (None, Some(false)) => format!("disabled a trigger (#{id})"),
            _ => format!("updated a trigger (#{id})"),
        });
        let row = load(tx, workspace_id, id)?.unwrap_or(current);
        Ok((row, detail))
    })
    .await
}

/// A trigger with its conditions as the public `triggerResource`.
pub fn trigger_json(found: &TriggerWithConditions) -> Value {
    let mut out = found.trigger.to_json();
    out["conditions"] = found
        .conditions
        .iter()
        .map(|c| {
            json!({
                "id": c.id.to_string(),
                "type": c.r#type,
                "sourceId": c.source_id,
                "operator": c.operator,
                "value": c.value.as_ref().map(|v| v.0.clone()).unwrap_or(Value::Null),
            })
        })
        .collect();
    out
}

/// One row of [`list_active_with_conditions_page`]
/// (`ActiveTriggerWithConditionsPageRow`): the trigger, its conditions and
/// the workspace timezone.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveTriggerPageRow {
    pub trigger: TriggerWithConditions,
    pub timezone: String,
}

/// `listActiveWithConditionsPage`: every active trigger, keyset-paginated
/// by id; `next_cursor` only when the page is full.
pub async fn list_active_with_conditions_page(
    db: &Db,
    cursor: Option<i64>,
    limit: i64,
) -> Result<(Vec<ActiveTriggerPageRow>, Option<i64>)> {
    tx::read(db, move |conn| {
        let triggers: Vec<Trigger> = repo::find_many(
            conn,
            r#"WHERE "active" = 1 AND (?1 IS NULL OR "id" > ?1) ORDER BY "id" LIMIT ?2"#,
            &[&cursor, &limit],
        )?;
        let mut rows = Vec::with_capacity(triggers.len());
        for trigger in triggers {
            let conditions: Vec<Condition> = repo::find_many(
                conn,
                r#"WHERE "triggerId" = ?1 ORDER BY "id""#,
                &[&trigger.id],
            )?;
            let timezone: String = conn
                .query_row(
                    r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
                    [trigger.workspace_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(repo_err)?
                .filter(|tz: &String| !tz.is_empty())
                .unwrap_or_else(|| "UTC".to_string());
            rows.push(ActiveTriggerPageRow {
                trigger: TriggerWithConditions {
                    trigger,
                    conditions,
                },
                timezone,
            });
        }
        let next_cursor = (rows.len() as i64 == limit)
            .then(|| rows.last().map(|r| r.trigger.trigger.id))
            .flatten();
        Ok((rows, next_cursor))
    })
    .await
}

/// `listExecutedPairs`: the stored `(triggerId, contactId)` executions;
/// no query when either list is empty.
pub async fn list_executed_pairs(
    db: &Db,
    trigger_ids: Vec<i64>,
    contact_ids: Vec<i64>,
) -> Result<Vec<(i64, i64)>> {
    if trigger_ids.is_empty() || contact_ids.is_empty() {
        return Ok(Vec::new());
    }
    tx::read(db, move |conn| {
        let triggers = serde_json::to_string(&trigger_ids).unwrap_or_default();
        let contacts = serde_json::to_string(&contact_ids).unwrap_or_default();
        let mut stmt = conn
            .prepare_cached(
                r#"SELECT "triggerId", "contactId" FROM "TriggerExecution"
                   WHERE "triggerId" IN (SELECT value FROM json_each(?1))
                     AND "contactId" IN (SELECT value FROM json_each(?2))"#,
            )
            .map_err(repo_err)?;
        let pairs = stmt
            .query_map(params![triggers, contacts], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok(pairs)
    })
    .await
}

/// `recordExecution`: one datetime execution row (insert, conflict ignored).
pub async fn record_execution(
    db: &Db,
    trigger_id: i64,
    contact_id: i64,
    workspace_id: i64,
) -> Result<()> {
    tx::write(db, move |tx| {
        let now = now_ms();
        tx.execute(
            r#"INSERT INTO "TriggerExecution" ("id", "triggerId", "contactId", "workspaceId", "createdAt", "executedAt")
               VALUES (?1, ?2, ?3, ?4, ?5, ?5) ON CONFLICT DO NOTHING"#,
            params![new_id(), trigger_id, contact_id, workspace_id, now],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await
}

/// `purgeExecutionsOlderThan`: the number of rows deleted.
pub async fn purge_executions_older_than(db: &Db, cutoff_ms: i64) -> Result<usize> {
    tx::write(db, move |tx| {
        tx.execute(
            r#"DELETE FROM "TriggerExecution" WHERE "executedAt" < ?1"#,
            [cutoff_ms],
        )
        .map_err(repo_err)
    })
    .await
}
