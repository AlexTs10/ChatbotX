//! `packages/business/src/webhook/service.ts`: outgoing webhooks and their
//! event conditions. ponytail: no webhook cache (`updateWebhookCache`): the
//! worker matcher reads the rows; no `distributedLock` (one SQLite writer).
//! Every mutating call returns its audit detail; the caller writes it.

use chatbotx_db::Db;
use chatbotx_db::repo::models::{Condition, Webhook};
use chatbotx_db::repo::webhook::{self as repo, ListWebhooksInput, WebhookWithConditions};
use chatbotx_db::repo::{self as db_repo, Json, id_list, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::Value;

use crate::errors::{BusinessError, Result};
use crate::{folder, tx};

pub const MAX_WEBHOOKS_PER_WORKSPACE: i64 = 100;

/// `ConditionInput` (an absent `id` = a new condition).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConditionInput {
    pub id: Option<i64>,
    pub r#type: String,
    pub source_id: Option<String>,
    pub operator: Option<String>,
    pub value: Option<Value>,
}

fn sql(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Webhook not found")
}

fn count(conn: &Connection, workspace_id: i64) -> Result<i64> {
    conn.query_row(
        r#"SELECT COUNT(*) FROM "Webhook" WHERE "workspaceId" = ?1"#,
        [workspace_id],
        |r| r.get(0),
    )
    .map_err(sql)
}

fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Webhook>> {
    Ok(db_repo::find_one::<Webhook>(
        conn,
        r#""id" = ?1 AND "workspaceId" = ?2"#,
        &[&id, &workspace_id],
    )?)
}

fn insert_condition(conn: &Connection, webhook_id: i64, c: &ConditionInput) -> Result<()> {
    conn.execute(
        r#"INSERT INTO "Condition" ("id", "webhookId", "type", "sourceId", "operator", "value")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
        params![
            new_id(),
            webhook_id,
            c.r#type,
            c.source_id,
            c.operator,
            c.value.clone().map(Json)
        ],
    )
    .map_err(sql)?;
    Ok(())
}

/// `assertDeliverableUrl`: deliveries are server-side POSTs, so the target
/// must be a public address (422 `invalidRequestData`).
pub async fn assert_deliverable_url(url: &str) -> Result<()> {
    crate::net::ssrf_guard::assert_public_url(url, "Webhook URL")
        .await
        .map_err(|message| BusinessError::exception(message, "invalidRequestData", 422))
}

/// `webhookService.list`: one SQL page with the conditions joined in.
pub async fn list(
    db: &Db,
    input: ListWebhooksInput,
) -> Result<(Vec<WebhookWithConditions>, i64)> {
    let per_page = input.limit.max(1);
    tx::read(db, move |c| {
        let (rows, total) = repo::list_webhooks_paginated(c, &input)?;
        let ids: Vec<i64> = rows.iter().map(|w| w.id).collect();
        let conditions: Vec<Condition> = db_repo::find_many(
            c,
            r#"WHERE "webhookId" IN (SELECT value FROM json_each(?1)) ORDER BY "id""#,
            &[&id_list(&ids)],
        )?;
        let data = rows
            .into_iter()
            .map(|webhook| WebhookWithConditions {
                conditions: conditions
                    .iter()
                    .filter(|x| x.webhook_id == Some(webhook.id))
                    .cloned()
                    .collect(),
                webhook,
            })
            .collect();
        Ok((data, (total + per_page - 1) / per_page))
    })
    .await
}

/// `webhookService.create`: the builder form (empty `url`, set later).
pub async fn create(
    db: &Db,
    workspace_id: i64,
    name: String,
    folder_id: Option<i64>,
) -> Result<(Webhook, String)> {
    if tx::read(db, move |c| count(c, workspace_id)).await? >= MAX_WEBHOOKS_PER_WORKSPACE {
        let mut error = BusinessError::validation("_", "validation.maxItemsReached");
        if let BusinessError::Exception { data, .. } = &mut error {
            *data = Some(
                serde_json::json!({ "max": MAX_WEBHOOKS_PER_WORKSPACE, "feature": "webhooks" }),
            );
        }
        return Err(error);
    }
    if let Some(folder_id) = folder_id {
        folder::find_or_fail(db, workspace_id, folder_id, Some("webhook")).await?;
    }
    let created = tx::write(db, move |c| {
        let id = new_id();
        let now = now_ms();
        c.execute(
            r#"INSERT INTO "Webhook" ("id", "createdAt", "updatedAt", "name", "folderId", "url", "workspaceId")
               VALUES (?1, ?2, ?2, ?3, ?4, '', ?5)"#,
            params![id, now, name, folder_id, workspace_id],
        )
        .map_err(sql)?;
        find_in(c, workspace_id, id)?.ok_or_else(not_found)
    })
    .await?;
    let detail = format!("created a new webhook (#{})", created.id);
    Ok((created, detail))
}

/// `webhookService.register`: a webhook with its conditions (public API).
pub async fn register(
    db: &Db,
    workspace_id: i64,
    name: String,
    url: String,
    conditions: Vec<ConditionInput>,
) -> Result<(Webhook, String)> {
    assert_deliverable_url(&url).await?;
    let created = tx::write(db, move |c| {
        if count(c, workspace_id)? >= MAX_WEBHOOKS_PER_WORKSPACE {
            return Err(BusinessError::exception(
                format!(
                    "Workspace has reached the maximum of {MAX_WEBHOOKS_PER_WORKSPACE} webhooks"
                ),
                "webhookLimitReached",
                400,
            ));
        }
        let id = new_id();
        let now = now_ms();
        c.execute(
            r#"INSERT INTO "Webhook" ("id", "createdAt", "updatedAt", "name", "url", "workspaceId")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5)"#,
            params![id, now, name, url, workspace_id],
        )
        .map_err(sql)?;
        for condition in &conditions {
            insert_condition(c, id, condition)?;
        }
        find_in(c, workspace_id, id)?.ok_or_else(not_found)
    })
    .await?;
    let detail = format!("created a new webhook (#{})", created.id);
    Ok((created, detail))
}

