//! Port of `repositories/reference-existence/repository.ts` (P3.3).
#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    unreachable_code,
    clippy::all
)]

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
    todo!("P3.3")
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
    todo!("P3.3")
}
