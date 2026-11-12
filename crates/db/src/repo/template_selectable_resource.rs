//! Port of `repositories/template-selectable-resource/repository.ts` (P3.3).
//!
//! Difference: names sort with SQLite's `BINARY` collation (Postgres used the database
//! collation). Keyword keyset keys are the `createdAt` unix-ms text (Postgres used the
//! microsecond timestamp text; SQLite stores ms, so ms keys are exact here).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::sql::Sql;
use super::utils::like_contains;
use super::{Json, Result};

/// `ALL_IDS_CAP`.
pub const ALL_IDS_CAP: i64 = 1000;

/// `SelectableResourceRow` (ids as API strings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectableResourceRow {
    pub id: String,
    pub name: String,
    pub badge: Option<String>,
}

/// `ListSelectableResourceRowsResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectableResourcePage {
    pub rows: Vec<SelectableResourceRow>,
    pub total: i64,
    pub all_ids: Option<Vec<String>>,
    pub has_more: bool,
    pub last_key: Option<String>,
}

/// `SelectableKeysetCursor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysetCursor {
    pub key: String,
    pub id: String,
}

/// `CategoryInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CategoryInput {
    pub workspace_id: i64,
    pub keyword: Option<String>,
    pub offset: i64,
    pub limit: i64,
    pub after: Option<KeysetCursor>,
}

impl CategoryInput {
    fn is_first_page(&self) -> bool {
        self.offset == 0 && self.after.is_none()
    }
    fn keyword(&self) -> Option<&str> {
        self.keyword.as_deref().filter(|k| !k.is_empty())
    }
}

/// One `{id, name}` category: table, soft-delete column, badge column.
struct Category {
    table: &'static str,
    soft_delete: bool,
    badge: Option<&'static str>,
}

fn category(kind: &str) -> Option<Category> {
    let (table, soft_delete, badge) = match kind {
        "flows" => ("Flow", false, None),
        "tags" => ("Tag", true, None),
        "customFields" => ("CustomField", false, None),
        "products" => ("Product", false, None),
        "aiFunctions" => ("AIFunction", false, None),
        "aiAgents" => ("AIAgent", false, None),
        "calendars" => ("AppointmentCalendar", true, None),
        "webchats" => ("IntegrationWebchat", false, None),
        "triggers" => ("Trigger", false, None),
        "fbCommentAutomations" => ("CommentAutomation", false, Some("type")),
        "aiFiles" => ("AIFile", false, None),
        "aiMcpServers" => ("AIMCPServer", false, None),
        "questionnaires" => ("Questionnaire", true, None),
        "couponTopics" => ("CouponTopic", true, None),
        "pipelines" => ("Pipeline", false, None),
        "sequences" => ("Sequence", false, None),
        "entryPointLinks" => ("Reflink", false, None),
        _ => return None,
    };
    Some(Category {
        table,
        soft_delete,
        badge,
    })
}

/// The `{id, name}` category names [`list`] accepts (the TS `list*` methods).
pub const CATEGORIES: &[&str] = &[
    "flows",
    "tags",
    "customFields",
    "products",
    "aiFunctions",
    "aiAgents",
    "calendars",
    "webchats",
    "triggers",
    "fbCommentAutomations",
    "aiFiles",
    "aiMcpServers",
    "questionnaires",
    "couponTopics",
    "pipelines",
    "sequences",
    "entryPointLinks",
];

fn base_where(cat: &Category, input: &CategoryInput) -> Sql {
    Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", input.workspace_id)),
        cat.soft_delete.then(|| Sql::raw("\"deletedAt\" IS NULL")),
        input
            .keyword()
            .map(|k| Sql::bind("ilike(\"name\", ", like_contains(k)).closed(")")),
    ])
}

fn count(table: &str, filter: Sql, conn: &Connection) -> Result<i64> {
    let mut c = Sql::raw(format!("SELECT count(*) FROM \"{table}\" WHERE "));
    c.append(filter);
    Ok(c.query_map(conn, |r| r.get::<_, i64>(0))?
        .into_iter()
        .next()
        .unwrap_or(0))
}