/// `findWithConditionsOrFail`.
pub async fn find_with_conditions_or_fail(
    db: &Db,
    workspace_id: i64,
    id: i64,
) -> Result<WebhookWithConditions> {
    tx::read(db, move |c| {
        Ok(repo::find_webhook_with_conditions(c, Some(id), Some(workspace_id))?)
    })
    .await?
    .ok_or_else(not_found)
}

/// `unregister`: 404 when nothing was deleted.
pub async fn unregister(db: &Db, workspace_id: i64, id: i64) -> Result<String> {
    tx::write(db, move |c| {
        let n = c
            .execute(
                r#"DELETE FROM "Webhook" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![id, workspace_id],
            )
            .map_err(sql)?;
        if n == 0 {
            return Err(not_found());
        }
        Ok(format!("deleted webhook(s) (#{id})"))
    })
    .await
}

/// `deleteMany`: the audit detail when something was deleted.
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<Option<String>> {
    tx::write(db, move |c| {
        let mut stmt = c
            .prepare(
                r#"DELETE FROM "Webhook" WHERE "workspaceId" = ?1
                   AND "id" IN (SELECT value FROM json_each(?2)) RETURNING "id""#,
            )
            .map_err(sql)?;
        let deleted = stmt
            .query_map(params![workspace_id, id_list(&ids)], |r| r.get::<_, i64>(0))
            .map_err(sql)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sql)?;
        Ok((!deleted.is_empty()).then(|| {
            let list = deleted
                .iter()
                .map(|id| format!("#{id}"))
                .collect::<Vec<_>>()
                .join(", ");
            let s = if deleted.len() > 1 { "s" } else { "" };
            format!("deleted webhook{s} ({list})")
        }))
    })
    .await
}

/// `updateWithConditions`: sets the `url` and diffs the conditions
/// (delete the unsent, update every sent one, create the new). `None` = not
/// a webhook of the workspace.
pub async fn update_with_conditions(
    db: &Db,
    workspace_id: i64,
    id: i64,
    url: String,
    conditions: Vec<ConditionInput>,
) -> Result<Option<(Webhook, String)>> {
    assert_deliverable_url(&url).await?;
    tx::write(db, move |c| {
        if find_in(c, workspace_id, id)?.is_none() {
            return Ok(None);
        }
        let existing: Vec<Condition> =
            db_repo::find_many(c, r#"WHERE "webhookId" = ?1"#, &[&id])?;
        c.execute(
            r#"UPDATE "Webhook" SET "url" = ?3, "updatedAt" = ?4 WHERE "workspaceId" = ?1 AND "id" = ?2"#,
            params![workspace_id, id, url, now_ms()],
        )
        .map_err(sql)?;
        let submitted: Vec<i64> = conditions.iter().filter_map(|x| x.id).collect();
        for old in existing.iter().filter(|old| !submitted.contains(&old.id)) {
            c.execute(r#"DELETE FROM "Condition" WHERE "id" = ?1"#, [old.id])
                .map_err(sql)?;
        }
        for condition in &conditions {
            match condition.id {
                Some(cid) if existing.iter().any(|old| old.id == cid) => {
                    c.execute(
                        r#"UPDATE "Condition" SET "type" = ?2, "sourceId" = ?3, "operator" = ?4, "value" = ?5,
                           "updatedAt" = ?6 WHERE "id" = ?1"#,
                        params![
                            cid,
                            condition.r#type,
                            condition.source_id,
                            condition.operator,
                            condition.value.clone().map(Json),
                            now_ms()
                        ],
                    )
                    .map_err(sql)?;
                }
                Some(_) => {}
                None => insert_condition(c, id, condition)?,
            }
        }
        let webhook = find_in(c, workspace_id, id)?.ok_or_else(not_found)?;
        let detail = format!("updated a webhook (#{})", webhook.id);
        Ok(Some((webhook, detail)))
    })
    .await
}

/// `updateSettings`: the audit detail, `None` for a no-op patch.
pub async fn update_settings(
    db: &Db,
    workspace_id: i64,
    id: i64,
    name: Option<String>,
    active: Option<bool>,
) -> Result<Option<String>> {
    tx::write(db, move |c| {
        let webhook = find_in(c, workspace_id, id)?.ok_or_else(not_found)?;
        let name = name.filter(|n| *n != webhook.name);
        let active = active.filter(|a| *a != webhook.active);
        if name.is_none() && active.is_none() {
            return Ok(None);
        }
        c.execute(
            r#"UPDATE "Webhook" SET "name" = COALESCE(?2, "name"), "active" = COALESCE(?3, "active"),
               "updatedAt" = ?4 WHERE "id" = ?1"#,
            params![webhook.id, name, active, now_ms()],
        )
        .map_err(sql)?;
        Ok(Some(match (name.is_none(), active) {
            (true, Some(true)) => format!("enabled a webhook (#{})", webhook.id),
            (true, Some(false)) => format!("disabled a webhook (#{})", webhook.id),
            _ => format!("updated a webhook (#{})", webhook.id),
        }))
    })
    .await
}
