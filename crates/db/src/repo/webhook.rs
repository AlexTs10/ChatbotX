//! Port of `repositories/webhook/repository.ts` (P3.3). The re-exported
//! `listContactCustomFieldsForDateTimeSweep` lives in [`super::contact_custom_field`].

use rusqlite::Connection;

use super::models::{Condition, Webhook, Workspace};
use super::sql::Sql;
use super::{Model, Result};

/// `rootFolderId`.
pub const ROOT_FOLDER_ID: &str = "0";

/// `ActiveDateTimeWebhookRow`.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveDateTimeWebhook {
    pub id: i64,
    pub workspace_id: i64,
    pub workspace_timezone: Option<String>,
    /// Only the `dateTimeBasedTrigger` conditions.
    pub conditions: Vec<Condition>,
}

/// Result of [`list_active_date_time_webhooks`].
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveDateTimeWebhookPage {
    pub next_cursor: Option<i64>,
    pub webhooks: Vec<ActiveDateTimeWebhook>,
}

/// A webhook with its conditions.
#[derive(Debug, Clone, PartialEq)]
pub struct WebhookWithConditions {
    pub webhook: Webhook,
    pub conditions: Vec<Condition>,
}

/// Input of [`list_webhooks_paginated`]: `folder_id` `None` = no filter, `Some(None)` =
/// root (TS `null` / `"0"`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListWebhooksInput {
    pub workspace_id: i64,
    pub folder_id: Option<Option<i64>>,
    pub name: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

fn conditions_of(conn: &Connection, webhook_id: i64) -> Result<Vec<Condition>> {
    super::find_many(
        conn,
        "WHERE \"webhookId\" = ?1 ORDER BY \"id\"",
        &[&webhook_id],
    )
}

/// `listActiveDateTimeWebhooks`: one page of active webhooks (by id) that have a
/// date-time condition; the cursor is set when the page was full.
pub fn list_active_date_time_webhooks(
    conn: &Connection,
    cursor: Option<i64>,
    limit: i64,
) -> Result<ActiveDateTimeWebhookPage> {
    let active: Vec<Webhook> = match cursor {
        Some(c) => super::find_many(
            conn,
            "WHERE \"active\" = 1 AND \"id\" > ?1 ORDER BY \"id\" ASC LIMIT ?2",
            &[&c, &limit],
        )?,
        None => super::find_many(
            conn,
            "WHERE \"active\" = 1 ORDER BY \"id\" ASC LIMIT ?1",
            &[&limit],
        )?,
    };
    let mut webhooks = Vec::new();
    for w in &active {
        let conditions: Vec<Condition> = conditions_of(conn, w.id)?
            .into_iter()
            .filter(|c| c.r#type == "dateTimeBasedTrigger")
            .collect();
        if conditions.is_empty() {
            continue;
        }
        let workspace = super::find_one::<Workspace>(conn, "\"id\" = ?1", &[&w.workspace_id])?;
        webhooks.push(ActiveDateTimeWebhook {
            id: w.id,
            workspace_id: w.workspace_id,
            workspace_timezone: workspace.map(|ws| ws.timezone),
            conditions,
        });
    }
    let full = i64::try_from(active.len()).unwrap_or(i64::MAX) == limit;
    Ok(ActiveDateTimeWebhookPage {
        next_cursor: if full {
            active.last().map(|w| w.id)
        } else {
            None
        },
        webhooks,
    })
}

fn webhook_where(input: &ListWebhooksInput) -> Sql {
    Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", input.workspace_id)),
        input.folder_id.map(|f| match f {
            None => Sql::raw("\"folderId\" IS NULL"),
            Some(id) => Sql::bind("\"folderId\" = ", id),
        }),
        input
            .name
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| Sql::bind("\"name\" = ", n.to_owned())),
    ])
}

/// `listWebhooksPaginated` (no explicit order, as the TS).
pub fn list_webhooks_paginated(
    conn: &Connection,
    input: &ListWebhooksInput,
) -> Result<(Vec<Webhook>, i64)> {
    let mut q = Sql::raw(format!("{} WHERE ", Webhook::select_sql()));
    q.append(webhook_where(input));
    q.push(" LIMIT ");
    q.push_bind(input.limit);
    q.push(" OFFSET ");
    q.push_bind(input.offset);
    let rows = q.models::<Webhook>(conn)?;
    let mut c = Sql::raw("SELECT count(*) FROM \"Webhook\" WHERE ");
    c.append(webhook_where(input));
    let total = c
        .query_map(conn, |r| r.get::<_, i64>(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    Ok((rows, total))
}

/// `findWebhookWithConditions`: by id and/or workspace; `None` when neither is given.
pub fn find_webhook_with_conditions(
    conn: &Connection,
    id: Option<i64>,
    workspace_id: Option<i64>,
) -> Result<Option<WebhookWithConditions>> {
    if id.is_none() && workspace_id.is_none() {
        return Ok(None);
    }
    let mut q = Sql::raw(format!("{} WHERE ", Webhook::select_sql()));
    q.append(Sql::and([
        id.map(|i| Sql::bind("\"id\" = ", i)),
        workspace_id.map(|w| Sql::bind("\"workspaceId\" = ", w)),
    ]));
    q.push(" LIMIT 1");
    let Some(webhook) = q.model::<Webhook>(conn)? else {
        return Ok(None);
    };
    let conditions = conditions_of(conn, webhook.id)?;
    Ok(Some(WebhookWithConditions {
        webhook,
        conditions,
    }))
}
