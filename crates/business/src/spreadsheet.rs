//! `packages/business/src/spreadsheet/service.ts` and the URL part of
//! `features/spreadsheets/lib/{google-sheets,manage-spreadsheet}.ts`.
//!
//! Create / update check the integration row, the URL and that Google can
//! read the spreadsheet (`listSheetNames`); `listWorksheets` and
//! `listWorksheetHeaders` read it (`features/spreadsheets/lib/google-sheets.ts`).

use chatbotx_db::Db;
use chatbotx_db::repo::models::Spreadsheet;
use chatbotx_db::repo::{self, id_list, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::Value;

use crate::email_marketing::Http;
use crate::errors::{BusinessError, Result};
use crate::{google_sheets, tx};

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

/// `resolveSpreadsheetIdFromUrl`: the integration, the URL, then one
/// `listSheetNames` read (any failure is the URL error).
async fn resolve_id(db: &Db, http: &dyn Http, workspace_id: i64, url: &str) -> Result<String> {
    let Some(row) = google_sheets::find_by_workspace(db, workspace_id).await? else {
        return Err(BusinessError::validation("url", INTEGRATION_MISSING));
    };
    let id =
        parse_spreadsheet_id(url).ok_or_else(|| BusinessError::validation("url", INVALID_URL))?;
    if let Err(err) = google_sheets::list_sheet_names(http, &row.auth, &id).await {
        tracing::error!(workspace_id, err = %err.message, "Unable to get data from google sheets");
        return Err(BusinessError::validation("url", INVALID_URL));
    }
    Ok(id)
}

/// A Sheets call failure of a read (the original rethrows: a 500).
fn provider(err: crate::email_marketing::ProviderError) -> BusinessError {
    BusinessError::Repo(repo::Error::msg(err.message))
}

/// `listWorksheets`: the sheet tab names.
pub async fn list_worksheets(
    db: &Db,
    http: &dyn Http,
    workspace_id: i64,
    id: i64,
) -> Result<Vec<String>> {
    let spreadsheet = find_or_fail(db, workspace_id, id).await?;
    let row = google_sheets::find_by_workspace_or_fail(db, workspace_id).await?;
    google_sheets::list_sheet_names(http, &row.auth, &spreadsheet.spreadsheet_id)
        .await
        .map_err(provider)
}

/// `listWorksheetHeaders`: the first row of a sheet.
pub async fn list_worksheet_headers(
    db: &Db,
    http: &dyn Http,
    workspace_id: i64,
    id: i64,
    sheet_name: &str,
) -> Result<Vec<String>> {
    let spreadsheet = find_or_fail(db, workspace_id, id).await?;
    let row = google_sheets::find_by_workspace_or_fail(db, workspace_id).await?;
    google_sheets::list_sheet_headers(http, &row.auth, &spreadsheet.spreadsheet_id, sheet_name)
        .await
        .map_err(provider)
}

/// `createSpreadsheet`: the new id.
pub async fn create(
    db: &Db,
    http: &dyn Http,
    workspace_id: i64,
    name: String,
    url: String,
) -> Result<i64> {
    let spreadsheet_id = resolve_id(db, http, workspace_id, &url).await?;
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
    http: &dyn Http,
    workspace_id: i64,
    id: i64,
    name: String,
    url: String,
) -> Result<Spreadsheet> {
    let spreadsheet_id = resolve_id(db, http, workspace_id, &url).await?;
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
