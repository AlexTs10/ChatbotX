//! Port of the helpers of `packages/database/src/client.ts` (the pool itself is
//! [`crate::Db`]; `setLocalStatementTimeout` is [`crate::WriteTx::set_statement_timeout`];
//! `liftDecompressionLimit` has no SQLite meaning).

use rusqlite::types::ToSql;
use rusqlite::Connection;

use super::{Error, Result};

/// Result of [`count_query_capped`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CappedCount {
    pub total: i64,
    pub capped: bool,
    pub cap: i64,
}

/// `countQueryCapped`: counts up to `cap` rows of `query` (a `SELECT ... FROM ...`
/// without a limit) by scanning at most `cap + 1` of them.
pub fn count_query_capped(
    conn: &Connection,
    cap: i64,
    query: &str,
    params: &[&dyn ToSql],
) -> Result<CappedCount> {
    todo!("P3.2");
    let sql = format!(
        "SELECT COUNT(*) FROM (SELECT 1 FROM ({query}) LIMIT {}) AS \"cappedCount\"",
        cap + 1
    );
    let count: i64 = conn.query_row(&sql, params, |row| row.get(0))?;
    Ok(CappedCount {
        total: count.min(cap),
        capped: count > cap,
        cap,
    })
}

/// `countWithRelationsFilter` for a plain SQL filter: `SELECT count(*) FROM "<table>"
/// [WHERE <filter>]`.
pub fn count_where(
    conn: &Connection,
    table: &str,
    filter: Option<&str>,
    params: &[&dyn ToSql],
) -> Result<i64> {
    let sql = match filter {
        Some(f) => format!("SELECT count(*) FROM \"{table}\" WHERE {f}"),
        None => format!("SELECT count(*) FROM \"{table}\""),
    };
    Ok(conn.query_row(&sql, params, |row| row.get(0))?)
}

/// `isUniqueViolationError(error, constraint?)` (SQLSTATE 23505).
pub fn is_unique_violation_error(error: &Error, constraint: Option<&str>) -> bool {
    is_constraint_violation(error, "23505", constraint)
}

/// `isForeignKeyViolationError(error, constraint?)` (SQLSTATE 23503). SQLite FKs have no
/// names, so a `constraint` never matches.
pub fn is_foreign_key_violation_error(error: &Error, constraint: Option<&str>) -> bool {
    is_constraint_violation(error, "23503", constraint)
}

/// `isStatementTimeoutError` (SQLSTATE 57014).
pub fn is_statement_timeout_error(error: &Error) -> bool {
    is_constraint_violation(error, "57014", None)
}

fn is_constraint_violation(error: &Error, code: &str, constraint: Option<&str>) -> bool {
    let Some(db) = error.as_db() else {
        return false;
    };
    if db.sqlstate() != Some(code) {
        return false;
    }
    match constraint {
        None => true,
        Some(name) => db.constraint() == Some(name),
    }
}

/// `toLoggableDatabaseError`: a database error becomes `Database error (<code>)`
/// without the bound values; any other error message is kept.
pub fn to_loggable_database_error(error: &Error) -> String {
    match error.as_db() {
        Some(db) => match db.sqlstate() {
            Some(code) => format!("Database error ({code})"),
            None => "Database error".to_owned(),
        },
        None => error.message(),
    }
}

/// `findOrFail`: the first row of `M` matching `where_sql`, else `ModelNotfoundException`
/// with `message` (default `Record not found`).
pub fn find_or_fail<M: super::Model>(
    conn: &Connection,
    where_sql: &str,
    params: &[&dyn ToSql],
    message: Option<&str>,
) -> Result<M> {
    super::find_one::<M>(conn, where_sql, params)?
        .ok_or_else(|| Error::NotFound(message.unwrap_or("Record not found").to_owned()))
}

/// `throwIfExists`: fails with `message` (default `Resource already exists`) when a row
/// of `table` matches `where_sql`.
pub fn throw_if_exists(
    conn: &Connection,
    table: &str,
    where_sql: &str,
    params: &[&dyn ToSql],
    message: Option<&str>,
) -> Result<()> {
    let sql = format!("SELECT 1 FROM \"{table}\" WHERE {where_sql} LIMIT 1");
    let mut stmt = conn.prepare_cached(&sql)?;
    if stmt.exists(params)? {
        return Err(Error::msg(message.unwrap_or("Resource already exists")));
    }
    Ok(())
}
