//! Port of `repositories/broadcast/repository.ts`.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{Broadcast, BroadcastRecurrence, BroadcastTarget, Contact, ContactOnBroadcast};
use super::sql::Sql;
use super::utils::{
    PaginationInput, SortItem, get_pagination_with_defaults, like_contains, order_by_sql,
    parse_order_by,
};
use super::{Model, Result};

/// `BroadcastListInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BroadcastListInput {
    pub workspace_id: i64,
    pub name: Option<String>,
    pub status: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
    pub channel: Option<String>,
    pub scheduled_from: Option<i64>,
    pub scheduled_to: Option<i64>,
    pub include_runs: Option<bool>,
    pub exclude_templates: Option<bool>,
    pub parent_broadcast_id: Option<i64>,
    pub schedules_type: Option<String>,
}

/// `{ id, name }` of a related row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdName {
    pub id: i64,
    pub name: String,
}

/// One `targets` row with its inbox and flow names (`withBroadcastTargets`).
#[derive(Debug, Clone, PartialEq)]
pub struct TargetWithNames {
    pub target: BroadcastTarget,
    pub inbox: Option<IdName>,
    pub flow: Option<IdName>,
}

/// The `recurrence` summary columns of `withBroadcastRecurrence`.
#[derive(Debug, Clone, PartialEq)]
pub struct RecurrenceSummary {
    pub id: i64,
    pub rule: super::Json,
    pub next_run_at: Option<i64>,
    pub occurrence_count: i64,
    pub last_occurrence_at: Option<i64>,
    pub paused_at: Option<i64>,
    pub ended_at: Option<i64>,
}

/// A broadcast with the relations of `listWithRelations` / `findByIdOrName`.
#[derive(Debug, Clone, PartialEq)]
pub struct BroadcastWithRelations {
    pub broadcast: Broadcast,
    /// Only loaded by `listWithRelations`.
    pub flow: Option<IdName>,
    pub integration_whatsapp: Option<IdName>,
    pub integration_messenger: Option<IdName>,
    pub targets: Vec<TargetWithNames>,
    pub recurrence: Option<RecurrenceSummary>,
    pub parent: Option<IdName>,
}

/// One audience row with its contact (`listAudience`).
#[derive(Debug, Clone, PartialEq)]
pub struct AudienceRow {
    pub row: ContactOnBroadcast,
    pub contact: Contact,
}

/// `buildWhere` over the alias `b`.
fn build_where(input: &BroadcastListInput) -> Sql {
    let mut parts = vec![
        Some(Sql::bind("b.\"workspaceId\" = ", input.workspace_id)),
        input
            .name
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| Sql::bind("ilike(b.\"name\", ", like_contains(n)).closed(")")),
        input
            .status
            .clone()
            .map(|s| Sql::bind("b.\"status\" = ", s)),
        input
            .channel
            .clone()
            .map(|c| Sql::bind("b.\"channel\" = ", c)),
        input
            .scheduled_from
            .map(|f| Sql::bind("b.\"schedulesAt\" >= ", f)),
        input
            .scheduled_to
            .map(|t| Sql::bind("b.\"schedulesAt\" <= ", t)),
        input
            .schedules_type
            .clone()
            .map(|t| Sql::bind("b.\"schedulesType\" = ", t)),
        input
            .parent_broadcast_id
            .map(|p| Sql::bind("b.\"parentBroadcastId\" = ", p)),
    ];
    if input.include_runs == Some(false) {
        parts.push(Some(Sql::raw("b.\"occurrenceAt\" IS NULL")));
    }
    if input.exclude_templates == Some(true) {
        parts.push(Some(Sql::raw(
            "b.\"schedulesType\" <> 'recurring' OR b.\"occurrenceAt\" IS NOT NULL",
        )));
    }
    parts.push(Some(Sql::raw("b.\"deletedAt\" IS NULL")));
    Sql::and(parts)
}

fn id_name(conn: &Connection, table: &str, id: Option<i64>) -> Result<Option<IdName>> {
    let Some(id) = id else { return Ok(None) };
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT \"id\", \"name\" FROM \"{table}\" WHERE \"id\" = ?1"
    ))?;
    let mut rows = stmt.query([id])?;
    Ok(match rows.next()? {
        Some(row) => Some(IdName {
            id: row.get(0)?,
            name: row.get(1)?,
        }),
        None => None,
    })
}

fn load_relations(
    conn: &Connection,
    broadcast: Broadcast,
    with_integrations: bool,
) -> Result<BroadcastWithRelations> {
    let (flow, integration_whatsapp, integration_messenger) = if with_integrations {
        (
            id_name(conn, "Flow", broadcast.flow_id)?,
            id_name(
                conn,
                "IntegrationWhatsapp",
                broadcast.integration_whatsapp_id,
            )?,
            id_name(
                conn,
                "IntegrationMessenger",
                broadcast.integration_messenger_id,
            )?,
        )
    } else {
        (None, None, None)
    };
    let raw_targets: Vec<BroadcastTarget> = super::find_many(
        conn,
        "WHERE \"broadcastId\" = ?1 ORDER BY \"inboxId\"",
        &[&broadcast.id],
    )?;
    let mut targets = Vec::with_capacity(raw_targets.len());
    for target in raw_targets {
        let inbox = id_name(conn, "Inbox", Some(target.inbox_id))?;
        let flow = id_name(conn, "Flow", target.flow_id)?;
        targets.push(TargetWithNames {
            target,
            inbox,
            flow,
        });
    }
    let recurrence =
        super::find_one::<BroadcastRecurrence>(conn, "\"broadcastId\" = ?1", &[&broadcast.id])?
            .map(|r| RecurrenceSummary {
                id: r.id,
                rule: r.rule,
                next_run_at: r.next_run_at,
                occurrence_count: r.occurrence_count,
                last_occurrence_at: r.last_occurrence_at,
                paused_at: r.paused_at,
                ended_at: r.ended_at,
            });
    let parent = id_name(conn, "Broadcast", broadcast.parent_broadcast_id)?;
    Ok(BroadcastWithRelations {
        broadcast,
        flow,
        integration_whatsapp,
        integration_messenger,
        targets,
        recurrence,
        parent,
    })
}

