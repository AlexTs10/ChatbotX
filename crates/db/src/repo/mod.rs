//! Database access layer, part A (P3.2, `conversion/specs/P3.2.md`).
//!
//! Port of the shared code of `packages/database/src` (`client.ts`, `errors.ts`,
//! `keys.ts`, `utils.ts`, `statement-timeout.ts`, `partials/`, `queries/`) and of the
//! repositories `ads-conversion-event` .. `integration-whatsapp`.
//!
//! Rules (spec section 2):
//! - A repository function is a plain sync function over a `&Connection`. Reads run on a
//!   reader (`db.read`), writes on the writer (`db.write`); a TS `db.transaction` is one
//!   [`write`] call around the whole operation.
//! - Ids are `i64` snowflakes ([`new_id`]); the API layer formats them as strings.
//! - Timestamps are unix ms (`i64`), "now" is a parameter or [`now_ms`].
//! - JSON columns are [`Json`] (compact `serde_json` text).
//! - Every query filters by `workspaceId` where the TypeScript does.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{Connection, Row};

use crate::{Db, DbError, WriteTx};

// ---------------------------------------------------------------------------
// Row structs
// ---------------------------------------------------------------------------

/// A table row type made by [`table_row!`].
pub trait Model: Sized {
    /// The quoted table name.
    const TABLE: &'static str;
    /// The column names in struct order.
    const COLUMNS: &'static [&'static str];
    /// Reads the struct from `row`, starting at column `start`.
    fn from_row_at(row: &Row<'_>, start: usize) -> rusqlite::Result<Self>;
    /// Reads the struct from `row`, starting at column 0.
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Self::from_row_at(row, 0)
    }
    /// `"a", "b", ...` (or `x."a", x."b"` with an alias).
    fn columns_sql(alias: Option<&str>) -> String {
        let mut out = String::new();
        for (i, c) in Self::COLUMNS.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            if let Some(a) = alias {
                out.push_str(a);
                out.push('.');
            }
            out.push('"');
            out.push_str(c);
            out.push('"');
        }
        out
    }
    /// `SELECT <columns> FROM "<table>"`.
    fn select_sql() -> String {
        format!(
            "SELECT {} FROM \"{}\"",
            Self::columns_sql(None),
            Self::TABLE
        )
    }
}

/// Declares a row struct of one table: `pub struct X in "T" { field: Ty = "col", .. }`.
macro_rules! table_row {
    (
        $(#[$meta:meta])*
        pub struct $name:ident in $table:literal {
            $($(#[$fmeta:meta])* $field:ident : $ty:ty = $col:literal),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq)]
        pub struct $name {
            $($(#[$fmeta])* pub $field: $ty,)*
        }

        impl $crate::repo::Model for $name {
            const TABLE: &'static str = $table;
            const COLUMNS: &'static [&'static str] = &[$($col),*];
            #[allow(unused_assignments)]
            fn from_row_at(
                row: &::rusqlite::Row<'_>,
                start: usize,
            ) -> ::rusqlite::Result<Self> {
                let mut i = start;
                $(
                    let $field: $ty = row.get(i)?;
                    i += 1;
                )*
                Ok($name { $($field,)* })
            }
        }
    };
}

/// One row of `M` by a SQL `WHERE` clause (`None` when there is none).
pub fn find_one<M: Model>(
    conn: &Connection,
    where_sql: &str,
    params: &[&dyn ToSql],
) -> Result<Option<M>, Error> {
    let sql = format!("{} WHERE {where_sql} LIMIT 1", M::select_sql());
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows = stmt.query(params)?;
    match rows.next()? {
        Some(row) => Ok(Some(M::from_row(row)?)),
        None => Ok(None),
    }
}

/// All rows of `M` by a SQL tail (`WHERE ... ORDER BY ... LIMIT ...`).
pub fn find_many<M: Model>(
    conn: &Connection,
    tail_sql: &str,
    params: &[&dyn ToSql],
) -> Result<Vec<M>, Error> {
    let sql = format!("{} {tail_sql}", M::select_sql());
    query_models(conn, &sql, params)
}

/// Runs `sql` (it must select the columns of `M` first) and maps every row.
pub fn query_models<M: Model>(
    conn: &Connection,
    sql: &str,
    params: &[&dyn ToSql],
) -> Result<Vec<M>, Error> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt
        .query_map(params, |row| M::from_row(row))?
        .collect::<rusqlite::Result<Vec<M>>>()?;
    Ok(rows)
}

/// Runs `sql` with `RETURNING <columns of M>` and maps the returned rows.
pub fn returning<M: Model>(
    conn: &Connection,
    sql: &str,
    params: &[&dyn ToSql],
) -> Result<Vec<M>, Error> {
    let sql = format!("{sql} RETURNING {}", M::columns_sql(None));
    query_models(conn, &sql, params)
}

// ---------------------------------------------------------------------------
// JSON columns
// ---------------------------------------------------------------------------

/// A JSON column value (stored as compact `serde_json` text).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Json(pub serde_json::Value);

impl Json {
    /// The JSON `null` value.
    pub fn null() -> Json {
        Json(serde_json::Value::Null)
    }
}

impl std::ops::Deref for Json {
    type Target = serde_json::Value;
    fn deref(&self) -> &serde_json::Value {
        &self.0
    }
}

impl From<serde_json::Value> for Json {
    fn from(value: serde_json::Value) -> Json {
        Json(value)
    }
}

impl ToSql for Json {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.0.to_string()))
    }
}

impl FromSql for Json {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Text(bytes) | ValueRef::Blob(bytes) => serde_json::from_slice(bytes)
                .map(Json)
                .map_err(|e| FromSqlError::Other(Box::new(e))),
            ValueRef::Integer(i) => Ok(Json(serde_json::Value::from(i))),
            ValueRef::Real(f) => Ok(Json(serde_json::Value::from(f))),
            ValueRef::Null => Err(FromSqlError::InvalidType),
        }
    }
}

