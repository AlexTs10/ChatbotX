//! `packages/business/src/spreadsheet/service.ts` and the URL part of
//! `features/spreadsheets/lib/{google-sheets,manage-spreadsheet}.ts`.
//!
//! ponytail: the Google Sheets reads (`listSheetNames` on create / update,
//! `listWorksheets`, `listWorksheetHeaders`) wait for the Google Sheets
//! client (Q.I4); create / update check the integration row and the URL only.

use chatbotx_db::Db;
use chatbotx_db::repo::models::Spreadsheet;
use chatbotx_db::repo::{self, id_list, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::Value;

use crate::errors::{BusinessError, Result};
use crate::tx;

const NOT_FOUND: &str = "Spreadsheet not found";
/// `manageMessages.integrationMissing`.
pub const INTEGRATION_MISSING: &str = "You need to setup google sheets first.";
/// `manageMessages.invalidUrl`.
pub const INVALID_URL: &str = "URL must be a valid, public or shareable Google Sheets link.";

/// `isGoogleSpreadsheetUrl` (the request refine).
pub fn is_google_spreadsheet_url(text: &str) -> bool {
    url::Url::parse(text).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str() == Some("docs.google.com")
            && u.port().is_none()
            && u.path().starts_with("/spreadsheets/")
    })
}

/// `parseSpreadsheetId`: the id of `https://docs.google.com/spreadsheets/d/<id>/…`.
pub fn parse_spreadsheet_id(text: &str) -> Option<String> {
    if !is_google_spreadsheet_url(text) {
        return None;
    }
    let path = url::Url::parse(text).ok()?.path().to_string();
    // `/\/d\/([^/]+)\//`: the id must be followed by a slash.
    let rest = &path[path.find("/d/")? + 3..];
    let end = rest.find('/')?;
    (end > 0).then(|| rest[..end].to_string())
}

/// The public resource (`spreadsheetResource` without `workspaceId`).
pub fn public_json(row: &Spreadsheet) -> Value {
    let mut out = row.to_json();
    if let Some(obj) = out.as_object_mut() {
        obj.remove("workspaceId");
    }
    out
}

fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Spreadsheet> {
    repo::find_one::<Spreadsheet>(
        conn,
        r#""id" = ?1 AND "workspaceId" = ?2"#,
        &[&id, &workspace_id],
    )?
    .ok_or_else(|| BusinessError::not_found(NOT_FOUND))
}

/// `findByWorkspaceIdOrFail`.
pub async fn find_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<Spreadsheet> {
    tx::read(db, move |c| find_in(c, workspace_id, id)).await
}

/// `resolveSpreadsheetIdFromUrl` without the remote read (see the module note).
async fn resolve_id(db: &Db, workspace_id: i64, url: &str) -> Result<String> {
    let has_integration = tx::read(db, move |c| {
        Ok(c.query_row(
            r#"SELECT count(*) FROM "IntegrationGoogleSheet" WHERE "workspaceId" = ?1"#,
            [workspace_id],
            |r| r.get::<_, i64>(0),
        )
        .map_err(repo::Error::from)?
            > 0)
    })
    .await?;
    if !has_integration {
        return Err(BusinessError::validation("url", INTEGRATION_MISSING));
    }
    parse_spreadsheet_id(url).ok_or_else(|| BusinessError::validation("url", INVALID_URL))
}

/// `createSpreadsheet`: the new id.
pub async fn create(db: &Db, workspace_id: i64, name: String, url: String) -> Result<i64> {
    let spreadsheet_id = resolve_id(db, workspace_id, &url).await?;
    tx::write(db, move |c| {
        let id = new_id();
        let now = now_ms();
        c.execute(
            r#"INSERT INTO "Spreadsheet" ("id", "createdAt", "updatedAt", "workspaceId", "name", "url", "spreadsheetId")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6)"#,
            params![id, now, workspace_id, name, url, spreadsheet_id],
        )
        .map_err(repo::Error::from)?;
        Ok(id)
    })
    .await
}

/// `updateSpreadsheet`.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    name: String,
    url: String,
) -> Result<Spreadsheet> {
    let spreadsheet_id = resolve_id(db, workspace_id, &url).await?;
    tx::write(db, move |c| {
        find_in(c, workspace_id, id)?;
        c.execute(
            r#"UPDATE "Spreadsheet" SET "name" = ?3, "url" = ?4, "spreadsheetId" = ?5, "updatedAt" = ?6
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id, name, url, spreadsheet_id, now_ms()],
        )
        .map_err(repo::Error::from)?;
        find_in(c, workspace_id, id)
    })
    .await
}

/// `deleteMany` (no error for unknown ids).
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    tx::write(db, move |c| {
        c.execute(
            r#"DELETE FROM "Spreadsheet" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2))"#,
            params![workspace_id, id_list(&ids)],
        )
        .map_err(repo::Error::from)?;
        Ok(())
    })
    .await
}

/// `list`: `(rows, pageCount)`; `per_page` `None` = every row, one page.
pub async fn list(
    db: &Db,
    workspace_id: i64,
    name: Option<String>,
    page: i64,
    per_page: Option<i64>,
) -> Result<(Vec<Spreadsheet>, i64)> {
    tx::read(db, move |c| {
        // `ilike %name%` (case-insensitive substring; `instr` needs no escaping).
        let filter =
            r#""workspaceId" = ?1 AND (?2 IS NULL OR instr(lower("name"), lower(?2)) > 0)"#;
        let name = name.filter(|n| !n.is_empty());
        let (limit, offset) = match per_page {
            Some(n) => (n.max(1), (page.max(1) - 1) * n.max(1)),
            None => (-1, 0),
        };
        let rows: Vec<Spreadsheet> = repo::find_many(
            c,
            &format!(r#"WHERE {filter} ORDER BY "id" LIMIT ?3 OFFSET ?4"#),
            &[&workspace_id, &name, &limit, &offset],
        )?;
        let page_count = match per_page {
            Some(n) => {
                let total: i64 = c
                    .query_row(
                        &format!(r#"SELECT count(*) FROM "Spreadsheet" WHERE {filter}"#),
                        params![workspace_id, name],
                        |r| r.get(0),
                    )
                    .map_err(repo::Error::from)?;
                (total + n.max(1) - 1) / n.max(1)
            }
            None => 1,
        };
        Ok((rows, page_count))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreadsheet_ids_come_only_from_parsed_google_urls() {
        let ok = "https://docs.google.com/spreadsheets/d/abc123/edit#gid=0";
        assert_eq!(parse_spreadsheet_id(ok).as_deref(), Some("abc123"));
        for bad in [
            "javascript:fetch(1)//docs.google.com/spreadsheets/d/ID/",
            "https://evil.com/docs.google.com/spreadsheets/d/ID/",
            "http://docs.google.com/spreadsheets/d/ID/",
            "https://docs.google.com/spreadsheets/d/ID",
        ] {
            assert_eq!(parse_spreadsheet_id(bad), None, "{bad}");
        }
    }
}
