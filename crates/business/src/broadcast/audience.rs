//! `countAudience`, `listAudiencePreview`, `listForCalendar` and
//! `listTemplateDetails` of `broadcast/service.ts`.

use chatbotx_core::js::to_iso_string;
use chatbotx_db::Db;
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::broadcast::load_relations;
use chatbotx_db::repo::models::{Broadcast, ContactInbox};
use chatbotx_db::repo::partials::broadcast::{
    BroadcastAudienceRange, CappedCount, clamp_audience_count_to_range,
    clamp_capped_audience_count, requires_recent_interaction_window, resolve_audience_page_window,
};
use chatbotx_db::repo::queries::contact_filter::permission::{
    contact_filter_uses_field, prune_email_phone_filter_conditions,
};
use chatbotx_db::repo::queries::contact_filter::{
    FilterCriteria, build_contact_inbox_contact_filter_sql, contact_inbox_has_dm_sql,
};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::{self, client::count_query_capped, now_ms};
use chatbotx_db::rusqlite::types::Value as SqlValue;
use serde_json::{Value, json};

use crate::count_mode::{BoundedCount, CountMode, CountOutcome, ExactCount, count_by_mode};
use crate::errors::Result;
use crate::inbox::broadcast::{BroadcastInboxResolutionInput, resolve_broadcast_inbox_ids};
use crate::tx;

const DEFAULT_PREVIEW_PER_PAGE: i64 = 20;
const MAX_PREVIEW_PER_PAGE: i64 = 50;
const CALENDAR_LIST_LIMIT: i64 = 500;

/// `BroadcastAudienceInput`.
#[derive(Debug, Clone, Default)]
pub struct AudienceInput {
    pub workspace_id: i64,
    pub channels: Option<Vec<String>>,
    pub inbox_ids: Option<Vec<i64>>,
    pub integration_whatsapp_id: Option<i64>,
    pub integration_messenger_id: Option<i64>,
    pub contact_filter: Option<FilterCriteria>,
    /// `canViewEmailAndPhone !== false`.
    pub can_view_email_and_phone: bool,
    pub subaction: Option<String>,
    pub restrict_to_assigned_user_id: Option<i64>,
    pub audience_range: Option<BroadcastAudienceRange>,
}

/// How a query joins the DM conversation (`AudienceDmJoin`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum DmJoin {
    None,
    Inner,
    Left,
}

/// The DM is the `sourceId IS NULL` conversation (`buildDmConversationJoin`).
const DM_CONVERSATION_JOIN: &str = r#""Conversation"."contactId" = "ContactInbox"."contactId" AND "Conversation"."sourceId" IS NULL"#;

fn in_inboxes(inbox_ids: &[i64]) -> Sql {
    Sql::new(
        r#""ContactInbox"."inboxId" IN (SELECT value FROM json_each(?))"#,
        vec![SqlValue::Text(json!(inbox_ids).to_string())],
    )
}

