//! `commentAutomationAnalyticsService` reads (`commentAutomationStatsRepository`,
//! `commentAutomationMissRepository.getMissContacts`) and the public
//! contacts drill-down (`listCommentAutomationContacts`).

use chatbotx_core::js::to_iso_string;
use chatbotx_db::repo;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params_from_iter};

use super::{Granularity, TimeRange};

/// `commentAutomationEventType` (the counter a contacts page lists).
pub const EVENT_TYPES: &[&str] = &[
    "message:sent",
    "message:delivered",
    "message:seen",
    "message:failed",
    "flow:clicked",
    "comment:missed",
];

/// The automation of the workspace: `(type, counter of event_type)`, the
/// counter being `commentAutomationStatCounters[event_type]` (`None` for a
/// type without one).
pub fn find(
    conn: &Connection,
    workspace_id: i64,
    automation_id: i64,
    event_type: &str,
) -> repo::Result<Option<(String, i64)>> {
    let counter = match event_type {
        "message:sent" => "sentCount",
        "message:delivered" => "deliveredCount",
        "message:seen" => "seenCount",
        "flow:clicked" => "clickedCount",
        "message:failed" => "failedCount",
        "comment:missed" => "missedCount",
        _ => "0",
    };
    let counter = if counter == "0" {
        "0".to_string()
    } else {
        format!(r#""{counter}""#)
    };
    Ok(conn
        .query_row(
            &format!(r#"SELECT "type", {counter} FROM "CommentAutomation" WHERE "id" = ?1 AND "workspaceId" = ?2"#),
            [automation_id, workspace_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

fn exists(conn: &Connection, workspace_id: i64, automation_id: i64) -> repo::Result<bool> {
    Ok(find(conn, workspace_id, automation_id, "")?.is_some())
}

/// `getReplyStatsByDateRange`: sent replies per local day (monthly past 60
/// days, keyed `YYYY-MM-01`), zero-filled. Empty for an unknown or foreign
/// automation.
pub fn replies_by_date(
    conn: &Connection,
    range: &TimeRange,
    automation_id: i64,
) -> repo::Result<Vec<(String, i64)>> {
    let Some((from, to)) = range.bounds() else {
        return Ok(Vec::new());
    };
    if !exists(conn, range.workspace_id, automation_id)? {
        return Ok(Vec::new());
    }
    let granularity = range.granularity();
    let key = granularity.key_sql(r#""occurredAt""#, range.tz);
    let mut stmt = conn.prepare(&format!(
        r#"SELECT {key}, COUNT(*) FROM "CommentAutomationEvent"
           WHERE "workspaceId" = ?1 AND "automationId" = ?2 AND "status" = 'sent'
             AND "occurredAt" >= ?3 AND "occurredAt" <= ?4
           GROUP BY 1"#
    ))?;
    let counts: std::collections::HashMap<String, i64> = stmt
        .query_map([range.workspace_id, automation_id, from, to], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<Result<_, _>>()?;
    Ok(granularity
        .series(range)
        .into_iter()
        .map(|bucket| {
            let count = counts.get(&bucket.key).copied().unwrap_or(0);
            let day = match granularity {
                Granularity::Month => format!("{}-01", bucket.key),
                Granularity::Day => bucket.key,
            };
            (day, count)
        })
        .collect())
}

/// One page of a list read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub data: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub page_count: i64,
}

impl<T> Page<T> {
    fn empty(page: i64) -> Page<T> {
        Page {
            data: Vec::new(),
            total: 0,
            page,
            page_count: 0,
        }
    }
}

fn page_count(total: i64, per_page: i64) -> i64 {
    (total.max(0) as u64).div_ceil(per_page.max(1) as u64) as i64
}

/// The paged list inputs (`page` from 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListInput {
    pub workspace_id: i64,
    pub automation_id: i64,
    pub from: i64,
    pub to: i64,
    pub page: i64,
    pub per_page: i64,
    /// `ILIKE %keyword%` (the keyword's own `%` / `_` stay wildcards).
    pub keyword: Option<String>,
}

impl ListInput {
    fn scope(&self, condition: &str, keyword_columns: &[&str]) -> (String, Vec<Sql>) {
        let mut binds = vec![
            Sql::Integer(self.workspace_id),
            Sql::Integer(self.automation_id),
            Sql::Integer(self.from),
            Sql::Integer(self.to),
        ];
        let mut sql = format!(
            r#"FROM "CommentAutomationEvent" WHERE "workspaceId" = ? AND "automationId" = ?
               AND "occurredAt" >= ? AND "occurredAt" <= ? AND {condition}"#
        );
        if let Some(keyword) = self.keyword.as_deref().filter(|k| !k.is_empty()) {
            let any: Vec<String> = keyword_columns
                .iter()
                .map(|column| format!(r#"ilike("{column}", ?)"#))
                .collect();
            sql.push_str(&format!(" AND ({})", any.join(" OR ")));
            for _ in keyword_columns {
                binds.push(Sql::Text(format!("%{keyword}%")));
            }
        }
        (sql, binds)
    }
}

/// `{ text, total }` of a text list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextTotal {
    pub text: String,
    pub total: i64,
}

fn text_totals(
    conn: &Connection,
    input: &ListInput,
    user_comments: bool,
) -> repo::Result<Page<TextTotal>> {
    if !exists(conn, input.workspace_id, input.automation_id)? {
        return Ok(Page::empty(input.page));
    }
    // Customer comments count distinct comments (a comment with a public and
    // a private reply is two rows); bot replies count sent rows.
    let (column, condition, measure) = if user_comments {
        (
            "commentText",
            r#""commentText" IS NOT NULL"#,
            r#"COUNT(DISTINCT "commentId")"#,
        )
    } else {
        (
            "replyText",
            r#""status" = 'sent' AND "replyText" IS NOT NULL"#,
            "COUNT(*)",
        )
    };
    let (scope, mut binds) = input.scope(condition, &[column]);
    let total: i64 = conn.query_row(
        &format!(r#"SELECT COUNT(DISTINCT "{column}") {scope}"#),
        params_from_iter(binds.iter()),
        |row| row.get(0),
    )?;
    binds.extend([
        Sql::Integer(input.per_page),
        Sql::Integer((input.page - 1) * input.per_page),
    ]);
    let mut stmt = conn.prepare(&format!(
        r#"SELECT "{column}", {measure} AS total {scope} GROUP BY 1 ORDER BY total DESC, 1 ASC LIMIT ? OFFSET ?"#
    ))?;
    let data = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            Ok(TextTotal {
                text: row.get(0)?,
                total: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Page {
        data,
        total,
        page: input.page,
        page_count: page_count(total, input.per_page),
    })
}

/// `listUserComments`: distinct customer comments, most frequent first.
pub fn user_comments(conn: &Connection, input: &ListInput) -> repo::Result<Page<TextTotal>> {
    text_totals(conn, input, true)
}

/// `listBotReplies`: distinct sent reply texts, most frequent first.
pub fn bot_replies(conn: &Connection, input: &ListInput) -> repo::Result<Page<TextTotal>> {
    text_totals(conn, input, false)
}

/// One `CommentAutomationErrorRow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorRow {
    pub id: i64,
    pub reply_channel: String,
    pub reply_type: String,
    pub error_detail: Option<String>,
    pub http_code: Option<String>,
    pub comment_text: Option<String>,
    /// `(firstName, lastName, avatar)` of a contact that still exists.
    pub contact: Option<(Option<String>, Option<String>, Option<String>)>,
    pub occurred_at: String,
}

/// `listErrors`: failed replies, newest first, with the contact's name.
pub fn errors(conn: &Connection, input: &ListInput) -> repo::Result<Page<ErrorRow>> {
    if !exists(conn, input.workspace_id, input.automation_id)? {
        return Ok(Page::empty(input.page));
    }
    let (scope, mut binds) = input.scope(r#""status" = 'failed'"#, &["errorDetail", "commentText"]);
    let total: i64 = conn.query_row(
        &format!("SELECT COUNT(*) {scope}"),
        params_from_iter(binds.iter()),
        |row| row.get(0),
    )?;
    binds.extend([
        Sql::Integer(input.per_page),
        Sql::Integer((input.page - 1) * input.per_page),
    ]);
    let mut stmt = conn.prepare(&format!(
        r#"SELECT e."id", e."replyChannel", e."replyType", e."errorDetail", e."httpCode", e."commentText",
                  c."id", c."firstName", c."lastName", c."avatar", e."occurredAt"
           FROM (SELECT * {scope} ORDER BY "occurredAt" DESC LIMIT ? OFFSET ?) e
           LEFT JOIN "Contact" c ON c."id" = e."contactId"
           ORDER BY e."occurredAt" DESC"#
    ))?;
    let data = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            let contact = row
                .get::<_, Option<i64>>(6)?
                .map(|_| -> chatbotx_db::rusqlite::Result<_> {
                    Ok((row.get(7)?, row.get(8)?, row.get(9)?))
                })
                .transpose()?;
            Ok(ErrorRow {
                id: row.get(0)?,
                reply_channel: row.get(1)?,
                reply_type: row.get(2)?,
                error_detail: row.get(3)?,
                http_code: row.get(4)?,
                comment_text: row.get(5)?,
                contact,
                occurred_at: to_iso_string(row.get(10)?).unwrap_or_default(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Page {
        data,
        total,
        page: input.page,
        page_count: page_count(total, input.per_page),
    })
}

/// One `commentAutomationContactPublicResource` row (no name or avatar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactRow {
    pub row_key: String,
    pub contact_id: String,
    pub contact_inbox_id: String,
    pub source_id: String,
    pub channel: String,
    pub conversation_id: String,
    pub error_content: Option<String>,
    pub occurred_at: String,
    pub comment_text: Option<String>,
    pub miss_reason: Option<String>,
}

/// `ListCommentAutomationContactsResponse`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contacts {
    pub data: Vec<ContactRow>,
    pub total: i64,
    pub contact_total: i64,
    pub page: i64,
    pub page_count: i64,
}

/// `listCommentAutomationContacts` for an automation [`find`] returned
/// (`kind` its type, `total` its counter): one row per event, newest first.
/// `comment:missed` reads the misses; every other type the replies of the
/// counted half (the DM, or the public reply on a channel without DMs).
/// Rows whose contact inbox is gone or of another workspace are dropped.
#[allow(clippy::too_many_arguments)] // the drill-down's own inputs
pub fn contacts(
    conn: &Connection,
    workspace_id: i64,
    automation_id: i64,
    kind: &str,
    event_type: &str,
    total: i64,
    page: i64,
    per_page: i64,
) -> repo::Result<Contacts> {
    let mut binds = vec![Sql::Integer(workspace_id), Sql::Integer(automation_id)];
    let (select, from) = if event_type == "comment:missed" {
        (
            r#"m."id", m."contactId", m."contactInboxId", NULL, m."occurredAt", m."commentText", m."reason""#,
            r#""CommentAutomationMiss" m WHERE m."workspaceId" = ? AND m."automationId" = ?
               AND m."contactInboxId" IS NOT NULL"#
                .to_string(),
        )
    } else {
        let (condition, at) = match event_type {
            "message:delivered" => (r#"m."deliveredAt" IS NOT NULL"#, r#"m."deliveredAt""#),
            "message:seen" => (r#"m."seenAt" IS NOT NULL"#, r#"m."seenAt""#),
            "message:failed" => (r#"m."failedAt" IS NOT NULL"#, r#"m."failedAt""#),
            "flow:clicked" => (r#"m."clickedAt" IS NOT NULL"#, r#"m."clickedAt""#),
            _ => (
                r#"(m."deliveredAt" IS NOT NULL OR m."failedAt" IS NOT NULL)"#,
                r#"COALESCE(m."deliveredAt", m."failedAt")"#,
            ),
        };
        let channel = if kind == "threads" {
            "public"
        } else {
            "private"
        };
        binds.push(Sql::Text(channel.into()));
        (
            match at {
                r#"m."deliveredAt""# => {
                    r#"m."id", m."contactId", m."contactInboxId", m."errorDetail", m."deliveredAt", NULL, NULL"#
                }
                r#"m."seenAt""# => {
                    r#"m."id", m."contactId", m."contactInboxId", m."errorDetail", m."seenAt", NULL, NULL"#
                }
                r#"m."failedAt""# => {
                    r#"m."id", m."contactId", m."contactInboxId", m."errorDetail", m."failedAt", NULL, NULL"#
                }
                r#"m."clickedAt""# => {
                    r#"m."id", m."contactId", m."contactInboxId", m."errorDetail", m."clickedAt", NULL, NULL"#
                }
                _ => {
                    r#"m."id", m."contactId", m."contactInboxId", m."errorDetail", COALESCE(m."deliveredAt", m."failedAt"), NULL, NULL"#
                }
            },
            format!(
                r#""CommentAutomationEvent" m WHERE m."workspaceId" = ? AND m."automationId" = ?
                   AND m."replyChannel" = ? AND m."contactInboxId" IS NOT NULL AND m."contactId" IS NOT NULL
                   AND {condition}"#
            ),
        )
    };
    let contact_total: i64 = conn.query_row(
        &format!(r#"SELECT COUNT(DISTINCT m."contactId") FROM {from}"#),
        params_from_iter(binds.iter()),
        |row| row.get(0),
    )?;
    binds.extend([Sql::Integer(per_page), Sql::Integer((page - 1) * per_page)]);
    // The order column of `buildEventFilter` (the 5th selected), newest
    // first, `id` breaking ties; the misses order by their own time.
    let order = if event_type == "comment:missed" {
        r#"5 DESC"#
    } else {
        r#"5 DESC, 1 DESC"#
    };
    let mut stmt = conn.prepare(&format!(
        r#"SELECT p.*, ci."contactId", ci."sourceId", ci."channel",
                  (SELECT MIN(cv."id") FROM "Conversation" cv WHERE cv."contactId" = ci."contactId")
           FROM (SELECT {select} FROM {from} ORDER BY {order} LIMIT ? OFFSET ?) p
           LEFT JOIN "ContactInbox" ci ON ci."id" = p."contactInboxId"
             AND EXISTS (SELECT 1 FROM "Contact" c WHERE c."id" = ci."contactId" AND c."workspaceId" = ?)
           ORDER BY {order}"#
    ))?;
    binds.push(Sql::Integer(workspace_id));
    let rows = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            let inbox_contact: Option<i64> = row.get(7)?;
            Ok(
                inbox_contact.map(|_| -> chatbotx_db::rusqlite::Result<ContactRow> {
                    Ok(ContactRow {
                        row_key: row.get::<_, i64>(0)?.to_string(),
                        contact_id: row.get::<_, i64>(7)?.to_string(),
                        contact_inbox_id: row.get::<_, i64>(2)?.to_string(),
                        source_id: row.get(8)?,
                        channel: row.get(9)?,
                        conversation_id: row
                            .get::<_, Option<i64>>(10)?
                            .map(|v| v.to_string())
                            .unwrap_or_default(),
                        error_content: row.get(3)?,
                        occurred_at: row
                            .get::<_, Option<i64>>(4)?
                            .and_then(to_iso_string)
                            .unwrap_or_default(),
                        comment_text: row.get(5)?,
                        miss_reason: row.get(6)?,
                    })
                }),
            )
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let data = rows.into_iter().flatten().collect::<Result<Vec<_>, _>>()?;
    Ok(Contacts {
        data,
        total,
        contact_total,
        page,
        page_count: page_count(total, per_page),
    })
}
