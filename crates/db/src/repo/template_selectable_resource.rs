//! Port of `repositories/template-selectable-resource/repository.ts` (P3.3).
//!
//! Difference: names sort with SQLite's `BINARY` collation (Postgres used the database
//! collation). Keyword keyset keys are the `createdAt` unix-ms text (Postgres used the
//! microsecond timestamp text; SQLite stores ms, so ms keys are exact here).
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

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
    todo!("P3.3")
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
    todo!("P3.3")
}

/// `listTags` (live tags only).
pub fn list_tags(conn: &Connection, input: &CategoryInput) -> Result<SelectableResourcePage> {
    todo!("P3.3")
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
    todo!("P3.3")
}

/// `listSettings`: `SavedReply` shortcuts and `BotField` names in one `(name, id)` list.
pub fn list_settings(conn: &Connection, input: &CategoryInput) -> Result<SelectableResourcePage> {
    todo!("P3.3")
}
