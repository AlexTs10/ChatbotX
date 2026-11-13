//! Port of `repositories/template-installed-resource/creation-order.ts` (`spreadCreatedAt`)
//! and `labels.ts` (`loadResourceLabels`); no TS tests, smoke test `tests/repo_p33_smoke.rs`.

use std::collections::BTreeMap;

use rusqlite::Connection;
use rusqlite::types::Value;

use super::sql::Sql;
use super::{Result, id_list};

/// The table of each template resource kind (`modelOfKind`).
pub fn table_of_kind(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "flow" => "Flow",
        "aiAgent" => "AIAgent",
        "aiFunction" => "AIFunction",
        "calendar" => "AppointmentCalendar",
        "integrationWebchat" => "IntegrationWebchat",
        "automatedResponse" => "AutomatedResponse",
        "reflink" => "Reflink",
        "trigger" => "Trigger",
        "fbCommentAutomation" => "CommentAutomation",
        "product" => "Product",
        "savedReply" => "SavedReply",
        "botField" => "BotField",
        "customField" => "CustomField",
        "tag" => "Tag",
        "folder" => "Folder",
        "pipeline" => "Pipeline",
        "productCategory" => "ProductCategory",
        "placeCategory" => "PlaceCategory",
        "sequence" => "Sequence",
        "aiMcpServer" => "AIMCPServer",
        "aiFile" => "AIFile",
        "questionnaire" => "Questionnaire",
        "couponTopic" => "CouponTopic",
        _ => return None,
    })
}

/// `spreadCreatedAt`: for each kind with two or more rows, adds `position` ms to the
/// `createdAt` of the n-th id (1-based), so "newest first" lists keep the install order.
pub fn spread_created_at(conn: &Connection, rows: &[(String, i64)]) -> Result<()> {
    let mut by_kind: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for (kind, id) in rows {
        by_kind.entry(kind.as_str()).or_default().push(*id);
    }
    for (kind, ids) in by_kind {
        let Some(table) = table_of_kind(kind) else {
            continue;
        };
        if ids.len() < 2 {
            continue;
        }
        conn.execute(
            &format!(
                "UPDATE \"{table}\" AS target SET \"createdAt\" = target.\"createdAt\" + ordered.position FROM (SELECT CAST(value AS INTEGER) AS id, key + 1 AS position FROM json_each(?1)) AS ordered WHERE target.\"id\" = ordered.id"
            ),
            [id_list(&ids)],
        )?;
    }
    Ok(())
}

/// The label SQL of a kind: `(table, label expression, soft-deleted rows skipped)`.
fn label_source(kind: &str) -> Option<(&'static str, &'static str, bool)> {
    let table = table_of_kind(kind)?;
    Some(match kind {
        "automatedResponse" => (
            table,
            "(SELECT group_concat(value, ', ') FROM json_each(t.\"keywords\"))",
            false,
        ),
        "savedReply" => (table, "t.\"shortcut\"", false),
        "calendar" | "questionnaire" | "couponTopic" => (table, "t.\"name\"", true),
        _ => (table, "t.\"name\"", false),
    })
}

/// `loadResourceLabels`: `"<kind>:<id>"` -> display label of the resources that still
/// exist in the workspace (unknown kinds are skipped).
pub fn load_resource_labels(
    conn: &Connection,
    workspace_id: i64,
    resources: &[(String, i64)],
) -> Result<BTreeMap<String, String>> {
    let mut by_kind: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for (kind, id) in resources {
        by_kind.entry(kind.as_str()).or_default().push(*id);
    }
    let mut labels = BTreeMap::new();
    for (kind, ids) in by_kind {
        let Some((table, label, live_only)) = label_source(kind) else {
            continue;
        };
        let live = if live_only {
            " AND t.\"deletedAt\" IS NULL"
        } else {
            ""
        };
        let rows: Vec<(i64, Option<String>)> = Sql::new(
            format!(
                "SELECT t.\"id\", {label} FROM \"{table}\" t WHERE t.\"workspaceId\" = ? AND t.\"id\" IN (SELECT value FROM json_each(?)){live}"
            ),
            vec![Value::Integer(workspace_id), Value::Text(id_list(&ids))],
        )
        .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?;
        for (id, text) in rows {
            labels.insert(format!("{kind}:{id}"), text.unwrap_or_default());
        }
    }
    Ok(labels)
}
