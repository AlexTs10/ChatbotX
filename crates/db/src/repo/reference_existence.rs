//! Port of `repositories/reference-existence/repository.ts` (P3.3).

use std::collections::BTreeSet;

use rusqlite::Connection;

use super::{Json, Result, text_list};

/// `VERIFIABLE_REFERENCE_KINDS`.
pub const VERIFIABLE_REFERENCE_KINDS: &[&str] = &[
    "inbox",
    "tag",
    "pipeline",
    "folder",
    "productCategory",
    "product",
    "vendor",
    "sequence",
    "miniApp",
    "leaderboard",
    "placeCategory",
    "webhook",
    "spreadsheet",
    "decisionProfile",
    "userPersistentMenu",
    "integration",
    "messengerPersona",
    "messengerTemplate",
    "whatsappTemplate",
    "whatsappFlow",
];

/// `isVerifiableReferenceKind`.
pub fn is_verifiable_reference_kind(kind: &str) -> bool {
    VERIFIABLE_REFERENCE_KINDS.contains(&kind)
}

/// The table of a kind whose rows carry `workspaceId` themselves.
fn direct_table(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "inbox" => "Inbox",
        "tag" => "Tag",
        "pipeline" => "Pipeline",
        "folder" => "Folder",
        "productCategory" => "ProductCategory",
        "product" => "Product",
        "vendor" => "Vendor",
        "sequence" => "Sequence",
        "miniApp" => "MiniApp",
        "leaderboard" => "Leaderboard",
        "placeCategory" => "PlaceCategory",
        "webhook" => "Webhook",
        "spreadsheet" => "Spreadsheet",
        "decisionProfile" => "DecisionProfile",
        "userPersistentMenu" => "UserPersistentMenu",
        _ => return None,
    })
}

fn ids_of(conn: &Connection, sql: &str, workspace_id: i64, ids: &str) -> Result<BTreeSet<String>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let found = stmt
        .query_map(rusqlite::params![workspace_id, ids], |row| {
            row.get::<_, i64>(0)
        })?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(found.into_iter().map(|id| id.to_string()).collect())
}

/// `findExistingIds`: which of `ids` exist as rows of `kind` in the workspace. Ids are
/// the API strings (snowflakes, persona ids); the result keeps the input spelling.
pub fn find_existing_ids(
    conn: &Connection,
    workspace_id: i64,
    kind: &str,
    ids: &[String],
) -> Result<BTreeSet<String>> {
    if ids.is_empty() {
        return Ok(BTreeSet::new());
    }
    let list = text_list(ids);
    if let Some(table) = direct_table(kind) {
        let sql = format!(
            "SELECT \"id\" FROM \"{table}\" WHERE \"workspaceId\" = ?1 \
             AND CAST(\"id\" AS TEXT) IN (SELECT value FROM json_each(?2))"
        );
        return ids_of(conn, &sql, workspace_id, &list);
    }
    match kind {
        "messengerTemplate" => ids_of(
            conn,
            "SELECT t.\"id\" FROM \"MessengerMessageTemplate\" t \
             INNER JOIN \"IntegrationMessenger\" i ON t.\"integrationMessengerId\" = i.\"id\" \
             WHERE i.\"workspaceId\" = ?1 AND CAST(t.\"id\" AS TEXT) IN (SELECT value FROM json_each(?2))",
            workspace_id,
            &list,
        ),
        "whatsappTemplate" => ids_of(
            conn,
            "SELECT t.\"id\" FROM \"WhatsappMessageTemplate\" t \
             INNER JOIN \"IntegrationWhatsapp\" i ON t.\"integrationWhatsappId\" = i.\"id\" \
             WHERE i.\"workspaceId\" = ?1 AND CAST(t.\"id\" AS TEXT) IN (SELECT value FROM json_each(?2))",
            workspace_id,
            &list,
        ),
        "whatsappFlow" => ids_of(
            conn,
            "SELECT t.\"id\" FROM \"WhatsappFlow\" t \
             INNER JOIN \"IntegrationWhatsapp\" i ON t.\"integrationWhatsappId\" = i.\"id\" \
             WHERE i.\"workspaceId\" = ?1 AND CAST(t.\"id\" AS TEXT) IN (SELECT value FROM json_each(?2))",
            workspace_id,
            &list,
        ),
        "messengerPersona" => {
            let mut stmt = conn.prepare_cached(
                "SELECT \"personas\" FROM \"IntegrationMessenger\" WHERE \"workspaceId\" = ?1",
            )?;
            let rows = stmt
                .query_map([workspace_id], |row| row.get::<_, Json>(0))?
                .collect::<rusqlite::Result<Vec<Json>>>()?;
            let wanted: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
            Ok(rows
                .iter()
                .filter_map(|j| j.as_array())
                .flatten()
                .filter_map(|p| p.get("id").and_then(|v| v.as_str()))
                .filter(|id| wanted.contains(id))
                .map(str::to_owned)
                .collect())
        }
        "integration" => {
            let mut found = BTreeSet::new();
            for table in [
                "Integration",
                "IntegrationSmtp",
                "IntegrationMessenger",
                "IntegrationOpenaiCompatible",
            ] {
                let sql = format!(
                    "SELECT \"id\" FROM \"{table}\" WHERE \"workspaceId\" = ?1 \
                     AND CAST(\"id\" AS TEXT) IN (SELECT value FROM json_each(?2))"
                );
                found.extend(ids_of(conn, &sql, workspace_id, &list)?);
            }
            Ok(found)
        }
        _ => Ok(BTreeSet::new()),
    }
}
