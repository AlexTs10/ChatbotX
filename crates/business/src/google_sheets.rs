//! Q.I4: the Google Sheets integration (`integrations/google-sheets`
//! actions and `disconnect`, `integrationGoogleSheetService`). The flow step
//! and the spreadsheet rows that call these are Q.D13.
//!
//! `OAuth2Client` behaviour: the access token is renewed with the refresh
//! token when it expires within 5 minutes, and once more after a 401. The
//! renewed token is kept for the call only (the original does not store it).

use chatbotx_db::Db;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use crate::email_marketing::drip::encode;
use crate::email_marketing::{Http, Outcome, ProviderError, Request, call, open_auth};
use crate::errors::{BusinessError, Result};
use crate::tx;

const SHEETS: &str = "https://sheets.googleapis.com/v4/spreadsheets";
const TOKEN: &str = "https://oauth2.googleapis.com/token";
const REVOKE: &str = "https://oauth2.googleapis.com/revoke";
/// `eagerRefreshThresholdMillis`.
const REFRESH_MARGIN_MS: i64 = 5 * 60 * 1000;

/// The `IntegrationGoogleSheet` row of a workspace.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetRow {
    pub id: i64,
    pub integration_id: i64,
    pub auth: Value,
}

/// `findByWorkspaceId`.
pub async fn find_by_workspace(db: &Db, workspace_id: i64) -> Result<Option<SheetRow>> {
    tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT "id", "integrationId", "auth" FROM "IntegrationGoogleSheet"
               WHERE "workspaceId" = ?1 ORDER BY "id" LIMIT 1"#,
            params![workspace_id],
            |r| {
                let auth: String = r.get(2)?;
                Ok(SheetRow {
                    id: r.get(0)?,
                    integration_id: r.get(1)?,
                    auth: open_auth(&auth).unwrap_or(Value::Null),
                })
            },
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await
}

/// `findByWorkspaceIdOrFail` (a plain `Error`).
pub async fn find_by_workspace_or_fail(db: &Db, workspace_id: i64) -> Result<SheetRow> {
    find_by_workspace(db, workspace_id).await?.ok_or_else(|| {
        BusinessError::Repo(chatbotx_db::repo::Error::msg(
            "Integration Google Sheet not found",
        ))
    })
}

/// `disconnectGoogleSheetsAction` without the audit: revoke the token (a
/// failure is logged only), then delete the `Integration` (the satellite
/// row cascades).
pub async fn disconnect(db: &Db, http: &dyn Http, workspace_id: i64) -> Result<()> {
    let row = find_by_workspace_or_fail(db, workspace_id).await?;
    if let Err(err) = revoke(http, &row.auth).await {
        tracing::error!(workspace_id, err = %err.message, "Unable to disconnect google sheets for workspace: {workspace_id}");
    }
    let integration_id = row.integration_id;
    tx::write(db, move |tx| {
        tx.execute(
            r#"DELETE FROM "Integration" WHERE "id" = ?1"#,
            params![integration_id],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(())
    })
    .await
}

fn token_text<'a>(auth: &'a Value, key: &str) -> &'a str {
    auth["tokens"][key].as_str().unwrap_or_default()
}

fn expiring(auth: &Value, now_ms: i64) -> bool {
    chrono::DateTime::parse_from_rfc3339(token_text(auth, "expiresAt"))
        .is_ok_and(|at| at.timestamp_millis() - now_ms <= REFRESH_MARGIN_MS)
}

fn google_error(payload: Option<&Value>, status: u16) -> String {
    payload
        .and_then(|p| {
            p.pointer("/error/message")
                .or_else(|| p.get("error_description"))
                .or_else(|| p.get("error"))
        })
        .and_then(Value::as_str)
        .map_or_else(
            || format!("Request failed with status code {status}"),
            str::to_owned,
        )
}

