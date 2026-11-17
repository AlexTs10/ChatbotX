//! `template/service.ts` (`templateService`), the publisher side without the
//! snapshot save: list, detail, share link, versions, install figures, soft
//! delete. One SQLite writer stands in for the Postgres row locks.

use chatbotx_db::Db;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::{Value, json};

use super::permissions::{Actor, assert_can_manage, assert_can_read};
use super::{ACTIVE_STATUSES_SQL, db_error, decode_text_cursor, encode_text_cursor, json_col};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `TEMPLATE_LIST_MAX_LIMIT`.
const LIST_MAX_LIMIT: i64 = 100;
/// `VERSION_HISTORY_LIMIT`.
const VERSION_HISTORY_LIMIT: i64 = 50;

/// `TemplateDetail` (the row without `payload`).
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub workspace_id: i64,
    pub tenant_id: i64,
    pub name: String,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub publisher_name: Option<String>,
    pub youtube_video_id: Option<String>,
    pub test_link: Option<String>,
    pub selection: Value,
    pub category_counts: Value,
    pub share_token: String,
    pub share_enabled: bool,
    pub share_expires_at: Option<i64>,
    pub default_permissions: Value,
    pub create_install_folder: bool,
    pub default_auto_update: bool,
    pub version: i64,
}

const COLUMNS: &str = r#""id", "createdAt", "updatedAt", "workspaceId", "tenantId", "name",
    "description", "imageUrl", "publisherName", "youtubeVideoId", "testLink", "selection",
    "categoryCounts", "shareToken", "shareEnabled", "shareExpiresAt", "defaultPermissions",
    "createInstallFolder", "defaultAutoUpdate", "version""#;

fn template_of(row: &Row<'_>) -> chatbotx_db::rusqlite::Result<Template> {
    Ok(Template {
        id: row.get(0)?,
        created_at: row.get(1)?,
        updated_at: row.get(2)?,
        workspace_id: row.get(3)?,
        tenant_id: row.get(4)?,
        name: row.get(5)?,
        description: row.get(6)?,
        image_url: row.get(7)?,
        publisher_name: row.get(8)?,
        youtube_video_id: row.get(9)?,
        test_link: row.get(10)?,
        selection: json_col(row, 11)?,
        category_counts: json_col(row, 12)?,
        share_token: row.get(13)?,
        share_enabled: row.get(14)?,
        share_expires_at: row.get(15)?,
        default_permissions: json_col(row, 16)?,
        create_install_folder: row.get(17)?,
        default_auto_update: row.get(18)?,
        version: row.get(19)?,
    })
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Template not found")
}

fn invalid_cursor() -> BusinessError {
    BusinessError::exception("Invalid cursor", "invalidCursor", 400)
}

