//! Port of `repositories/template-installed-resource/uninstall.ts`
//! (`templateUninstallRepository` of the installed-resource folder); no TS tests, smoke
//! test `tests/repo_p33_smoke.rs`. `lockKeywords` (`FOR UPDATE`) is a plain read under the
//! one SQLite writer.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Connection;
use rusqlite::types::Value;

use super::sql::Sql;
use super::{Result, id_list, text_list};

/// `TEMPLATE_UNINSTALL_TOMBSTONES`.
pub const TEMPLATE_UNINSTALL_TOMBSTONES: [&str; 3] =
    ["deleting", "deletingModified", "deletingUnverified"];

/// `TEMPLATE_UNINSTALL_DOOMED`: tombstones and failures awaiting a retry.
pub const TEMPLATE_UNINSTALL_DOOMED: [&str; 9] = [
    "deleting",
    "deletingModified",
    "deletingUnverified",
    "failed",
    "failedModified",
    "failedUnverified",
    "blockedByReference",
    "blockedByReferenceModified",
    "blockedByReferenceUnverified",
];

/// `valueMentions`: the ids of `wanted` a JSON value (string / number / nested arrays)
/// mentions.
fn value_mentions(
    value: &serde_json::Value,
    wanted: &BTreeSet<String>,
    out: &mut BTreeSet<String>,
) {
    match value {
        serde_json::Value::String(s) if wanted.contains(s) => {
            out.insert(s.clone());
        }
        serde_json::Value::Number(n) => {
            let s = n.to_string();
            if wanted.contains(&s) {
                out.insert(s);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                value_mentions(item, wanted, out);
            }
        }
        _ => {}
    }
}

/// `lockKeywords`: keyword id -> `inbound` / `outbound` (any other type reads `inbound`).
pub fn lock_keywords(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
) -> Result<BTreeMap<i64, String>> {
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let rows: Vec<(i64, String)> = Sql::new(
        "SELECT \"id\", \"type\" FROM \"AutomatedResponse\" WHERE \"workspaceId\" = ? AND \"id\" IN (SELECT value FROM json_each(?))",
        vec![Value::Integer(workspace_id), Value::Text(id_list(ids))],
    )
    .query_map(conn, |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows
        .into_iter()
        .map(|(id, t)| {
            let kind = if t == "outbound" {
                "outbound"
            } else {
                "inbound"
            };
            (id, kind.to_owned())
        })
        .collect())
}

/// `findReferencedKeywordIds`: the keyword ids still referenced by a trigger / webhook
/// condition, a live broadcast filter or an ads conversion rule.
pub fn find_referenced_keyword_ids(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
) -> Result<BTreeSet<i64>> {
    let wanted: BTreeSet<String> = ids.iter().map(ToString::to_string).collect();
    let mut referenced: BTreeSet<String> = BTreeSet::new();
    if wanted.is_empty() {
        return Ok(BTreeSet::new());
    }
    let conditions: Vec<Option<String>> = Sql::new(
        "SELECT c.\"value\" FROM \"Condition\" c LEFT JOIN \"Trigger\" t ON t.\"id\" = c.\"triggerId\" LEFT JOIN \"Webhook\" w ON w.\"id\" = c.\"webhookId\" WHERE c.\"type\" = 'keywordsReceived' AND (t.\"workspaceId\" = ? OR w.\"workspaceId\" = ?)",
        vec![Value::Integer(workspace_id), Value::Integer(workspace_id)],
    )
    .query_map(conn, |row| row.get(0))?;
    for text in conditions.into_iter().flatten() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            value_mentions(&value, &wanted, &mut referenced);
        }
    }
    let filters: Vec<Option<String>> = Sql::new(
        "SELECT \"contactFilter\" FROM \"Broadcast\" WHERE \"workspaceId\" = ? AND \"deletedAt\" IS NULL AND \"contactFilter\" LIKE '%keywordsReceived%'",
        vec![Value::Integer(workspace_id)],
    )
    .query_map(conn, |row| row.get(0))?;
    for text in filters.into_iter().flatten() {
        // `JSON.stringify` of the stored filter (compact JSON text).
        let compact =
            serde_json::from_str::<serde_json::Value>(&text).map_or(text, |v| v.to_string());
        for id in &wanted {
            if compact.contains(&format!("\"{id}\"")) {
                referenced.insert(id.clone());
            }
        }
    }
    let rules: Vec<String> = Sql::new(
        "SELECT \"trigger\" FROM \"AdsConversionRule\" WHERE \"workspaceId\" = ? AND \"trigger\" ->> '$.type' = 'keywordMatched'",
        vec![Value::Integer(workspace_id)],
    )
    .query_map(conn, |row| row.get(0))?;
    for text in rules {
        if let Ok(trigger) = serde_json::from_str::<serde_json::Value>(&text)
            && let Some(ids) = trigger.get("automatedResponseIds")
        {
            value_mentions(ids, &wanted, &mut referenced);
        }
    }
    Ok(referenced
        .into_iter()
        .filter_map(|s| s.parse::<i64>().ok())
        .collect())
}