/// Serializes `value` to JSON text (for a bound parameter).
pub fn json_text<T: serde::Serialize + ?Sized>(value: &T) -> Result<String, Error> {
    serde_json::to_string(value).map_err(|e| Error::Message(format!("invalid JSON value: {e}")))
}

// ---------------------------------------------------------------------------
// Errors (errors.ts + the throw sites of the repositories)
// ---------------------------------------------------------------------------

/// Error of a repository call.
#[derive(Debug)]
pub enum Error {
    /// A database error (SQLSTATE mapping of `crate::DbError`).
    Db(DbError),
    /// `ModelNotfoundException` (`errors.ts`).
    NotFound(String),
    /// A plain `throw new Error(message)` of the TypeScript.
    Message(String),
    /// A typed error with a stable code (`ConnectSessionError`, ...).
    Coded {
        /// The error class name of the TypeScript (`ConnectSessionError`, ...).
        kind: &'static str,
        /// The code field.
        code: String,
        /// The message.
        message: String,
    },
}

impl Error {
    /// `new Error(message)`.
    pub fn msg(message: impl Into<String>) -> Error {
        Error::Message(message.into())
    }

    /// The error message (same text as the TypeScript `error.message`).
    pub fn message(&self) -> String {
        match self {
            Error::Db(e) => e.message().to_owned(),
            Error::NotFound(m) | Error::Message(m) => m.clone(),
            Error::Coded { message, .. } => message.clone(),
        }
    }

    /// The database error, if this is one.
    pub fn as_db(&self) -> Option<&DbError> {
        match self {
            Error::Db(e) => Some(e),
            _ => None,
        }
    }

    /// `instanceof ModelNotfoundException`.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::NotFound(_))
    }

    /// The `code` of a [`Error::Coded`].
    pub fn code(&self) -> Option<&str> {
        match self {
            Error::Coded { code, .. } => Some(code),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Db(e) => fmt::Display::fmt(e, f),
            Error::NotFound(m) | Error::Message(m) => f.write_str(m),
            Error::Coded { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Db(e) => Some(e),
            _ => None,
        }
    }
}