/// `templateRepository.findLiveDetail`.
pub(crate) fn find_live(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Template>> {
    conn.query_row(
        &format!(
            r#"SELECT {COLUMNS} FROM "Template"
               WHERE "id" = ?1 AND "workspaceId" = ?2 AND "deletedAt" IS NULL"#
        ),
        params![id, workspace_id],
        template_of,
    )
    .optional()
    .map_err(db_error)
}

/// `getDetailOrFail` / `findByIdOrFail`.
pub async fn get_detail(db: &Db, workspace_id: i64, id: i64) -> Result<Template> {
    tx::read(db, move |conn| {
        find_live(conn, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// One page of `listPage`: the items with their active install count.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplatePage {
    pub items: Vec<(Template, i64)>,
    pub next_cursor: Option<String>,
}

/// `listPage`: newest first (`createdAt`, then `id`), `limit` 1..=100.
pub async fn list_page(
    db: &Db,
    workspace_id: i64,
    cursor: Option<String>,
    limit: Option<i64>,
) -> Result<TemplatePage> {
    let after = match cursor.as_deref().filter(|c| !c.is_empty()) {
        None => None,
        // A legacy offset cursor is no template list cursor.
        Some(c) if c.bytes().all(|b| b.is_ascii_digit()) => return Err(invalid_cursor()),
        Some(c) => {
            let (key, id) = decode_text_cursor(c).ok_or_else(invalid_cursor)?;
            Some((pg_text_ms(&key).ok_or_else(invalid_cursor)?, id))
        }
    };
    let limit = limit.unwrap_or(LIST_MAX_LIMIT).clamp(1, LIST_MAX_LIMIT);
    tx::read(db, move |conn| {
        let (after_ms, after_id) = after.unwrap_or((i64::MAX, i64::MAX));
        let mut stmt = conn
            .prepare(&format!(
                r#"SELECT {COLUMNS}, (SELECT coalesce(sum(i."successfulInstallEvents"), 0)
                     FROM "TemplateInstallation" i
                     WHERE i."templateId" = t."id" AND i."status" IN {ACTIVE_STATUSES_SQL})
                   FROM "Template" t
                   WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL
                     AND ("createdAt" < ?2 OR ("createdAt" = ?2 AND "id" < ?3))
                   ORDER BY "createdAt" DESC, "id" DESC LIMIT ?4"#
            ))
            .map_err(db_error)?;
        let mut items = stmt
            .query_map(
                params![workspace_id, after_ms, after_id, limit + 1],
                |row| Ok((template_of(row)?, row.get::<_, i64>(20)?)),
            )
            .map_err(db_error)?
            .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        let has_more = items.len() as i64 > limit;
        items.truncate(limit as usize);
        let next_cursor = has_more
            .then(|| items.last())
            .flatten()
            .map(|(last, _)| encode_text_cursor(&ms_pg_text(last.created_at), last.id));
        Ok(TemplatePage { items, next_cursor })
    })
    .await
}

/// Postgres `timestamptz::text` of a ms timestamp (`2026-10-30 10:00:00.123+00`).
fn ms_pg_text(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.format("%Y-%m-%d %H:%M:%S%.3f+00").to_string())
        .unwrap_or_default()
}

/// The ms of a `timestamptz::text` cursor key; `None` = not a timestamp.
fn pg_text_ms(key: &str) -> Option<i64> {
    let normalized = if key.ends_with('Z') {
        format!("{}+00:00", key.trim_end_matches('Z'))
    } else {
        key.to_string()
    };
    chrono::DateTime::parse_from_str(&normalized, "%Y-%m-%d %H:%M:%S%.f%#z")
        .ok()
        .map(|d| d.timestamp_millis())
}

/// `TemplateVersionHistoryItem`.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionItem {
    pub version: i64,
    pub created_at: i64,
    pub changes: Value,
}

/// `listVersions`: newest first, at most 50.
pub async fn list_versions(db: &Db, workspace_id: i64, id: i64) -> Result<Vec<VersionItem>> {
    tx::read(db, move |conn| {
        find_live(conn, workspace_id, id)?.ok_or_else(not_found)?;
        let mut stmt = conn
            .prepare(
                r#"SELECT "version", "createdAt", "changes" FROM "TemplateVersion"
                   WHERE "templateId" = ?1 ORDER BY "version" DESC LIMIT ?2"#,
            )
            .map_err(db_error)?;
        stmt.query_map(params![id, VERSION_HISTORY_LIMIT], |row| {
            Ok(VersionItem {
                version: row.get(0)?,
                created_at: row.get(1)?,
                changes: json_col(row, 2)?,
            })
        })
        .map_err(db_error)?
        .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
        .map_err(db_error)
    })
    .await
}

/// `getInstallAnalytics` as the JSON the API returns.
pub async fn install_analytics(
    db: &Db,
    workspace_id: i64,
    id: i64,
    actor: &Actor,
) -> Result<Value> {
    assert_can_read(actor)?;
    tx::read(db, move |conn| {
        find_live(conn, workspace_id, id)?.ok_or_else(not_found)?;
        let mut stmt = conn
            .prepare(
                r#"SELECT "status", count(*), sum("autoUpdate"),
                          coalesce(sum("successfulInstallEvents"), 0)
                   FROM "TemplateInstallation" WHERE "templateId" = ?1 GROUP BY "status""#,
            )
            .map_err(db_error)?;
        let rows = stmt
            .query_map(params![id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(db_error)?
            .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        let mut stmt = conn
            .prepare(&format!(
                r#"SELECT "sourceVersion", count(*) FROM "TemplateInstallation"
                   WHERE "templateId" = ?1 AND "status" IN {ACTIVE_STATUSES_SQL}
                   GROUP BY "sourceVersion" ORDER BY "sourceVersion""#
            ))
            .map_err(db_error)?;
        let by_version = stmt
            .query_map(params![id], |row| {
                Ok(json!({ "version": row.get::<_, i64>(0)?, "count": row.get::<_, i64>(1)? }))
            })
            .map_err(db_error)?
            .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        let active = |status: &str| status == "completed" || status == "partial";
        Ok(json!({
            "byStatus": rows.iter().map(|(s, c, _, _)| json!({ "status": s, "count": c })).collect::<Vec<_>>(),
            "activeAutoUpdate": rows.iter().filter(|r| active(&r.0)).map(|r| r.2).sum::<i64>(),
            "totalInstallEvents": rows.iter().map(|r| r.3).sum::<i64>(),
            "byVersion": by_version,
        }))
    })
    .await
}

/// `generateShareToken`: 32 random bytes, base64url without padding.
pub fn generate_share_token() -> String {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// `rotateShareToken`: every link shared before stops working.
pub async fn rotate_share_token(
    db: &Db,
    workspace_id: i64,
    id: i64,
    actor: &Actor,
) -> Result<Template> {
    assert_can_manage(actor)?;
    let token = generate_share_token();
    tx::write(db, move |tx| {
        let changed = tx
            .execute(
                r#"UPDATE "Template" SET "shareToken" = ?1,
                     "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER)
                   WHERE "id" = ?2 AND "workspaceId" = ?3 AND "deletedAt" IS NULL"#,
                params![token, id, workspace_id],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err(not_found());
        }
        find_live(tx, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// `updateShareSettings`; `expires_at` `None` keeps the expiry, `Some(None)` clears it.
pub async fn update_share_settings(
    db: &Db,
    workspace_id: i64,
    id: i64,
    enabled: bool,
    expires_at: Option<Option<i64>>,
    actor: &Actor,
) -> Result<Template> {
    assert_can_manage(actor)?;
    tx::write(db, move |tx| {
        let changed = tx
            .execute(
                r#"UPDATE "Template" SET "shareEnabled" = ?1,
                     "shareExpiresAt" = CASE WHEN ?2 THEN ?3 ELSE "shareExpiresAt" END,
                     "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER)
                   WHERE "id" = ?4 AND "workspaceId" = ?5 AND "deletedAt" IS NULL"#,
                params![
                    enabled,
                    expires_at.is_some(),
                    expires_at.flatten(),
                    id,
                    workspace_id
                ],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err(not_found());
        }
        find_live(tx, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// `softDelete`: stamps the template and its installations (`markSourceDeleted`).
pub async fn soft_delete(db: &Db, workspace_id: i64, id: i64, actor: &Actor) -> Result<()> {
    assert_can_manage(actor)?;
    tx::write(db, move |tx| {
        let now = chatbotx_db::repo::now_ms();
        let changed = tx
            .execute(
                r#"UPDATE "Template" SET "deletedAt" = ?1, "updatedAt" = ?1
                   WHERE "id" = ?2 AND "workspaceId" = ?3 AND "deletedAt" IS NULL"#,
                params![now, id, workspace_id],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err(not_found());
        }
        tx.execute(
            r#"UPDATE "TemplateInstallation" SET "sourceDeletedAt" = ?1
               WHERE "sourceTemplateId" = ?2 AND "sourceDeletedAt" IS NULL"#,
            params![now, id],
        )
        .map_err(db_error)?;
        Ok(())
    })
    .await
}

/// `resolveTemplateShareUrl` on the app URL (one tenant, no custom domain).
pub fn share_url(app_url: &str, template_id: i64, share_token: &str) -> String {
    format!(
        "{}/t/template?id={template_id}&key={}",
        app_url.trim_end_matches('/'),
        chatbotx_core::js::encode_uri_component(share_token)
    )
}

/// `isShareLive`.
pub fn is_share_live(template: &Template, now_ms: i64) -> bool {
    template.share_enabled && template.share_expires_at.is_none_or(|at| at >= now_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_keys_round_trip() {
        let text = ms_pg_text(1_798_000_000_123);
        assert_eq!(pg_text_ms(&text), Some(1_798_000_000_123));
        assert_eq!(
            pg_text_ms("2026-10-30 10:00:00.123456+00"),
            pg_text_ms("2026-10-30 10:00:00.123+00")
        );
        assert_eq!(pg_text_ms("not a date"), None);
        assert_eq!(generate_share_token().len(), 43);
    }
}
