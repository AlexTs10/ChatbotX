//! Port of `repositories/trigger/repository.ts` (P3.3).
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

use rusqlite::Connection;

use super::Result;
use super::models::{Condition, Trigger};
use super::sql::Sql;

/// A trigger with its `conditions`.
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerWithConditions {
    pub trigger: Trigger,
    pub conditions: Vec<Condition>,
}

/// Input of [`list_paginated_with_conditions`]. `folder_id`: `None` = no folder filter;
/// `Some(None)` (TS `null` or `""`) = root (no folder); `Some(Some(id))` = that folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListTriggersInput {
    pub workspace_id: i64,
    pub folder_id: Option<Option<i64>>,
    pub name: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

/// Result of [`list_paginated_with_conditions`].
#[derive(Debug, Clone, PartialEq)]
pub struct TriggerPage {
    pub rows: Vec<TriggerWithConditions>,
    pub total: i64,
}

fn build_where(input: &ListTriggersInput) -> Sql {
    Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", input.workspace_id)),
        input.folder_id.map(|folder| match folder {
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

fn conditions_of(conn: &Connection, trigger_id: i64) -> Result<Vec<Condition>> {
    super::find_many(
        conn,
        "WHERE \"triggerId\" = ?1 ORDER BY \"id\"",
        &[&trigger_id],
    )
}

/// `listPaginatedWithConditions`: newest first (`createdAt`, then `id`), with conditions.
pub fn list_paginated_with_conditions(
    conn: &Connection,
    input: &ListTriggersInput,
) -> Result<TriggerPage> {
    todo!("P3.3")
}

/// `findWithConditions`: by `id` and/or `workspace_id`; `None` when neither is given.
pub fn find_with_conditions(
    conn: &Connection,
    id: Option<i64>,
    workspace_id: Option<i64>,
) -> Result<Option<TriggerWithConditions>> {
    todo!("P3.3")
}