impl From<DbError> for Error {
    fn from(e: DbError) -> Error {
        Error::Db(e)
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Error {
        Error::Db(DbError::from(e))
    }
}

/// Result of a repository call.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Runs `f` in one write transaction (`BEGIN IMMEDIATE`). Any `Err` rolls back; a
/// non-database error comes back unchanged (the TS `db.transaction` rethrows).
pub async fn write<F, R>(db: &Db, f: F) -> Result<R>
where
    F: FnOnce(&mut WriteTx<'_>) -> Result<R> + Send + 'static,
    R: Send + 'static,
{
    let slot: Arc<Mutex<Option<Error>>> = Arc::new(Mutex::new(None));
    let inner = Arc::clone(&slot);
    let outcome = db
        .write(move |tx| match f(tx) {
            Ok(v) => Ok(v),
            Err(Error::Db(e)) => Err(e),
            Err(other) => {
                let message = other.message();
                *inner.lock().unwrap_or_else(PoisonError::into_inner) = Some(other);
                Err(DbError::other(message))
            }
        })
        .await;
    match outcome {
        Ok(v) => Ok(v),
        Err(e) => match slot.lock().unwrap_or_else(PoisonError::into_inner).take() {
            Some(domain) => Err(domain),
            None => Err(Error::Db(e)),
        },
    }
}

/// Runs `f` on a reader connection (one snapshot).
pub async fn read<F, R>(db: &Db, f: F) -> Result<R>
where
    F: FnOnce(&Connection) -> Result<R> + Send + 'static,
    R: Send + 'static,
{
    let slot: Arc<Mutex<Option<Error>>> = Arc::new(Mutex::new(None));
    let inner = Arc::clone(&slot);
    let outcome = db
        .read(move |conn| match f(conn) {
            Ok(v) => Ok(v),
            Err(Error::Db(e)) => Err(e),
            Err(other) => {
                let message = other.message();
                *inner.lock().unwrap_or_else(PoisonError::into_inner) = Some(other);
                Err(DbError::other(message))
            }
        })
        .await;
    match outcome {
        Ok(v) => Ok(v),
        Err(e) => match slot.lock().unwrap_or_else(PoisonError::into_inner).take() {
            Some(domain) => Err(domain),
            None => Err(Error::Db(e)),
        },
    }
}

// ---------------------------------------------------------------------------
// Ids and clock
// ---------------------------------------------------------------------------

/// `Date.parse("2004-02-01T00:00:00.000Z")` (snowflake epoch of `packages/utils/src/id.ts`).
const SNOWFLAKE_EPOCH_MS: u64 = 1_075_593_600_000;
const SEQUENCE_SPACE: u64 = 1024;
const PLACE_ID_MAX: u64 = 15;

struct IdState {
    last: Option<u64>,
    start: u64,
    issued: u64,
}

static ID_STATE: Mutex<IdState> = Mutex::new(IdState {
    last: None,
    start: 0,
    issued: 0,
});

fn random_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default(),
    );
    hasher.finish()
}

fn place_id() -> u64 {
    static PLACE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *PLACE.get_or_init(|| {
        std::env::var("SNOWFLAKE_PLACE_ID")
            .ok()
            .filter(|raw| !raw.is_empty() && raw.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|id| *id <= PLACE_ID_MAX)
            .unwrap_or_else(|| 1 + random_u64() % PLACE_ID_MAX)
    })
}

/// A new snowflake id (`createId()`): `ms since 2004-02-01 << 14 | place << 10 | seq`.
/// Strictly increasing in this process.
pub fn new_id() -> i64 {
    let wall = u64::try_from(now_ms())
        .unwrap_or_default()
        .saturating_sub(SNOWFLAKE_EPOCH_MS);
    let mut state = ID_STATE.lock().unwrap_or_else(PoisonError::into_inner);
    let now = match state.last {
        Some(last) if wall <= last => {
            if state.start + state.issued >= SEQUENCE_SPACE {
                last + 1
            } else {
                last
            }
        }
        _ => wall,
    };
    if state.last != Some(now) {
        state.last = Some(now);
        state.start = random_u64() % SEQUENCE_SPACE;
        state.issued = 0;
    }
    let sequence = state.start + state.issued;
    state.issued += 1;
    i64::try_from((now << 14) | (place_id() << 10) | sequence).unwrap_or(i64::MAX)
}

