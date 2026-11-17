//! `packages/business/src/template` (Q.D4): the template marketplace.
//!
//! Ported: permissions, the view-only / undeletable lookups, the publisher
//! reads and share settings ([`service`]) and the installer's installation
//! reads and settings ([`installations`]). The snapshot save, install, sync,
//! uninstall engines and their worker jobs are not ported yet (specs/Q.D4.md).
//! Postgres advisory locks are the one SQLite writer.

pub mod installations;
pub mod permissions;
pub mod service;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::{Row, params_from_iter, types::Value as Sql};
use serde_json::Value;

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `ACTIVE_INSTALLATION_STATUSES` as an SQL list.
pub(crate) const ACTIVE_STATUSES_SQL: &str = "('completed', 'partial')";

/// `templateResourceKinds`.
pub const RESOURCE_KINDS: &[&str] = &[
    "flow",
    "aiAgent",
    "aiFunction",
    "calendar",
    "integrationWebchat",
    "automatedResponse",
    "reflink",
    "trigger",
    "fbCommentAutomation",
    "product",
    "savedReply",
    "botField",
    "customField",
    "tag",
    "folder",
    "pipeline",
    "productCategory",
    "placeCategory",
    "sequence",
    "aiMcpServer",
    "aiFile",
    "questionnaire",
    "couponTopic",
];

pub(crate) fn db_error(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// A JSON text column (`NULL` = `null`).
pub(crate) fn json_col(row: &Row<'_>, index: usize) -> chatbotx_db::rusqlite::Result<Value> {
    let text: Option<String> = row.get(index)?;
    Ok(text
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null))
}

/// `encodeSelectableCursor`: base64url of `encodeURIComponent(JSON [key, id])`.
pub(crate) fn encode_text_cursor(key: &str, id: i64) -> String {
    let text = serde_json::json!([key, id.to_string()]).to_string();
    URL_SAFE_NO_PAD.encode(chatbotx_core::js::encode_uri_component(&text))
}

/// `decodeSelectableCursor` of an `after` cursor: `(key, id)`.
pub(crate) fn decode_text_cursor(cursor: &str) -> Option<(String, i64)> {
    let bytes = URL_SAFE_NO_PAD.decode(cursor.trim_end_matches('=')).ok()?;
    let text = chatbotx_core::js::decode_uri_component(std::str::from_utf8(&bytes).ok()?).ok()?;
    let parsed: Value = serde_json::from_str(&text).ok()?;
    let [Value::String(key), Value::String(id)] = parsed.as_array()?.as_slice() else {
        return None;
    };
    if key.contains('\0')
        || id.is_empty()
        || id.len() > 19
        || !id.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((key.clone(), id.parse().ok()?))
}

/// Which kind of template rule `list_blocking_ids` reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocking {
    /// `listLockedIds`: the content is view-only.
    Edit,
    /// `listUndeletableIds`: the template forbids deleting it.
    Delete,
}

/// `listLockedIds` / `listUndeletableIds` (`findBlocking`): the ids, among
/// `resource_ids`, of live copies of a still-governing installation whose
/// `allowEdit` / `allowDelete` is off. One query for the page.
pub async fn list_blocking_ids(
    db: &Db,
    workspace_id: i64,
    resource_kind: &str,
    resource_ids: &[i64],
    blocking: Blocking,
) -> Result<Vec<String>> {
    if resource_ids.is_empty() {
        return Ok(Vec::new());
    }
    let column = match blocking {
        Blocking::Edit => "allowEdit",
        Blocking::Delete => "allowDelete",
    };
    let marks = vec!["?"; resource_ids.len()].join(", ");
    let sql = format!(
        r#"SELECT DISTINCT r."resourceId" FROM "TemplateInstalledResource" r
           JOIN "TemplateInstallation" i ON i."id" = r."installationId"
           WHERE r."workspaceId" = ? AND r."resourceKind" = ? AND r."resourceId" IN ({marks})
             AND r."wasExisting" = 0 AND r."detachedAt" IS NULL AND r."{column}" = 0
             AND i."sourceDeletedAt" IS NULL AND i."templateId" IS NOT NULL
           ORDER BY r."resourceId""#
    );
    let mut values = vec![
        Sql::Integer(workspace_id),
        Sql::Text(resource_kind.to_string()),
    ];
    values.extend(resource_ids.iter().map(|id| Sql::Integer(*id)));
    tx::read(db, move |conn| {
        let mut stmt = conn.prepare(&sql).map_err(db_error)?;
        stmt.query_map(params_from_iter(values), |row| {
            Ok(row.get::<_, i64>(0)?.to_string())
        })
        .map_err(db_error)?
        .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
        .map_err(db_error)
    })
    .await
}

/// `listLockedTemplateResourceIds`: `(lockedIds, undeletableIds)`.
pub async fn list_locked_resource_ids(
    db: &Db,
    workspace_id: i64,
    resource_kind: &str,
    resource_ids: &[i64],
) -> Result<(Vec<String>, Vec<String>)> {
    let locked = list_blocking_ids(
        db,
        workspace_id,
        resource_kind,
        resource_ids,
        Blocking::Edit,
    )
    .await?;
    let undeletable = list_blocking_ids(
        db,
        workspace_id,
        resource_kind,
        resource_ids,
        Blocking::Delete,
    )
    .await?;
    Ok((locked, undeletable))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_cursor_round_trip() {
        let cursor = encode_text_cursor("Café ü", 42);
        assert_eq!(
            decode_text_cursor(&cursor),
            Some(("Café ü".to_string(), 42))
        );
        assert_eq!(decode_text_cursor("bm90LWpzb24"), None);
        assert_eq!(
            decode_text_cursor(&URL_SAFE_NO_PAD.encode("%5B%22a%22%2C%22x%22%5D")),
            None
        );
    }
}
