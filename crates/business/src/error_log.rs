//! `packages/business/src/error-log/service.ts` (Q.D11 slice 4): the error
//! log table of a workspace (`listErrorLogs`, `deleteErrorLogs`) and the
//! provider-failure writer (`logProviderError*`), which emits
//! `error-log:recorded` onto the error-log bus (the worker writes the row).

use std::time::Duration;

use chatbotx_core::js::slice_utf16;
use chatbotx_core::logger::scrub_secrets_in_string;
use chatbotx_core::utils::error_log::{
    MAX_STACK_LENGTH, error_log_providers_matching_label, is_error_log_provider,
    resolve_stack_frames,
};
use chatbotx_db::Db;
use chatbotx_db::repo::models::{Contact, ErrorLog};
use chatbotx_db::repo::utils::{order_by_sql, parse_order_by};
use chatbotx_db::repo::{
    Model, PaginationInput, SortItem, get_pagination_with_defaults, like_contains,
};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{OptionalExtension as _, params_from_iter};
use chatbotx_jobs::events::emit::{EventBuses, emit};
use serde_json::{Map, Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `WITHHELD_ERROR_LOG_COLUMNS`: developer-only columns, never listed.
pub const WITHHELD_ERROR_LOG_COLUMNS: &[&str] = &["stackTrace", "sourceId"];

/// `SORTABLE_COLUMNS`.
pub const SORTABLE_COLUMNS: &[&str] = &["id", "action", "detail", "createdAt", "updatedAt"];

/// One OR term of the keyword search (the drizzle `where.OR` entries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchTerm {
    /// `{ action: { ilike } }`
    ActionIlike(String),
    /// `{ detail: { ilike } }`
    DetailIlike(String),
    /// `{ action: { in: providers } }`: the providers whose label matches.
    ActionIn(Vec<&'static str>),
}

/// The keyword search of `listErrorLogs`: empty without a keyword. The
/// channel-side `sourceId` is never searched.
pub fn search_terms(keyword: Option<&str>) -> Vec<SearchTerm> {
    let Some(keyword) = keyword.filter(|k| !k.is_empty()) else {
        return Vec::new();
    };
    let mut terms = vec![
        SearchTerm::ActionIlike(like_contains(keyword)),
        SearchTerm::DetailIlike(like_contains(keyword)),
    ];
    let providers = error_log_providers_matching_label(keyword);
    if !providers.is_empty() {
        terms.push(SearchTerm::ActionIn(providers));
    }
    terms
}

/// Input of [`list`].
#[derive(Debug, Clone, Default)]
pub struct ListInput {
    pub workspace_id: i64,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
    pub keyword: Option<String>,
}

/// One listed row: the log (withheld columns cleared) and its contact.
#[derive(Debug, Clone)]
pub struct ErrorLogRow {
    pub log: ErrorLog,
    pub contact: Option<Contact>,
    /// `contact.conversation.id`.
    pub conversation_id: Option<i64>,
}

/// `{ data, pageCount }`.
#[derive(Debug, Clone)]
pub struct ErrorLogPage {
    pub data: Vec<ErrorLogRow>,
    pub page_count: i64,
}

/// `listErrorLogs`: default order `createdAt desc`.
pub async fn list(db: &Db, input: ListInput) -> Result<ErrorLogPage> {
    tx::read(db, move |conn| {
        let mut filter = String::from(r#""workspaceId" = ?"#);
        let mut binds = vec![Sql::Integer(input.workspace_id)];
        let terms = search_terms(input.keyword.as_deref());
        if !terms.is_empty() {
            let mut parts = Vec::new();
            for term in terms {
                match term {
                    SearchTerm::ActionIlike(p) => {
                        parts.push(r#"ilike("action", ?)"#.to_string());
                        binds.push(Sql::Text(p));
                    }
                    SearchTerm::DetailIlike(p) => {
                        parts.push(r#"ilike("detail", ?)"#.to_string());
                        binds.push(Sql::Text(p));
                    }
                    SearchTerm::ActionIn(providers) => {
                        let marks = vec!["?"; providers.len()].join(", ");
                        parts.push(format!(r#""action" IN ({marks})"#));
                        binds.extend(providers.iter().map(|p| Sql::Text((*p).to_string())));
                    }
                }
            }
            filter.push_str(&format!(" AND ({})", parts.join(" OR ")));
        }
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: input.per_page,
        });
        let mut order = parse_order_by(
            ErrorLog::COLUMNS,
            input.sort.as_deref(),
            Some(SORTABLE_COLUMNS),
        );
        if order.is_empty() {
            order.push((
                "createdAt".to_string(),
                chatbotx_db::repo::utils::SortDir::Desc,
            ));
        }
        let total: i64 = conn
            .query_row(
                &format!(r#"SELECT COUNT(*) FROM "ErrorLog" WHERE {filter}"#),
                params_from_iter(binds.iter()),
                |row| row.get(0),
            )
            .map_err(repo_err)?;
        let mut page_binds = binds.clone();
        page_binds.push(Sql::Integer(pagination.limit));
        page_binds.push(Sql::Integer(pagination.offset));
        let logs = conn
            .prepare(&format!(
                "{} WHERE {filter} {} LIMIT ? OFFSET ?",
                ErrorLog::select_sql(),
                order_by_sql(&order, None)
            ))
            .map_err(repo_err)?
            .query_map(params_from_iter(page_binds.iter()), ErrorLog::from_row)
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        let mut data = Vec::with_capacity(logs.len());
        for mut log in logs {
            log.source_id = None;
            log.stack_trace = None;
            let contact = match log.contact_id {
                Some(id) => conn
                    .query_row(
                        &format!(r#"{} WHERE "id" = ?1"#, Contact::select_sql()),
                        [id],
                        Contact::from_row,
                    )
                    .optional()
                    .map_err(repo_err)?,
                None => None,
            };
            let conversation_id = match &contact {
                Some(c) => conn
                    .query_row(
                        r#"SELECT "id" FROM "Conversation" WHERE "contactId" = ?1 LIMIT 1"#,
                        [c.id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(repo_err)?,
                None => None,
            };
            data.push(ErrorLogRow {
                log,
                contact,
                conversation_id,
            });
        }
        let limit = pagination.limit.max(1);
        Ok(ErrorLogPage {
            data,
            page_count: (total + limit - 1) / limit,
        })
    })
    .await
}

/// `deleteErrorLogs`: deletes the ids of one workspace; no query for an
/// empty list. Returns the number of deleted rows.
pub async fn delete(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    tx::write(db, move |conn| {
        let marks = vec!["?"; ids.len()].join(", ");
        let mut binds = vec![Sql::Integer(workspace_id)];
        binds.extend(ids.into_iter().map(Sql::Integer));
        conn.execute(
            &format!(r#"DELETE FROM "ErrorLog" WHERE "workspaceId" = ? AND "id" IN ({marks})"#),
            params_from_iter(binds.iter()),
        )
        .map_err(repo_err)
    })
    .await
}

/// The thrown value of a provider failure (the TS `error: unknown`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Thrown {
    /// An `Error` (an `SdkException` carries `status`).
    Error {
        message: String,
        stack: Option<String>,
        status: Option<i64>,
    },
    /// A thrown string.
    Text(String),
    /// A plain object (a `ParsedError` off a stream): `message`, `statusCode`.
    Object {
        message: Option<String>,
        status: Option<i64>,
    },
    /// `undefined` / `null`.
    Nothing,
}

/// `LogProviderErrorInput`.
#[derive(Debug, Clone)]
pub struct ProviderErrorInput {
    /// Written verbatim to `ErrorLog.action` (an `errorLogProviders` value).
    pub provider: String,
    pub workspace_id: String,
    pub contact_id: Option<String>,
    pub source_id: Option<String>,
    pub error: Thrown,
    /// `Some` overrides the status derived from `error` (`Some(None)` = null).
    pub http_code: Option<Option<String>>,
    /// `Some` overrides the frames derived from `error` (`Some(None)` = none).
    pub stack_trace: Option<Option<String>>,
}

impl ProviderErrorInput {
    /// An input with no ids, overrides or contact.
    pub fn new(provider: &str, workspace_id: &str, error: Thrown) -> Self {
        Self {
            provider: provider.to_string(),
            workspace_id: workspace_id.to_string(),
            contact_id: None,
            source_id: None,
            error,
            http_code: None,
            stack_trace: None,
        }
    }
}

/// `MAX_DETAIL_LENGTH` (UTF-16 units).
pub const MAX_DETAIL_LENGTH: usize = 8192;

/// `UNKNOWN_ERROR_DETAIL`.
pub const UNKNOWN_ERROR_DETAIL: &str = "An unknown error occurred";

/// `EMIT_TIMEOUT_MS`.
pub const EMIT_TIMEOUT: Duration = Duration::from_millis(2000);

fn truncate(value: &str, max: usize) -> String {
    slice_utf16(value, 0, max).to_string()
}

fn resolve_message(error: &Thrown) -> String {
    match error {
        Thrown::Error { message, .. } | Thrown::Text(message) => message.clone(),
        Thrown::Object {
            message: Some(m), ..
        } if !m.is_empty() => m.clone(),
        Thrown::Object { .. } | Thrown::Nothing => UNKNOWN_ERROR_DETAIL.to_string(),
    }
}

fn resolve_http_code(input: &ProviderErrorInput) -> Option<String> {
    if let Some(code) = &input.http_code {
        return code.clone();
    }
    let status = match &input.error {
        Thrown::Error { status, .. } | Thrown::Object { status, .. } => *status,
        _ => None,
    };
    status.filter(|s| *s > 0).map(|s| s.to_string())
}

fn resolve_stack_trace(input: &ProviderErrorInput) -> Option<String> {
    if let Some(stack) = &input.stack_trace {
        return stack.as_deref().map(|s| truncate(s, MAX_STACK_LENGTH));
    }
    match &input.error {
        Thrown::Error { message, stack, .. } => resolve_stack_frames(message, stack.as_deref()),
        _ => None,
    }
}

/// `toEntry`: the `error-log:recorded` payload (a fresh row id; secrets
/// scrubbed from message and stack; absent ids leave their key out).
pub fn to_entry(input: &ProviderErrorInput) -> Value {
    let mut error = Map::new();
    error.insert(
        "message".into(),
        json!(truncate(
            &scrub_secrets_in_string(&resolve_message(&input.error)),
            MAX_DETAIL_LENGTH
        )),
    );
    error.insert("httpCode".into(), json!(resolve_http_code(input)));
    if let Some(stack) = resolve_stack_trace(input) {
        error.insert("stackTrace".into(), json!(scrub_secrets_in_string(&stack)));
    }
    let mut entry = Map::new();
    entry.insert("id".into(), json!(chatbotx_core::utils::id::create_id()));
    entry.insert("workspaceId".into(), json!(input.workspace_id));
    entry.insert("provider".into(), json!(input.provider));
    if let Some(id) = &input.contact_id {
        entry.insert("contactId".into(), json!(id));
    }
    if let Some(id) = &input.source_id {
        entry.insert("sourceId".into(), json!(id));
    }
    entry.insert("error".into(), Value::Object(error));
    Value::Object(entry)
}

/// `logProviderErrors`: one event per failure, emitted concurrently. Never
/// fails; returns the indexes that could not be emitted (rejected, timed
/// out, or a synchronous routing failure). A schema rejection (`""`) is
/// dropped, not reported. `None` buses (no Redis) = nothing emitted.
pub async fn log_provider_errors(
    buses: Option<&EventBuses>,
    inputs: &[ProviderErrorInput],
) -> Vec<usize> {
    let Some(buses) = buses else {
        return Vec::new();
    };
    let sends = inputs.iter().map(|input| async move {
        match emit(buses, "error-log:recorded", to_entry(input)) {
            None => Err("routing"),
            Some(future) => match tokio::time::timeout(EMIT_TIMEOUT, future).await {
                Err(_) => Err("timeout"),
                Ok(Err(_)) => Err("rejected"),
                Ok(Ok(id)) => Ok(id),
            },
        }
    });
    let results = futures::future::join_all(sends).await;
    let mut failed = Vec::new();
    for (index, result) in results.into_iter().enumerate() {
        match result {
            Err(reason) => {
                tracing::warn!(
                    workspace_id = %inputs[index].workspace_id,
                    reason,
                    "logProviderErrors: failed to emit error log"
                );
                failed.push(index);
            }
            Ok(id) if id.is_empty() => tracing::warn!(
                workspace_id = %inputs[index].workspace_id,
                "logProviderErrors: error log rejected by schema, dropping"
            ),
            Ok(_) => {}
        }
    }
    failed
}

/// `logProviderError`: one failure; never fails.
pub async fn log_provider_error(buses: Option<&EventBuses>, input: ProviderErrorInput) {
    log_provider_errors(buses, std::slice::from_ref(&input)).await;
}

/// `logProviderErrorForChannel`: an unknown channel writes no row.
pub async fn log_provider_error_for_channel(
    buses: Option<&EventBuses>,
    channel: &str,
    mut input: ProviderErrorInput,
) {
    if !is_error_log_provider(channel) {
        return;
    }
    input.provider = channel.to_string();
    log_provider_error(buses, input).await;
}