/// `list<Category>` (`findAllQuery`): one `{id, name}` page by `(name, id)` with a keyset
/// cursor or offset, the total, and the capped `allIds` on the first page.
pub fn list(
    conn: &Connection,
    kind: &str,
    input: &CategoryInput,
) -> Result<SelectableResourcePage> {
    let Some(cat) = category(kind) else {
        return Err(super::Error::msg(format!(
            "unknown selectable resource category: {kind}"
        )));
    };
    let badge = cat.badge.map_or("NULL".to_owned(), |b| format!("\"{b}\""));
    let mut q = Sql::raw(format!(
        "SELECT CAST(\"id\" AS TEXT), \"name\", {badge} FROM \"{}\" WHERE ",
        cat.table
    ));
    let mut filter = base_where(&cat, input);
    if let Some(after) = &input.after {
        filter = Sql::and([
            Some(filter),
            Some(Sql::new(
                "(\"name\", \"id\") > (?, ?)",
                vec![Value::Text(after.key.clone()), id_value(&after.id)],
            )),
        ]);
    }
    q.append(filter);
    q.push(" ORDER BY \"name\" ASC, \"id\" ASC LIMIT ");
    q.push_bind(input.limit + 1);
    q.push(" OFFSET ");
    q.push_bind(input.offset);
    let fetched = q.query_map(conn, |r| {
        Ok(SelectableResourceRow {
            id: r.get(0)?,
            name: r.get(1)?,
            badge: r.get::<_, Option<String>>(2)?,
        })
    })?;
    let total = count(cat.table, base_where(&cat, input), conn)?;
    let (rows, has_more) = take_page(fetched, input.limit);
    let all_ids = if input.is_first_page() && total <= ALL_IDS_CAP {
        let mut a = Sql::raw(format!(
            "SELECT CAST(\"id\" AS TEXT) FROM \"{}\" WHERE ",
            cat.table
        ));
        a.append(base_where(&cat, input));
        a.push(" ORDER BY \"name\" ASC, \"id\" ASC LIMIT ");
        a.push_bind(ALL_IDS_CAP);
        Some(a.query_map(conn, |r| r.get::<_, String>(0))?)
    } else {
        None
    };
    let last_key = rows.last().map(|r| r.name.clone());
    Ok(SelectableResourcePage {
        rows,
        total,
        all_ids,
        has_more,
        last_key,
    })
}

fn id_value(id: &str) -> Value {
    id.parse::<i64>()
        .map_or_else(|_| Value::Text(id.to_owned()), Value::Integer)
}

fn take_page<T>(mut rows: Vec<T>, limit: i64) -> (Vec<T>, bool) {
    let limit = usize::try_from(limit.max(0)).unwrap_or(0);
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    (rows, has_more)
}

/// `listFlows`.
pub fn list_flows(conn: &Connection, input: &CategoryInput) -> Result<SelectableResourcePage> {
    list(conn, "flows", input)
}

/// `listTags` (live tags only).
pub fn list_tags(conn: &Connection, input: &CategoryInput) -> Result<SelectableResourcePage> {
    list(conn, "tags", input)
}

fn keyword_where(input: &CategoryInput) -> Sql {
    Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", input.workspace_id)),
        Some(Sql::raw("\"type\" = 'inbound'")),
        input.keyword().map(|k| {
            Sql::or([
                Some(Sql::bind("ilike(\"text\", ", like_contains(k)).closed(")")),
                Some(
                    Sql::bind(
                        "EXISTS (SELECT 1 FROM json_each(\"keywords\") WHERE value = ",
                        k.to_owned(),
                    )
                    .closed(")"),
                ),
            ])
        }),
    ])
}

