//! Port of `repositories/workspace-deletion-record/repository.ts` (P3.3).

use rusqlite::{Connection, OptionalExtension};

use super::models::{Workspace, WorkspaceDeletionRecord};
use super::sql::Sql;
use super::{Json, Model, Result, new_id, now_ms};

/// `CreateInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct NewDeletionRecord {
    pub deletion_requested_at: Option<i64>,
    pub deletion_requested_by_email: Option<String>,
    pub deletion_requested_by_id: Option<i64>,
    pub details: Json,
    pub owner_email: Option<String>,
    pub owner_id: i64,
    pub scheduled_deletion_at: Option<i64>,
    pub tenant_id: i64,
    pub usage_snapshot: Json,
    pub workspace_id: i64,
    pub workspace_name: String,
}

/// `{ email, name, role, userId }` of a member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberSnapshot {
    pub email: String,
    pub name: Option<String>,
    pub role: String,
    pub user_id: i64,
}

/// `{ channel, id, name, status }` of an inbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxSnapshot {
    pub channel: String,
    pub id: i64,
    pub name: String,
    pub status: String,
}

/// `{ count, periodEnd, periodStart }` of a usage period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodCount {
    pub count: i64,
    pub period_end: i64,
    pub period_start: i64,
}

/// The `UserQuota` columns the snapshot reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaSnapshot {
    pub bot_messages_used: i64,
    pub mac_used: i64,
    pub monthly_bot_messages_used: i64,
    pub period_end: Option<i64>,
    pub period_start: Option<i64>,
    pub plan_name: Option<String>,
    pub plan_status: Option<String>,
}

/// Result of [`read_snapshot_sources`].
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotSources {
    pub bot_messages: Vec<PeriodCount>,
    pub contacts: i64,
    pub inboxes: Vec<InboxSnapshot>,
    pub mac: Vec<PeriodCount>,
    pub members: Vec<MemberSnapshot>,
    /// `(email, name, tenantId)`.
    pub owner: Option<(String, Option<String>, i64)>,
    pub quota: Option<QuotaSnapshot>,
    /// `(email, name)`.
    pub requested_by: Option<(String, Option<String>)>,
    pub workspace: Workspace,
}