/// A random RFC 4122 version 4 UUID text (`crypto.randomUUID()`); used for claim tokens.
pub fn random_uuid() -> String {
    let a = random_u64() ^ u64::from_ne_bytes(new_id().to_ne_bytes()).rotate_left(17);
    let b = random_u64();
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_be_bytes());
    bytes[8..].copy_from_slice(&b.to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The current time in unix ms.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

/// `Date#toISOString()` of a unix ms timestamp.
pub fn iso(ms: i64) -> String {
    use chrono::{Datelike, Timelike};
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| {
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
                d.year(),
                d.month(),
                d.day(),
                d.hour(),
                d.minute(),
                d.second(),
                d.timestamp_subsec_millis()
            )
        })
        .unwrap_or_default()
}

/// Parses an ISO-8601 / RFC 3339 timestamp (`toISOString()` form, or with an offset) to
/// unix ms.
pub fn parse_iso(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|d| d.timestamp_millis())
}

/// `?1, ?2, ...` for an `IN (...)` list starting at placeholder `first`.
pub fn placeholders(first: usize, count: usize) -> String {
    (first..first + count)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// JSON array text of ids for `IN (SELECT value FROM json_each(?))`.
pub fn id_list(ids: &[i64]) -> String {
    serde_json::Value::from(ids.to_vec()).to_string()
}

/// JSON array text of strings for `IN (SELECT value FROM json_each(?))`.
pub fn text_list<S: AsRef<str>>(items: &[S]) -> String {
    serde_json::Value::from(
        items
            .iter()
            .map(|s| s.as_ref().to_owned())
            .collect::<Vec<_>>(),
    )
    .to_string()
}

pub mod client;
pub mod keys;
pub mod models;
pub mod partials;
pub mod queries;
pub mod sql;
pub mod utils;

pub mod ads_conversion_event;
pub mod ai_conversation_embedding;
pub mod ai_file_embedding;
pub mod ai_file_embedding_id;
pub mod ai_handover_bulk_run;
pub mod ai_handover_settings;
pub mod ai_workspace_scope;
pub mod appointment_calendar;
pub mod automation_throttle;
pub mod broadcast;
pub mod broadcast_purge;
pub mod broadcast_recurrence;
pub mod chunked_purge;
pub mod comment_automation_event;
pub mod connect_session;
pub mod connection;
pub mod contact;
pub mod contact_activity;
pub mod contact_custom_field;
pub mod contact_inbox;
pub mod conversation_ai_context;
pub mod inbox;

// P3.3 (second half of the repositories).
pub mod auth_account;
pub mod invitation;
pub mod license;
pub mod mac_partitions;
pub mod media_library_file;
pub mod media_library_folder;
pub mod messaging_ad_operation;
pub mod messaging_ads_connection;
pub mod messenger_welcome_message_flow;
pub mod meta_capi_event;
pub mod mini_app;
pub mod mobile_refresh_token;
pub mod oauth_access_token;
pub mod oauth_client;
pub mod oauth_consent;
pub mod oauth_refresh_token;
pub mod patch;
pub mod place;
pub mod platform_admin;
pub mod reference_existence;
pub mod reflink;
pub mod resource_deletion_log;
pub mod response_sla;
pub mod sequence;
pub mod sequence_dispatch;
pub mod small_repos;
pub mod store_integration;
pub mod tag_channel;
pub mod template_resource_backup;
pub mod template_selectable_resource;
pub mod template_version;
pub mod tenant;
pub mod trigger;
pub mod user;
pub mod vendor;
pub mod webhook;
pub mod whatsapp_business_account;
pub mod whatsapp_coexist_staging;
pub mod workspace;
pub mod workspace_api_token;
pub mod workspace_deletion_record;
pub mod workspace_member;
pub mod workspace_purge;
pub mod leaderboard;
pub mod leaderboard_history;
pub mod leaderboard_participant;
pub mod meta_catalog_item;
pub mod poll;
pub mod product;
pub mod working_hours;

pub use client::{
    CappedCount, count_query_capped, is_foreign_key_violation_error, is_statement_timeout_error,
    is_unique_violation_error, to_loggable_database_error,
};
pub use utils::{
    Pagination, PaginationInput, SortItem, escape_like_pattern, get_pagination_with_defaults,
    is_bigint_id, like_contains, parse_pagination, with_id_tie_break,
};