/// `listKeywords`: inbound automated responses, newest first; label = `text`, else the
/// keyword list, else `(untitled)`.
pub fn list_keywords(conn: &Connection, input: &CategoryInput) -> Result<SelectableResourcePage> {
    let mut q = Sql::raw(
        "SELECT CAST(\"id\" AS TEXT), \"text\", \"keywords\", CAST(\"createdAt\" AS TEXT) FROM \"AutomatedResponse\" WHERE ",
    );
    let mut filter = keyword_where(input);
    if let Some(after) = &input.after {
        let key = after.key.parse::<i64>().unwrap_or_default();
        filter = Sql::and([
            Some(filter),
            Some(Sql::new(
                "(\"createdAt\", \"id\") < (?, ?)",
                vec![Value::Integer(key), id_value(&after.id)],
            )),
        ]);
    }
    q.append(filter);
    q.push(" ORDER BY \"createdAt\" DESC, \"id\" DESC LIMIT ");
    q.push_bind(input.limit + 1);
    q.push(" OFFSET ");
    q.push_bind(input.offset);
    let fetched = q.query_map(conn, |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Json>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let total = count("AutomatedResponse", keyword_where(input), conn)?;
    let (rows, has_more) = take_page(fetched, input.limit);
    let all_ids = if input.is_first_page() && total <= ALL_IDS_CAP {
        let mut a = Sql::raw("SELECT CAST(\"id\" AS TEXT) FROM \"AutomatedResponse\" WHERE ");
        a.append(keyword_where(input));
        a.push(" ORDER BY \"createdAt\" DESC, \"id\" DESC LIMIT ");
        a.push_bind(ALL_IDS_CAP);
        Some(a.query_map(conn, |r| r.get::<_, String>(0))?)
    } else {
        None
    };
    let last_key = rows.last().map(|r| r.3.clone());
    let rows = rows
        .into_iter()
        .map(|(id, text, keywords, _)| {
            let text = text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
            let joined = keywords
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let name = text.unwrap_or_else(|| {
                if joined.is_empty() {
                    "(untitled)".to_owned()
                } else {
                    joined
                }
            });
            SelectableResourceRow {
                id,
                name,
                badge: None,
            }
        })
        .collect();
    Ok(SelectableResourcePage {
        rows,
        total,
        all_ids,
        has_more,
        last_key,
    })
}

/// `listSettings`: `SavedReply` shortcuts and `BotField` names in one `(name, id)` list.
pub fn list_settings(conn: &Connection, input: &CategoryInput) -> Result<SelectableResourcePage> {
    let union = |sql: &mut Sql| {
        sql.push(
            "(SELECT \"id\", \"shortcut\" AS \"name\" FROM \"SavedReply\" WHERE \"workspaceId\" = ",
        );
        sql.push_bind(input.workspace_id);
        sql.push(" UNION ALL SELECT \"id\", \"name\" FROM \"BotField\" WHERE \"workspaceId\" = ");
        sql.push_bind(input.workspace_id);
        sql.push(") s");
    };
    let search = |sql: &mut Sql| {
        if let Some(k) = input.keyword() {
            sql.push(" WHERE ilike(s.\"name\", ");
            sql.push_bind(like_contains(k));
            sql.push(")");
        }
    };
    let mut q = Sql::raw("SELECT CAST(s.\"id\" AS TEXT), s.\"name\" FROM ");
    union(&mut q);
    search(&mut q);
    if let Some(after) = &input.after {
        q.push(if input.keyword().is_some() {
            " AND"
        } else {
            " WHERE"
        });
        q.push(" (s.\"name\", s.\"id\") > (");
        q.push_bind(after.key.clone());
        q.push(", ");
        q.push_bind(id_value(&after.id));
        q.push(")");
    }
    q.push(" ORDER BY s.\"name\" ASC, s.\"id\" ASC LIMIT ");
    q.push_bind(input.limit + 1);
    q.push(" OFFSET ");
    q.push_bind(input.offset);
    let fetched = q.query_map(conn, |r| {
        Ok(SelectableResourceRow {
            id: r.get(0)?,
            name: r.get(1)?,
            badge: None,
        })
    })?;
    let mut c = Sql::raw("SELECT count(*) FROM ");
    union(&mut c);
    search(&mut c);
    let total = c
        .query_map(conn, |r| r.get::<_, i64>(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    let (rows, has_more) = take_page(fetched, input.limit);
    let all_ids = if input.is_first_page() && total <= ALL_IDS_CAP {
        let mut a = Sql::raw("SELECT CAST(s.\"id\" AS TEXT) FROM ");
        union(&mut a);
        search(&mut a);
        a.push(" ORDER BY s.\"name\" ASC, s.\"id\" ASC LIMIT ");
        a.push_bind(ALL_IDS_CAP);
        Some(a.query_map(conn, |r| r.get::<_, String>(0))?)
    } else {
        None
    };
    let last_key = rows.last().map(|r| r.name.clone());
    Ok(SelectableResourcePage {
        rows,
        total,
        all_ids,
        has_more,
        last_key,
    })
}