/// `refreshAccessToken`.
// ponytail: a JSON body; the token endpoint also takes form data if Google ever drops JSON.
async fn refresh(http: &dyn Http, auth: &Value) -> Outcome<String> {
    let refresh_token = token_text(auth, "refreshToken");
    if refresh_token.is_empty() {
        return Err(ProviderError::new("No refresh token is set."));
    }
    let body = json!({
        "client_id": auth["clientId"],
        "client_secret": auth["clientSecret"],
        "refresh_token": refresh_token,
        "grant_type": "refresh_token",
    });
    let reply = call(http, Request::new("POST", TOKEN).json(body)).await?;
    let payload = reply.payload();
    if !reply.ok() {
        return Err(ProviderError::http(
            google_error(payload.as_ref(), reply.status),
            reply.status,
            &reply,
        ));
    }
    payload
        .as_ref()
        .and_then(|p| p["access_token"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| ProviderError::new("Google OAuth response has no access token"))
}

/// One Sheets API call with the bearer token (renewed as `OAuth2Client`).
async fn sheets(
    http: &dyn Http,
    auth: &Value,
    method: &'static str,
    url: String,
    body: Option<Value>,
) -> Outcome<Value> {
    let mut token = token_text(auth, "accessToken").to_owned();
    let mut refreshed = false;
    if token.is_empty() || expiring(auth, chrono::Utc::now().timestamp_millis()) {
        token = refresh(http, auth).await?;
        refreshed = true;
    }
    loop {
        let mut request =
            Request::new(method, url.clone()).header("Authorization", format!("Bearer {token}"));
        if let Some(body) = &body {
            request = request.json(body.clone());
        }
        let reply = call(http, request).await?;
        if reply.status == 401 && !refreshed {
            token = refresh(http, auth).await?;
            refreshed = true;
            continue;
        }
        let payload = reply.payload();
        if !reply.ok() {
            return Err(ProviderError::http(
                google_error(payload.as_ref(), reply.status),
                reply.status,
                &reply,
            ));
        }
        return Ok(payload.unwrap_or(Value::Null));
    }
}

fn strings(row: &Value) -> Vec<String> {
    row.as_array()
        .into_iter()
        .flatten()
        .map(|cell| {
            cell.as_str()
                .map_or_else(|| cell.to_string(), str::to_owned)
        })
        .collect()
}

fn values_url(spreadsheet_id: &str, range: &str, suffix: &str) -> String {
    format!(
        "{SHEETS}/{}/values/{}{suffix}",
        encode(spreadsheet_id),
        encode(range)
    )
}

/// `listSheetNames`.
pub async fn list_sheet_names(
    http: &dyn Http,
    auth: &Value,
    spreadsheet_id: &str,
) -> Outcome<Vec<String>> {
    let data = sheets(
        http,
        auth,
        "GET",
        format!("{SHEETS}/{}", encode(spreadsheet_id)),
        None,
    )
    .await?;
    Ok(data["sheets"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|s| {
            s.pointer("/properties/title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        })
        .collect())
}

/// `listSheetHeaders`: row 1.
pub async fn list_sheet_headers(
    http: &dyn Http,
    auth: &Value,
    spreadsheet_id: &str,
    sheet: &str,
) -> Outcome<Vec<String>> {
    let data = sheets(
        http,
        auth,
        "GET",
        values_url(spreadsheet_id, &format!("{sheet}!1:1"), ""),
        None,
    )
    .await?;
    Ok(data["values"].get(0).map(strings).unwrap_or_default())
}

/// `getSheetValues`.
pub async fn get_sheet_values(
    http: &dyn Http,
    auth: &Value,
    spreadsheet_id: &str,
    sheet: &str,
) -> Outcome<Vec<Vec<String>>> {
    let data = sheets(
        http,
        auth,
        "GET",
        values_url(spreadsheet_id, sheet, ""),
        None,
    )
    .await?;
    Ok(data["values"]
        .as_array()
        .into_iter()
        .flatten()
        .map(strings)
        .collect())
}

/// `insertRow` (`RAW`, `INSERT_ROWS`).
pub async fn insert_row(
    http: &dyn Http,
    auth: &Value,
    spreadsheet_id: &str,
    sheet: &str,
    data: &[String],
) -> Outcome<()> {
    let url = values_url(
        spreadsheet_id,
        sheet,
        ":append?valueInputOption=RAW&insertDataOption=INSERT_ROWS",
    );
    sheets(http, auth, "POST", url, Some(json!({ "values": [data] })))
        .await
        .map(|_| ())
}

/// `updateRow`: `<sheet>!A<rowIndex + 1>`.
pub async fn update_row(
    http: &dyn Http,
    auth: &Value,
    spreadsheet_id: &str,
    sheet: &str,
    row_index: u32,
    data: &[String],
) -> Outcome<()> {
    let range = format!("{sheet}!A{}", u64::from(row_index) + 1);
    let url = values_url(spreadsheet_id, &range, "?valueInputOption=RAW");
    sheets(http, auth, "PUT", url, Some(json!({ "values": [data] })))
        .await
        .map(|_| ())
}

/// `clearRow`: `<sheet>!A<n>:Z<n>`.
pub async fn clear_row(
    http: &dyn Http,
    auth: &Value,
    spreadsheet_id: &str,
    sheet: &str,
    row_index: u32,
) -> Outcome<()> {
    let n = u64::from(row_index) + 1;
    let url = values_url(spreadsheet_id, &format!("{sheet}!A{n}:Z{n}"), ":clear");
    sheets(http, auth, "POST", url, Some(json!({})))
        .await
        .map(|_| ())
}

/// `revokeToken`.
pub async fn revoke(http: &dyn Http, auth: &Value) -> Outcome<()> {
    let url = format!("{REVOKE}?token={}", encode(token_text(auth, "accessToken")));
    let reply = call(http, Request::new("POST", url)).await?;
    if reply.ok() {
        return Ok(());
    }
    Err(ProviderError::http(
        google_error(reply.payload().as_ref(), reply.status),
        reply.status,
        &reply,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::email_marketing::test_http::FakeHttp;

    fn auth(expires_at: &str) -> Value {
        json!({
            "clientId": "cid", "clientSecret": "cs",
            "tokens": { "accessToken": "old", "refreshToken": "r1", "expiresAt": expires_at },
        })
    }

    #[test]
    fn expiry_margin() {
        let now = chrono::Utc::now().timestamp_millis();
        let soon = chrono::DateTime::from_timestamp_millis(now + 60_000)
            .unwrap()
            .to_rfc3339();
        let later = chrono::DateTime::from_timestamp_millis(now + 3_600_000)
            .unwrap()
            .to_rfc3339();
        assert!(expiring(&auth(&soon), now));
        assert!(!expiring(&auth(&later), now));
        assert!(!expiring(&auth(""), now));
    }

    #[tokio::test]
    async fn retries_once_after_401_with_a_fresh_token() {
        let http = FakeHttp::with(vec![
            (401, r#"{ "error": { "message": "expired" } }"#),
            (200, r#"{ "access_token": "new" }"#),
            (
                200,
                r#"{ "sheets": [{ "properties": { "title": "Leads" } }, {}] }"#,
            ),
        ]);
        let names = list_sheet_names(&http, &auth(""), "sheet/1").await.unwrap();
        assert_eq!(names, vec!["Leads".to_owned(), String::new()]);
        let sent = http.requests();
        assert_eq!(sent[0].url, format!("{SHEETS}/sheet%2F1"));
        assert_eq!(sent[1].url, TOKEN);
        assert!(
            sent[2]
                .headers
                .iter()
                .any(|(k, v)| *k == "Authorization" && v == "Bearer new")
        );
    }

    #[tokio::test]
    async fn builds_row_ranges() {
        let http = FakeHttp::with(vec![(200, "{}"), (200, "{}")]);
        let later = chrono::DateTime::from_timestamp_millis(
            chrono::Utc::now().timestamp_millis() + 3_600_000,
        )
        .unwrap()
        .to_rfc3339();
        update_row(&http, &auth(&later), "s", "Leads", 2, &["a".into()])
            .await
            .unwrap();
        clear_row(&http, &auth(&later), "s", "Leads", 2)
            .await
            .unwrap();
        let sent = http.requests();
        assert_eq!(
            sent[0].url,
            format!("{SHEETS}/s/values/Leads%21A3?valueInputOption=RAW")
        );
        assert_eq!(sent[0].body, Some(json!({ "values": [["a"]] })));
        assert_eq!(
            sent[1].url,
            format!("{SHEETS}/s/values/Leads%21A3%3AZ3:clear")
        );
    }
}