/// `listWithRelations`: one page of the workspace's live (not soft-deleted) broadcasts with
/// flow, integrations, targets, recurrence summary and parent; requested sort, else newest
/// first.
pub fn list_with_relations(
    conn: &Connection,
    input: &BroadcastListInput,
) -> Result<Vec<BroadcastWithRelations>> {
    let pagination = get_pagination_with_defaults(PaginationInput {
        page: input.page,
        per_page: input.per_page,
    });
    let requested = parse_order_by(Broadcast::COLUMNS, input.sort.as_deref(), None);
    let order = if requested.is_empty() {
        "ORDER BY b.\"createdAt\" DESC".to_owned()
    } else {
        order_by_sql(&requested, Some("b"))
    };
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"Broadcast\" AS b WHERE ",
        Broadcast::columns_sql(Some("b"))
    ));
    q.append(build_where(input));
    q.push(&format!(" {order} LIMIT "));
    q.push_bind(pagination.limit);
    q.push(" OFFSET ");
    q.push_bind(pagination.offset);
    let rows: Vec<Broadcast> = q.models(conn)?;
    rows.into_iter()
        .map(|b| load_relations(conn, b, true))
        .collect()
}

/// `count`: the number of rows [`list_with_relations`] filters (all pages).
pub fn count(conn: &Connection, input: &BroadcastListInput) -> Result<i64> {
    let mut q = Sql::raw("SELECT count(*) FROM \"Broadcast\" AS b WHERE ");
    q.append(build_where(input));
    Ok(q.query_map(conn, |row| row.get(0))?
        .into_iter()
        .next()
        .unwrap_or(0))
}

/// `countActive`: live broadcasts of the workspace and channel in `statuses` (except
/// `exclude_id`).
pub fn count_active(
    conn: &Connection,
    workspace_id: i64,
    channel: &str,
    statuses: &[&str],
    exclude_id: Option<i64>,
) -> Result<i64> {
    let mut q = Sql::new(
        "SELECT count(*) FROM \"Broadcast\" WHERE \"workspaceId\" = ? AND \"channel\" = ? AND \"status\" IN (SELECT value FROM json_each(?)) AND \"deletedAt\" IS NULL",
        vec![
            Value::Integer(workspace_id),
            Value::Text(channel.to_owned()),
            Value::Text(super::text_list(statuses)),
        ],
    );
    if let Some(id) = exclude_id {
        q.push(" AND \"id\" <> ");
        q.push_bind(id);
    }
    Ok(q.query_map(conn, |row| row.get(0))?
        .into_iter()
        .next()
        .unwrap_or(0))
}

/// `listAudience`: audience rows of the broadcast with their contact.
pub fn list_audience(
    conn: &Connection,
    broadcast_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<AudienceRow>> {
    let sql = format!(
        "SELECT {}, {} FROM \"ContactOnBroadcast\" AS cb INNER JOIN \"Contact\" AS c ON c.\"id\" = cb.\"contactId\" \
         WHERE cb.\"broadcastId\" = ?1 ORDER BY cb.\"contactId\" LIMIT ?2 OFFSET ?3",
        ContactOnBroadcast::columns_sql(Some("cb")),
        Contact::columns_sql(Some("c"))
    );
    let n = ContactOnBroadcast::COLUMNS.len();
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params![broadcast_id, limit, offset], |row| {
            Ok(AudienceRow {
                row: ContactOnBroadcast::from_row(row)?,
                contact: Contact::from_row_at(row, n)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `countAudience`.
pub fn count_audience(conn: &Connection, broadcast_id: i64) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM \"ContactOnBroadcast\" WHERE \"broadcastId\" = ?1",
        [broadcast_id],
        |row| row.get(0),
    )?)
}

/// `findByIdOrName`: by id when `id_or_name` is all digits, else by name; live rows of the
/// workspace, with targets, recurrence summary and parent.
pub fn find_by_id_or_name(
    conn: &Connection,
    workspace_id: i64,
    id_or_name: &str,
) -> Result<Option<BroadcastWithRelations>> {
    let numeric = !id_or_name.is_empty() && id_or_name.bytes().all(|b| b.is_ascii_digit());
    let found: Option<Broadcast> = if numeric {
        let Ok(id) = id_or_name.parse::<i64>() else {
            return Ok(None);
        };
        super::find_one(
            conn,
            "\"id\" = ?1 AND \"workspaceId\" = ?2 AND \"deletedAt\" IS NULL",
            &[&id, &workspace_id],
        )?
    } else {
        super::find_one(
            conn,
            "\"name\" = ?1 AND \"workspaceId\" = ?2 AND \"deletedAt\" IS NULL",
            &[&id_or_name, &workspace_id],
        )?
    };
    found.map(|b| load_relations(conn, b, false)).transpose()
}
