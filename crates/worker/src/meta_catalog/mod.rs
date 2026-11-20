//! `apps/worker/src/default/handlers/meta-catalog/*`, the Graph calls of
//! `integrations/meta-catalog` and the worker half of
//! `meta-catalog/{integration,sync-run,import}.service.ts`.

pub mod import;
pub mod items;
pub mod sync;

use std::sync::{LazyLock, OnceLock};
use std::time::Duration;

use chatbotx_business::BusinessError;
pub use chatbotx_business::errors::{is_query_dump, sanitize_public_text};
use chatbotx_business::meta_catalog::{DEFAULT_API_VERSION, GRAPH_API_URL, StoredAuth};
use chatbotx_db::repo::models::MetaCatalogSyncRun;
use chatbotx_db::repo::{self, now_ms};
use chatbotx_db::rusqlite::{Connection, params};
use regex::Regex;
use serde_json::{Value, json};

const GENERIC_SYNC_FAILURE: &str = "Meta Catalog sync failed. Please try again or contact support.";
const GENERIC_IMPORT_FAILURE: &str =
    "Meta Catalog import failed. Please try again or contact support.";
const MAX_DIAGNOSTIC_ITEMS: usize = 50;

/// A thrown value, as `toPublicErrorMessage` tells them apart.
#[derive(Debug, Clone, PartialEq)]
pub enum McError {
    /// `MetaCatalogException` (an `SdkException`).
    Meta {
        message: String,
        status: u16,
        graph_code: Option<i64>,
        graph_subcode: Option<i64>,
        fbtrace_id: Option<String>,
    },
    /// `ChatbotXException`: its message is shown.
    Public(String),
    /// Any other `Error`: only the fallback is shown.
    Internal(String),
}

impl McError {
    pub fn internal(message: impl Into<String>) -> McError {
        McError::Internal(message.into())
    }

    fn graph_code(&self) -> Option<i64> {
        match self {
            McError::Meta { graph_code, .. } => *graph_code,
            _ => None,
        }
    }

    /// `isInvalidMetaTokenError`.
    pub fn is_invalid_token(&self) -> bool {
        self.graph_code() == Some(190)
    }

    /// `isDefiniteMetaRequestRejection`.
    pub fn is_definite_rejection(&self) -> bool {
        matches!(self, McError::Meta { status: 400, .. }) && self.graph_code() == Some(100)
    }

    /// `error instanceof Error ? error.message : ...`.
    pub fn message(&self) -> &str {
        match self {
            McError::Meta { message, .. }
            | McError::Public(message)
            | McError::Internal(message) => message,
        }
    }
}

impl From<BusinessError> for McError {
    fn from(error: BusinessError) -> McError {
        match error {
            BusinessError::Exception { message, .. } => McError::Public(message),
            BusinessError::Repo(error) => McError::Internal(error.message()),
        }
    }
}

impl From<repo::Error> for McError {
    fn from(error: repo::Error) -> McError {
        McError::Internal(error.message())
    }
}

impl From<chatbotx_db::DbError> for McError {
    fn from(error: chatbotx_db::DbError) -> McError {
        McError::Internal(error.to_string())
    }
}

impl From<chatbotx_db::rusqlite::Error> for McError {
    fn from(error: chatbotx_db::rusqlite::Error) -> McError {
        McError::Internal(error.to_string())
    }
}

pub type McResult<T> = Result<T, McError>;

// ---- errors.ts: sanitizePublicText / toPublicErrorMessage --------------------

/// `toPublicErrorMessage(error, fallback)`.
pub fn to_public_error_message(error: &McError, fallback: &str) -> String {
    let message = match error {
        McError::Meta {
            message,
            graph_code,
            ..
        } => {
            let text = message.trim().to_owned();
            if text.is_empty() {
                return fallback.to_owned();
            }
            // `SdkException.code`: the Graph code, else "metaCatalogError".
            let code = graph_code.map_or("metaCatalogError".to_owned(), |c| c.to_string());
            if graph_code != &Some(-1) && !text.contains(&code) {
                format!("{text} (code {code})")
            } else {
                text
            }
        }
        McError::Public(message) => message.clone(),
        McError::Internal(_) => return fallback.to_owned(),
    };
    if message.is_empty() || is_query_dump(&message) {
        return fallback.to_owned();
    }
    let sanitized = sanitize_public_text(&message);
    if sanitized.is_empty() {
        fallback.to_owned()
    } else {
        sanitized
    }
}