/// `findTombstoned`: the resource ids an uninstall has doomed and not finished.
pub fn find_tombstoned(
    conn: &Connection,
    workspace_id: i64,
    resource_kind: &str,
    resource_ids: &[i64],
) -> Result<Vec<i64>> {
    if resource_ids.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "SELECT \"resourceId\" FROM \"TemplateInstalledResource\" WHERE \"workspaceId\" = ? AND \"resourceKind\" = ? AND \"resourceId\" IN (SELECT value FROM json_each(?)) AND \"wasExisting\" = 0 AND \"detachedAt\" IS NULL AND \"uninstallOutcome\" IN (SELECT value FROM json_each(?))",
        vec![
            Value::Integer(workspace_id),
            Value::Text(resource_kind.to_owned()),
            Value::Text(id_list(resource_ids)),
            Value::Text(text_list(&TEMPLATE_UNINSTALL_DOOMED[..])),
        ],
    )
    .query_map(conn, |row| row.get(0))
}

/// `deleteKeywords`: deletes the keywords of one type.
pub fn delete_keywords(
    conn: &Connection,
    workspace_id: i64,
    ids: &[i64],
    r#type: &str,
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM \"AutomatedResponse\" WHERE \"workspaceId\" = ?1 AND \"id\" IN (SELECT value FROM json_each(?2)) AND \"type\" = ?3",
        (workspace_id, id_list(ids), r#type),
    )?;
    Ok(())
}

/// `findDeleteBlockedByOthers`: resource ids another active installation forbids deleting.
pub fn find_delete_blocked_by_others(
    conn: &Connection,
    workspace_id: i64,
    resource_kind: &str,
    resource_ids: &[i64],
    exclude_installation_ids: &[i64],
) -> Result<BTreeSet<i64>> {
    if resource_ids.is_empty() {
        return Ok(BTreeSet::new());
    }
    let rows: Vec<i64> = Sql::new(
        "SELECT r.\"resourceId\" FROM \"TemplateInstalledResource\" r JOIN \"TemplateInstallation\" i ON i.\"id\" = r.\"installationId\" WHERE r.\"workspaceId\" = ? AND r.\"resourceKind\" = ? AND r.\"resourceId\" IN (SELECT value FROM json_each(?)) AND r.\"installationId\" NOT IN (SELECT value FROM json_each(?)) AND r.\"wasExisting\" = 0 AND r.\"detachedAt\" IS NULL AND r.\"allowDelete\" = 0 AND i.\"sourceDeletedAt\" IS NULL AND i.\"templateId\" IS NOT NULL",
        vec![
            Value::Integer(workspace_id),
            Value::Text(resource_kind.to_owned()),
            Value::Text(id_list(resource_ids)),
            Value::Text(id_list(exclude_installation_ids)),
        ],
    )
    .query_map(conn, |row| row.get(0))?;
    Ok(rows.into_iter().collect())
}