fn periods(
    conn: &Connection,
    table: &str,
    count_col: &str,
    workspace_id: i64,
) -> Result<Vec<PeriodCount>> {
    let sql = format!(
        "SELECT \"{count_col}\", \"periodEnd\", \"periodStart\" FROM \"{table}\" WHERE \"workspaceId\" = ?1"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map([workspace_id], |r| {
            Ok(PeriodCount {
                count: r.get(0)?,
                period_end: r.get(1)?,
                period_start: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `readSnapshotSources`: `None` when the workspace does not exist.
pub fn read_snapshot_sources(
    conn: &Connection,
    owner_id: i64,
    workspace_id: i64,
) -> Result<Option<SnapshotSources>> {
    let Some(workspace) = super::find_one::<Workspace>(conn, "\"id\" = ?1", &[&workspace_id])?
    else {
        return Ok(None);
    };
    let owner = conn
        .prepare_cached("SELECT \"email\", \"name\", \"tenantId\" FROM \"User\" WHERE \"id\" = ?1")?
        .query_row([owner_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    let requested_by = match workspace.deletion_requested_by_id {
        Some(id) => conn
            .prepare_cached("SELECT \"email\", \"name\" FROM \"User\" WHERE \"id\" = ?1")?
            .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?,
        None => None,
    };
    let members = conn
        .prepare_cached(
            "SELECT u.\"email\", u.\"name\", m.\"role\", m.\"userId\" FROM \"WorkspaceMember\" m \
             INNER JOIN \"User\" u ON m.\"userId\" = u.\"id\" WHERE m.\"workspaceId\" = ?1",
        )?
        .query_map([workspace_id], |r| {
            Ok(MemberSnapshot {
                email: r.get(0)?,
                name: r.get(1)?,
                role: r.get(2)?,
                user_id: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let inboxes = conn
        .prepare_cached("SELECT \"channel\", \"id\", \"name\", \"status\" FROM \"Inbox\" WHERE \"workspaceId\" = ?1")?
        .query_map([workspace_id], |r| {
            Ok(InboxSnapshot {
                channel: r.get(0)?,
                id: r.get(1)?,
                name: r.get(2)?,
                status: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mac = periods(conn, "WorkspaceMac", "macCount", workspace_id)?;
    let bot_messages = periods(conn, "WorkspaceBotMessageUsage", "count", workspace_id)?;
    let contacts = super::client::count_where(
        conn,
        "Contact",
        Some("\"workspaceId\" = ?1"),
        &[&workspace_id],
    )?;
    let quota = conn
        .prepare_cached(
            "SELECT \"botMessagesUsed\", \"macUsed\", \"monthlyBotMessagesUsed\", \"periodEnd\", \"periodStart\", \
             \"planName\", \"planStatus\" FROM \"UserQuota\" WHERE \"userId\" = ?1",
        )?
        .query_row([owner_id], |r| {
            Ok(QuotaSnapshot {
                bot_messages_used: r.get(0)?,
                mac_used: r.get(1)?,
                monthly_bot_messages_used: r.get(2)?,
                period_end: r.get(3)?,
                period_start: r.get(4)?,
                plan_name: r.get(5)?,
                plan_status: r.get(6)?,
            })
        })
        .optional()?;
    Ok(Some(SnapshotSources {
        bot_messages,
        contacts,
        inboxes,
        mac,
        members,
        owner,
        quota,
        requested_by,
        workspace,
    }))
}

/// `insertIfAbsent`: `false` when the workspace already has a record.
pub fn insert_if_absent(conn: &Connection, v: &NewDeletionRecord) -> Result<bool> {
    let now = now_ms();
    let changed = conn.execute(
        "INSERT INTO \"WorkspaceDeletionRecord\" (\"id\", \"createdAt\", \"updatedAt\", \"workspaceId\", \"workspaceName\", \
         \"ownerId\", \"ownerEmail\", \"tenantId\", \"deletionRequestedById\", \"deletionRequestedByEmail\", \
         \"deletionRequestedAt\", \"scheduledDeletionAt\", \"usageSnapshot\", \"details\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) ON CONFLICT DO NOTHING",
        rusqlite::params![
            new_id(),
            now,
            v.workspace_id,
            v.workspace_name,
            v.owner_id,
            v.owner_email,
            v.tenant_id,
            v.deletion_requested_by_id,
            v.deletion_requested_by_email,
            v.deletion_requested_at,
            v.scheduled_deletion_at,
            v.usage_snapshot,
            v.details
        ],
    )?;
    Ok(changed > 0)
}

/// `markPurged`.
pub fn mark_purged(conn: &Connection, workspace_id: i64) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "UPDATE \"WorkspaceDeletionRecord\" SET \"purgedAt\" = ?2, \"updatedAt\" = ?2 WHERE \"workspaceId\" = ?1",
        rusqlite::params![workspace_id, now],
    )?;
    Ok(())
}

fn page(
    conn: &Connection,
    filter: Sql,
    limit: i64,
    offset: i64,
) -> Result<(Vec<WorkspaceDeletionRecord>, i64)> {
    let mut q = Sql::raw(format!("{} WHERE ", WorkspaceDeletionRecord::select_sql()));
    q.append(filter.clone());
    q.push(" ORDER BY \"createdAt\" DESC LIMIT ");
    q.push_bind(limit);
    q.push(" OFFSET ");
    q.push_bind(offset);
    let data = q.models(conn)?;
    let mut c = Sql::raw("SELECT count(*) FROM \"WorkspaceDeletionRecord\" WHERE ");
    c.append(filter);
    let total = c
        .query_map(conn, |r| r.get::<_, i64>(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    Ok((data, total))
}

/// `listForOwner`: newest first, with the total.
pub fn list_for_owner(
    conn: &Connection,
    owner_id: i64,
    limit: i64,
    offset: i64,
) -> Result<(Vec<WorkspaceDeletionRecord>, i64)> {
    page(conn, Sql::bind("\"ownerId\" = ", owner_id), limit, offset)
}

/// `listForAdmin`: optional tenant and `%search%` (workspace name or owner email).
pub fn list_for_admin(
    conn: &Connection,
    limit: i64,
    offset: i64,
    search: Option<&str>,
    tenant_id: Option<i64>,
) -> Result<(Vec<WorkspaceDeletionRecord>, i64)> {
    let filter = Sql::and([
        tenant_id.map(|t| Sql::bind("\"tenantId\" = ", t)),
        search.filter(|s| !s.is_empty()).map(|s| {
            let pattern = format!("%{s}%");
            Sql::or([
                Some(Sql::bind("ilike(\"workspaceName\", ", pattern.clone()).closed(")")),
                Some(Sql::bind("ilike(\"ownerEmail\", ", pattern).closed(")")),
            ])
        }),
    ]);
    page(conn, filter, limit, offset)
}