/// `toPublicMetaCatalog{,Import}Message`: a plain sentence.
fn public_text(message: &str, fallback: &str) -> String {
    to_public_error_message(&McError::Public(message.to_owned()), fallback)
}

static SAFE_FB_TRACE_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]{1,128}$").unwrap());

/// `safeMetaCatalogErrorLog`: `(message, details)`.
pub fn safe_error_log(error: &McError, fallback: &str) -> (String, Value) {
    let message = to_public_error_message(error, fallback);
    let mut details = json!({ "error": message });
    match error {
        McError::Meta {
            status,
            graph_code,
            graph_subcode,
            fbtrace_id,
            ..
        } => {
            details["err"] = json!({ "name": "MetaCatalogException", "message": message });
            details["errorType"] = json!("MetaCatalogException");
            if let Some(code) = graph_code {
                details["graphCode"] = json!(code);
            }
            if let Some(code) = graph_subcode {
                details["graphSubcode"] = json!(code);
            }
            if let Some(id) = fbtrace_id
                .as_deref()
                .filter(|id| SAFE_FB_TRACE_ID.is_match(id))
            {
                details["fbTraceId"] = json!(id);
            }
            details["statusCode"] = json!(status);
        }
        _ => details["errorType"] = json!("Error"),
    }
    (message, details)
}

fn log_failure(error: &McError, fallback: &str, context: Value) {
    let (message, details) = safe_error_log(error, fallback);
    tracing::error!(details = %details, context = %context, "{message}");
}

// ---- lib/http-client.ts --------------------------------------------------------

fn client() -> McResult<&'static reqwest::Client> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| McError::internal(e.clone()))
}

const RETRY_STATUSES: &[u16] = &[408, 429, 500, 502, 503, 504];