/// `buildDmRequirement`: none for a filter on **Last interaction**.
fn dm_requirement(input: &AudienceInput, join: DmJoin) -> Option<Sql> {
    if contact_filter_uses_field(input.contact_filter.as_ref(), "lastInteraction") {
        return None;
    }
    match join {
        DmJoin::Inner => None,
        DmJoin::Left => Some(Sql::raw(r#""Conversation"."id" IS NOT NULL"#)),
        DmJoin::None => Some(contact_inbox_has_dm_sql()),
    }
}

/// `buildAudienceWhere`.
fn audience_where(inbox_ids: &[i64], input: &AudienceInput, join: DmJoin, now: i64) -> Sql {
    let filter = prune_email_phone_filter_conditions(
        input.contact_filter.as_ref(),
        input.can_view_email_and_phone,
    );
    Sql::and([
        Some(in_inboxes(inbox_ids)),
        filter.map(|f| {
            build_contact_inbox_contact_filter_sql(
                r#""ContactInbox"."contactId""#,
                input.workspace_id,
                &f,
                now,
            )
        }),
        dm_requirement(input, join),
        // `contactInboxInteractedWithin24hSQL`.
        requires_recent_interaction_window(input.subaction.as_deref()).then(|| {
            Sql::new(
                r#""ContactInbox"."lastIncomingMessageAt" >= ?"#,
                vec![SqlValue::Integer(now - 86_400_000)],
            )
        }),
    ])
}

/// `buildAssignedConversationWhere`.
fn assigned_where(input: &AudienceInput) -> Option<Sql> {
    input.restrict_to_assigned_user_id.map(|user| {
        Sql::new(
            r#""Conversation"."workspaceId" = ? AND "Conversation"."assignedUserId" = ?"#,
            vec![
                SqlValue::Integer(input.workspace_id),
                SqlValue::Integer(user),
            ],
        )
    })
}

async fn resolve_inbox_ids(db: &Db, input: &AudienceInput) -> Result<Vec<i64>> {
    let resolution = BroadcastInboxResolutionInput {
        workspace_id: input.workspace_id,
        channels: input.channels.clone(),
        inbox_ids: input.inbox_ids.clone(),
        integration_whatsapp_id: input.integration_whatsapp_id,
        integration_messenger_id: input.integration_messenger_id,
    };
    tx::read(db, move |conn| {
        Ok(resolve_broadcast_inbox_ids(conn, &resolution)?)
    })
    .await
}

/// `countAudience`: the reachable contact inboxes, clamped to the position window.
pub async fn count_audience(
    db: &Db,
    input: AudienceInput,
    mode: Option<CountMode>,
) -> Result<CountOutcome> {
    let inbox_ids = resolve_inbox_ids(db, &input).await?;
    if inbox_ids.is_empty() {
        return Ok(CountOutcome {
            total: 0,
            total_count_capped: false,
            count_cap: None,
        });
    }
    let now = now_ms();
    // Inner-joined to the DM, so the join itself is the DM requirement.
    let scoped = match assigned_where(&input) {
        Some(assigned) => {
            let mut sql = Sql::raw(format!(
                r#"SELECT 1 FROM "ContactInbox" INNER JOIN "Conversation" ON {DM_CONVERSATION_JOIN} WHERE "#
            ));
            sql.append(Sql::and([
                Some(audience_where(&inbox_ids, &input, DmJoin::Inner, now)),
                Some(assigned),
            ]));
            sql
        }
        None => {
            let mut sql = Sql::raw(r#"SELECT 1 FROM "ContactInbox" WHERE "#);
            sql.append(audience_where(&inbox_ids, &input, DmJoin::None, now));
            sql
        }
    };
    let exact_sql = scoped.clone();
    let exact: ExactCount = Box::new(move |tx| {
        let mut sql = Sql::raw("SELECT count(*) FROM (");
        sql.append(exact_sql);
        let sql = sql.closed(")");
        Ok(sql
            .query_map(tx, |row| row.get::<_, i64>(0))?
            .first()
            .copied()
            .unwrap_or(0))
    });
    let bounded: BoundedCount = Box::new(move |cap, tx| {
        let params: Vec<&dyn chatbotx_db::rusqlite::ToSql> = scoped
            .params
            .iter()
            .map(|p| p as &dyn chatbotx_db::rusqlite::ToSql)
            .collect();
        Ok(count_query_capped(tx, cap, &scoped.text, &params)?)
    });
    let outcome = count_by_mode(db, mode, exact, bounded).await?;
    let range = input.audience_range;
    if !outcome.total_count_capped {
        return Ok(CountOutcome {
            total: clamp_audience_count_to_range(outcome.total, range),
            ..outcome
        });
    }
    let clamped = clamp_capped_audience_count(
        CappedCount {
            total: outcome.total,
            capped: true,
        },
        range,
    );
    Ok(CountOutcome {
        total: clamped.total,
        total_count_capped: clamped.capped,
        ..outcome
    })
}

/// `listAudiencePreview`: one page of the audience, in `ContactInbox.id` order.
pub async fn list_audience_preview(
    db: &Db,
    input: AudienceInput,
    page: Option<i64>,
    per_page: Option<i64>,
) -> Result<Vec<Value>> {
    let inbox_ids = resolve_inbox_ids(db, &input).await?;
    if inbox_ids.is_empty() {
        return Ok(Vec::new());
    }
    let page = page.unwrap_or(1).max(1);
    let per_page = per_page
        .unwrap_or(DEFAULT_PREVIEW_PER_PAGE)
        .clamp(1, MAX_PREVIEW_PER_PAGE);
    let Some(window) = resolve_audience_page_window(page, per_page, input.audience_range) else {
        return Ok(Vec::new());
    };
    let mut sql = Sql::raw(format!(
        r#"SELECT c."id", "ContactInbox"."id", c."firstName", c."lastName", c."fullName", c."avatar", c."createdAt", "ContactInbox"."channel", "Conversation"."id"
           FROM "ContactInbox" INNER JOIN "Contact" AS c ON c."id" = "ContactInbox"."contactId"
           LEFT JOIN "Conversation" ON {DM_CONVERSATION_JOIN} WHERE "#
    ));
    let filter = Sql::and([
        Some(audience_where(&inbox_ids, &input, DmJoin::Left, now_ms())),
        Some(Sql::new(
            r#"c."workspaceId" = ?"#,
            vec![SqlValue::Integer(input.workspace_id)],
        )),
        assigned_where(&input),
    ]);
    sql.append(filter);
    sql.append(Sql::new(
        r#" ORDER BY "ContactInbox"."id" ASC LIMIT ? OFFSET ?"#,
        vec![
            SqlValue::Integer(window.limit),
            SqlValue::Integer(window.offset),
        ],
    ));
    tx::read(db, move |conn| {
        Ok(sql.query_map(conn, |row| {
            Ok(json!({
                "contactId": row.get::<_, i64>(0)?.to_string(),
                "contactInboxId": row.get::<_, i64>(1)?.to_string(),
                "firstName": row.get::<_, Option<String>>(2)?,
                "lastName": row.get::<_, Option<String>>(3)?,
                "fullName": row.get::<_, Option<String>>(4)?,
                "avatar": row.get::<_, Option<String>>(5)?,
                "createdAt": to_iso_string(row.get::<_, i64>(6)?),
                "channel": row.get::<_, String>(7)?,
                "conversationId": row.get::<_, Option<i64>>(8)?.map(|id| id.to_string()),
            }))
        })?)
    })
    .await
}

/// `listForCalendar` filters.
#[derive(Debug, Clone, Default)]
pub struct CalendarInput {
    pub workspace_id: i64,
    pub from: i64,
    pub to: i64,
    pub status: Option<String>,
    pub schedules_type: Option<String>,
    pub name: Option<String>,
}

/// `listForCalendar`: broadcasts scheduled in `[from, to]`; a paused or ended
/// series (no run, `cancelled`/`sent`) is not an event.
pub async fn list_for_calendar(db: &Db, input: CalendarInput) -> Result<Vec<Value>> {
    tx::read(db, move |conn| {
        let rows = repo::find_many::<Broadcast>(
            conn,
            r#"WHERE "workspaceId" = ?1 AND "schedulesAt" >= ?2 AND "schedulesAt" <= ?3
                 AND (?4 IS NULL OR "status" = ?4) AND (?5 IS NULL OR "schedulesType" = ?5)
                 AND (?6 IS NULL OR "name" LIKE '%' || ?6 || '%' ESCAPE '\')
                 AND "deletedAt" IS NULL
                 AND ("schedulesType" <> 'recurring' OR "occurrenceAt" IS NOT NULL OR "status" NOT IN ('cancelled', 'sent'))
               ORDER BY "schedulesAt" ASC LIMIT ?7"#,
            &[
                &input.workspace_id,
                &input.from,
                &input.to,
                &input.status,
                &input.schedules_type,
                &input.name.as_deref().filter(|n| !n.is_empty()).map(escape_like),
                &CALENDAR_LIST_LIMIT,
            ],
        )?;
        rows.into_iter()
            .map(|row| Ok(super::with_relations_json(&load_relations(conn, row, true)?, false, true)))
            .collect()
    })
    .await
}

/// `likeContains` escaping of `%`, `_` and `\`.
fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// `DEFAULT_CHUNK_SIZE` of `forEachAudienceChunk`.
pub const AUDIENCE_CHUNK_SIZE: i64 = 1000;

/// `forEachAudienceChunk` as a cursor: each [`AudienceWalk::next_chunk`]
/// is the next page of the reachable contact inboxes in `ContactInbox.id`
/// order, inside the position window; empty = done.
// ponytail: one keyset query with the contact filter in SQL; the original's
// separate filtered scan (5000-row pages matched by `findIdsMatchingFilter`)
// is a Postgres speed path with the same rows.
pub struct AudienceWalk {
    input: AudienceInput,
    inbox_ids: Vec<i64>,
    chunk_size: i64,
    last_id: Option<i64>,
    offset: i64,
    remaining: Option<i64>,
}

/// Starts an [`AudienceWalk`] (`chunk_size` `None` = [`AUDIENCE_CHUNK_SIZE`]).
pub async fn audience_walk(
    db: &Db,
    input: AudienceInput,
    chunk_size: Option<i64>,
) -> Result<AudienceWalk> {
    let inbox_ids = resolve_inbox_ids(db, &input).await?;
    let range = input.audience_range;
    Ok(AudienceWalk {
        inbox_ids,
        chunk_size: chunk_size.unwrap_or(AUDIENCE_CHUNK_SIZE).max(1),
        last_id: None,
        offset: range.map_or(0, |r| r.offset),
        remaining: range.and_then(|r| r.size),
        input,
    })
}

impl AudienceWalk {
    /// The next chunk; empty once the audience or the window is exhausted.
    pub async fn next_chunk(&mut self, db: &Db) -> Result<Vec<ContactInbox>> {
        let limit = match self.remaining {
            Some(remaining) => self.chunk_size.min(remaining),
            None => self.chunk_size,
        };
        if self.inbox_ids.is_empty() || limit <= 0 {
            return Ok(Vec::new());
        }
        let mut sql = Sql::raw(format!("{} WHERE ", ContactInbox::select_sql()));
        sql.append(Sql::and([
            Some(audience_where(
                &self.inbox_ids,
                &self.input,
                DmJoin::None,
                now_ms(),
            )),
            self.last_id
                .map(|id| Sql::new(r#""ContactInbox"."id" > ?"#, vec![SqlValue::Integer(id)])),
        ]));
        sql.append(Sql::new(
            r#" ORDER BY "ContactInbox"."id" ASC LIMIT ? OFFSET ?"#,
            vec![SqlValue::Integer(limit), SqlValue::Integer(self.offset)],
        ));
        let rows = tx::read(db, move |conn| {
            Ok(sql.query_map(conn, ContactInbox::from_row)?)
        })
        .await?;
        self.offset = 0;
        self.last_id = rows.last().map(|row| row.id).or(self.last_id);
        if let Some(remaining) = self.remaining.as_mut() {
            *remaining -= rows.len() as i64;
        }
        if (rows.len() as i64) < limit {
            self.remaining = Some(0);
        }
        Ok(rows)
    }
}
