//! `magicLinkAnalyticsService` / `refLinkAnalyticsService` reads
//! (`LinkStatsRepository`, `listLinkContactStats`).

use chatbotx_core::js::to_iso_string;
use chatbotx_db::repo;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params_from_iter};

use super::{Granularity, zone};

/// The link-stat table family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// `MagicLinkStat` of a `MagicLink`.
    Magic,
    /// `RefLinkStat` of a `Reflink`.
    Ref,
}

impl Link {
    fn tables(self) -> (&'static str, &'static str) {
        match self {
            Link::Magic => ("MagicLinkStat", "MagicLink"),
            Link::Ref => ("RefLinkStat", "Reflink"),
        }
    }
}

/// `getStatsByDateRange`: clicks per local day of `timezone` (`dateReport`
/// `YYYY-MM-DD`, ascending, no zero fill, no link check).
pub fn stats_by_day(
    conn: &Connection,
    link: Link,
    workspace_id: i64,
    link_id: i64,
    (from, to): (i64, i64),
    timezone: &str,
) -> repo::Result<Vec<(String, i64)>> {
    let (stats, _) = link.tables();
    let key = Granularity::Day.key_sql(r#""occurredAt""#, zone(timezone));
    let sql = format!(
        r#"SELECT {key}, COUNT(*) FROM "{stats}"
           WHERE "workspaceId" = ?1 AND "linkId" = ?2 AND "occurredAt" >= ?3 AND "occurredAt" <= ?4
           GROUP BY 1 ORDER BY 1 ASC"#
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([workspace_id, link_id, from, to], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One `linkContactPublicResource` row (`flowNodeContactData` without the
/// name and avatar). A click whose contact inbox is gone keeps only its time:
/// empty ids, `None` source and channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkContact {
    pub contact_id: String,
    pub contact_inbox_id: String,
    pub source_id: Option<String>,
    pub channel: Option<String>,
    pub conversation_id: String,
    /// ISO time of the contact inbox's first click in the range.
    pub occurred_at: String,
}

/// `ListFlowNodeContactsResponse` of the link contacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkContacts {
    pub data: Vec<LinkContact>,
    pub total: i64,
    pub page: i64,
    pub page_count: i64,
}

/// `listLinkContactStats`: one row per contact inbox (its first click),
/// newest first, paged; `total` counts the distinct contact inboxes. The
/// range applies only when both ends are given. An unknown or foreign link
/// is an empty first page.
pub fn contacts(
    conn: &Connection,
    link: Link,
    workspace_id: i64,
    link_id: Option<i64>,
    page: i64,
    per_page: i64,
    range: Option<(i64, i64)>,
) -> repo::Result<LinkContacts> {
    let empty = LinkContacts {
        data: Vec::new(),
        total: 0,
        page: 1,
        page_count: 0,
    };
    let (stats, owner) = link.tables();
    let Some(link_id) = link_id else {
        return Ok(empty);
    };
    let exists = conn
        .query_row(
            &format!(r#"SELECT 1 FROM "{owner}" WHERE "id" = ?1 AND "workspaceId" = ?2"#),
            [link_id, workspace_id],
            |_| Ok(()),
        )
        .optional()?;
    if exists.is_none() {
        return Ok(empty);
    }
    let mut binds = vec![Sql::Integer(workspace_id), Sql::Integer(link_id)];
    let window = match range {
        Some((from, to)) => {
            binds.extend([Sql::Integer(from), Sql::Integer(to)]);
            r#" AND "occurredAt" >= ? AND "occurredAt" <= ?"#
        }
        None => "",
    };
    let filter = format!(r#"FROM "{stats}" WHERE "workspaceId" = ? AND "linkId" = ?{window}"#);
    let total: i64 = conn.query_row(
        &format!(r#"SELECT COUNT(DISTINCT "contactInboxId") {filter}"#),
        params_from_iter(binds.iter()),
        |row| row.get(0),
    )?;
    binds.extend([Sql::Integer(per_page), Sql::Integer((page - 1) * per_page)]);
    // The contact inbox, its contact and the contact's first conversation
    // (`contactInbox.conversation` joins on `contactId`).
    let sql = format!(
        r#"SELECT ci."id", ci."contactId", ci."sourceId", ci."channel",
                  (SELECT MIN(c."id") FROM "Conversation" c WHERE c."contactId" = ci."contactId"),
                  s.at
           FROM (SELECT "contactInboxId" AS id, MIN("occurredAt") AS at {filter}
                 GROUP BY "contactInboxId" ORDER BY at DESC, "contactInboxId" LIMIT ? OFFSET ?) s
           LEFT JOIN "ContactInbox" ci ON ci."id" = s.id
           ORDER BY s.at DESC, s.id"#
    );
    let mut stmt = conn.prepare(&sql)?;
    let data = stmt
        .query_map(params_from_iter(binds.iter()), |row| {
            let id = |i: usize| -> chatbotx_db::rusqlite::Result<String> {
                Ok(row
                    .get::<_, Option<i64>>(i)?
                    .map(|v| v.to_string())
                    .unwrap_or_default())
            };
            Ok(LinkContact {
                contact_inbox_id: id(0)?,
                contact_id: id(1)?,
                source_id: row.get(2)?,
                channel: row.get(3)?,
                conversation_id: id(4)?,
                occurred_at: to_iso_string(row.get(5)?).unwrap_or_default(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(LinkContacts {
        data,
        total,
        page,
        page_count: (total as u64).div_ceil(per_page.max(1) as u64) as i64,
    })
}