/// One Graph call: `(body, x-business-use-case-usage)`. GET retries 3 times
/// on a network error or a retry status (ky: `backoffLimit` 1 s).
async fn graph(
    auth: &StoredAuth,
    path: &str,
    query: &[(&str, &str)],
    form: Option<&[(&str, String)]>,
) -> McResult<(Value, Option<String>)> {
    let version = auth.version.as_deref().unwrap_or(DEFAULT_API_VERSION);
    let relative = format!("{version}/{path}");
    let mut url = url::Url::parse(&format!("{GRAPH_API_URL}/{relative}"))
        .map_err(|e| McError::internal(e.to_string()))?;
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    let client = client()?;
    let mut attempt = 0u32;
    loop {
        let request = match form {
            Some(fields) => client
                .post(url.as_str())
                .header("content-type", "application/x-www-form-urlencoded")
                .body(
                    url::form_urlencoded::Serializer::new(String::new())
                        .extend_pairs(fields)
                        .finish(),
                ),
            None => client.get(url.as_str()),
        };
        let retry = form.is_none() && attempt < 3;
        let response = match request.bearer_auth(&auth.access_token).send().await {
            Ok(response) => response,
            Err(_) if retry => {
                attempt += 1;
                tokio::time::sleep(backoff(attempt)).await;
                continue;
            }
            Err(error) => return Err(McError::internal(error.to_string())),
        };
        let status = response.status().as_u16();
        if retry && RETRY_STATUSES.contains(&status) {
            attempt += 1;
            tokio::time::sleep(backoff(attempt)).await;
            continue;
        }
        let usage = response
            .headers()
            .get("x-business-use-case-usage")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = response
            .bytes()
            .await
            .map_err(|e| McError::internal(e.to_string()))?;
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            let error = body.get("error");
            let field = |key: &str| error.and_then(|e| e.get(key));
            return Err(McError::Meta {
                message: chatbotx_core::utils::graph_error::format_graph_error(error)
                    .unwrap_or_else(|| format!("Meta Graph request failed: {relative}")),
                status,
                graph_code: field("code").and_then(Value::as_i64),
                graph_subcode: field("error_subcode").and_then(Value::as_i64),
                fbtrace_id: field("fbtrace_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
        return Ok((body, usage));
    }
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis((300u64 << attempt.min(4)).min(1000))
}

// ---- outcome-status.ts / sync-run + integration services ---------------------

/// `resolveMetaCatalogOutcome`.
pub fn outcome(succeeded: i64, failed: i64, skipped: i64) -> &'static str {
    if failed == 0 && skipped == 0 {
        "succeeded"
    } else if succeeded > 0 {
        "partial"
    } else {
        "failed"
    }
}

fn first_run(rows: repo::Result<Vec<MetaCatalogSyncRun>>) -> McResult<Option<MetaCatalogSyncRun>> {
    Ok(rows?.into_iter().next())
}

/// `metaCatalogSyncRunService.claim` / `claimSubmission` (with a lease).
fn claim_run(
    conn: &Connection,
    run_id: i64,
    workspace_id: i64,
    lease: Option<&str>,
) -> McResult<Option<MetaCatalogSyncRun>> {
    let now = now_ms();
    first_run(repo::returning(
        conn,
        r#"UPDATE "MetaCatalogSyncRun" SET "status" = 'running', "startedAt" = ?3, "updatedAt" = ?3,
             "submissionLeaseId" = coalesce(?4, "submissionLeaseId")
           WHERE "id" = ?1 AND "workspaceId" = ?2 AND "status" = 'queued'"#,
        &[&run_id, &workspace_id, &now, &lease],
    ))
}

/// `logProviderError({ provider: "meta-catalog", workspaceId, error })`: the
/// raw error (`ErrorLog.detail` is stored unredacted past the secret scrub,
/// as the original); never fails.
async fn log_provider_error(deps: &crate::Deps, workspace_id: i64, error: &McError) {
    let error = match error {
        McError::Meta {
            message, status, ..
        } => json!({ "message": message, "statusCode": status }),
        other => json!({ "message": other.message() }),
    };
    crate::events::log_provider_errors(
        deps.events.as_ref(),
        &[crate::events::ProviderError {
            provider: "meta-catalog".into(),
            workspace_id: workspace_id.to_string(),
            error,
            ..Default::default()
        }],
    )
    .await;
}

/// `metaCatalogSyncRunService.fail(runId, error)`.
async fn fail_run(db: &chatbotx_db::Db, run_id: i64, error: &McError) -> McResult<()> {
    let message = to_public_error_message(error, GENERIC_SYNC_FAILURE);
    db.write(move |tx| {
        let now = now_ms();
        tx.execute(
            r#"UPDATE "MetaCatalogSyncRun" SET "status" = 'failed', "error" = ?2, "finishedAt" = ?3, "updatedAt" = ?3
               WHERE "id" = ?1 AND "status" IN ('queued', 'running')"#,
            params![run_id, message, now],
        )?;
        Ok(())
    })
    .await?;
    Ok(())
}

/// `integrationMetaCatalogService.markInvalid`.
async fn mark_invalid(db: &chatbotx_db::Db, workspace_id: i64) -> McResult<()> {
    db.write(move |tx| {
        tx.execute(
            r#"UPDATE "IntegrationMetaCatalog" SET "status" = 'invalid', "updatedAt" = ?2
               WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL"#,
            params![workspace_id, now_ms()],
        )?;
        Ok(())
    })
    .await?;
    Ok(())
}

/// `sanitizeItemErrors`.
fn sanitize_item_errors(errors: &[Value]) -> Vec<Value> {
    errors
        .iter()
        .map(|item| {
            let mut item = item.clone();
            let reason = item
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            item["reason"] = json!(public_text(&reason, GENERIC_SYNC_FAILURE));
            item
        })
        .collect()
}

fn capped(items: &[Value]) -> String {
    Value::Array(items.iter().take(MAX_DIAGNOSTIC_ITEMS).cloned().collect()).to_string()
}

fn workspace_name(
    conn: &Connection,
    workspace_id: i64,
) -> chatbotx_db::rusqlite::Result<Option<String>> {
    use chatbotx_db::rusqlite::OptionalExtension as _;
    conn.query_row(
        r#"SELECT "name" FROM "Workspace" WHERE "id" = ?1"#,
        [workspace_id],
        |r| r.get(0),
    )
    .optional()
}

#[cfg(test)]
mod tests {
    //! `safe-error-log.test.ts`.
    use super::*;

    /// The `logProviderError` call of `import-products.ts` / `submit.ts` /
    /// `check.ts`: one `error-log:recorded` for `meta-catalog`, Graph status
    /// as the HTTP code.
    #[tokio::test]
    async fn a_failure_is_written_to_the_provider_error_log() {
        let (mut deps, _) = crate::inbound::tests::setup().await;
        let emitted = std::sync::Arc::new(crate::inbound::tests::Emitted::default());
        deps.events = Some(chatbotx_jobs::events::EventBuses {
            message: emitted.clone(),
            flow: emitted.clone(),
            dashboard: emitted.clone(),
            error_log: emitted.clone(),
        });
        let error = McError::Meta {
            message: "Invalid OAuth access token".into(),
            status: 401,
            graph_code: Some(190),
            graph_subcode: None,
            fbtrace_id: None,
        };
        log_provider_error(&deps, 7, &error).await;
        log_provider_error(&deps, 7, &McError::internal("boom")).await;
        let events = emitted.0.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, "error-log:recorded");
        let (first, second) = (&events[0].1, &events[1].1);
        assert_eq!(
            (&first["provider"], &first["workspaceId"]),
            (&json!("meta-catalog"), &json!("7"))
        );
        assert_eq!(
            first["error"],
            json!({ "message": "Invalid OAuth access token", "httpCode": "401" })
        );
        assert_eq!(
            second["error"],
            json!({ "message": "boom", "httpCode": null })
        );
    }

    #[test]
    fn redacts_upstream_credentials_while_retaining_non_secret_graph_metadata() {
        let error = McError::Meta {
            message: "Rejected {\"access_token\":\"SECRET\"}\nclient_secret=OTHER authorization: Basic dXNlcjpwYXNz".into(),
            status: 401,
            graph_code: Some(190),
            graph_subcode: Some(463),
            fbtrace_id: Some("trace-1".into()),
        };
        let expected = r#"Rejected {"access_token":"[REDACTED]"} client_secret=[REDACTED] Authorization: [REDACTED] (code 190)"#;
        let (message, details) = safe_error_log(&error, "Meta request failed");
        assert_eq!(message, expected);
        assert_eq!(
            details,
            json!({
                "err": { "message": expected, "name": "MetaCatalogException" },
                "error": expected,
                "errorType": "MetaCatalogException",
                "fbTraceId": "trace-1",
                "graphCode": 190,
                "graphSubcode": 463,
                "statusCode": 401,
            })
        );
        // The original also checks `err.stack`; the Rust log has no stack.
        for secret in ["SECRET", "OTHER", "dXNlcjpwYXNz"] {
            assert!(!details.to_string().contains(secret));
        }
    }

    #[test]
    fn does_not_log_an_untrusted_error_message_or_stack() {
        let error = McError::internal("postgres://admin:password@database.internal");
        assert_eq!(
            safe_error_log(&error, "Meta request failed"),
            (
                "Meta request failed".to_owned(),
                json!({ "error": "Meta request failed", "errorType": "Error" })
            )
        );
    }

    #[test]
    fn keeps_a_public_message_and_redacts_bearer_and_query_tokens() {
        assert_eq!(
            to_public_error_message(
                &McError::Public("Bad Bearer abc.def at https://x/?access_token=zz&a=1".into()),
                "fallback"
            ),
            "Bad Bearer [REDACTED] at https://x/?access_token=[REDACTED]&a=1"
        );
        assert_eq!(
            to_public_error_message(&McError::Public("failed query: select".into()), "fallback"),
            "fallback"
        );
    }

    #[test]
    fn resolves_the_outcome() {
        assert_eq!(outcome(0, 0, 0), "succeeded");
        assert_eq!(outcome(2, 1, 0), "partial");
        assert_eq!(outcome(0, 0, 1), "failed");
    }
}
