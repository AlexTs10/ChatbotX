//! Error mapping to Postgres-like SQLSTATE codes (spec section 2.4).
//!
//! Replaces `describeDatabaseError` / `isUniqueViolationError` /
//! `isForeignKeyViolationError` of `packages/database/src/client.ts`.

use std::fmt;

use rusqlite::{Connection, ffi, params};

/// SQLSTATE of a statement interrupted by the statement timeout.
pub(crate) const QUERY_CANCELED: &str = "57014";

/// A database error with the Postgres-like fields the business code checks.
#[derive(Debug)]
pub struct DbError {
    inner: Box<Inner>,
}

#[derive(Debug)]
struct Inner {
    code: Option<&'static str>,
    constraint: Option<String>,
    table: Option<String>,
    detail: Option<String>,
    message: String,
    /// Columns of `UNIQUE constraint failed: T.a, T.b`, until the writer maps them to an
    /// index name (needs the catalog, see [`DbError::resolve_constraint`]).
    unique_columns: Vec<String>,
    /// `SQLITE_CONSTRAINT_PRIMARYKEY`: `constraint` is `<table>_pkey` until
    /// [`DbError::resolve_constraint`] finds a `CONSTRAINT "<name>" PRIMARY KEY` in the table SQL.
    primary_key: bool,
    source: Option<rusqlite::Error>,
}

impl DbError {
    /// An error that does not come from SQLite (closed database, failed check, ...).
    pub fn other(message: impl Into<String>) -> DbError {
        DbError {
            inner: Box::new(Inner {
                code: None,
                constraint: None,
                table: None,
                detail: None,
                message: message.into(),
                unique_columns: Vec::new(),
                primary_key: false,
                source: None,
            }),
        }
    }

    /// The writer thread or the reader pool is gone.
    pub(crate) fn closed() -> DbError {
        DbError::other("the database is closed")
    }

    /// The SQLSTATE-like code of spec 2.4 (`23505`, `23503`, `23502`,
    /// `23514`, `57014`, `55P03`, `54000`), or `None`.
    pub fn sqlstate(&self) -> Option<&str> {
        self.inner.code
    }

    /// For `23505`: the unique index / UNIQUE constraint name (`<table>_pkey` for a
    /// primary key, or the name of a `CONSTRAINT "<name>" PRIMARY KEY`).
    /// For `23514`: the text after `CHECK constraint failed: ` (the constraint name of a
    /// named CHECK; the column name or expression of an unnamed column CHECK).
    /// `None` for every other error.
    pub fn constraint(&self) -> Option<&str> {
        self.inner.constraint.as_deref()
    }

    /// The table named in the SQLite message, if any.
    pub fn table(&self) -> Option<&str> {
        self.inner.table.as_deref()
    }

    /// The SQLite message (the Postgres `detail` field).
    pub fn detail(&self) -> Option<&str> {
        self.inner.detail.as_deref()
    }

    /// The error message.
    pub fn message(&self) -> &str {
        &self.inner.message
    }

    /// True for SQLSTATE `23505`.
    pub fn is_unique_violation(&self) -> bool {
        self.inner.code == Some("23505")
    }

    /// True for SQLSTATE `23503`.
    pub fn is_foreign_key_violation(&self) -> bool {
        self.inner.code == Some("23503")
    }

    /// Maps the column list of a UNIQUE error to the index / constraint name with the
    /// catalog of `conn` (spec 2.4: `(table, [columns]) -> index name`; PRIMARY KEY ->
    /// `<table>_pkey`, or the name of `CONSTRAINT "<name>" PRIMARY KEY` in the table SQL;
    /// of two indexes on the same columns the non-partial one wins).
    /// For the `index 'name'` form it fills in the table. Errors of the catalog lookup
    /// are ignored: the error keeps its code and message.
    pub(crate) fn resolve_constraint(&mut self, conn: &Connection) {
        if !self.is_unique_violation() {
            return;
        }
        if self.inner.primary_key {
            if let Some(table) = &self.inner.table {
                if let Ok(Some(name)) = named_primary_key(conn, table) {
                    self.inner.constraint = Some(name);
                }
            }
            return;
        }
        if let (Some(name), None) = (&self.inner.constraint, &self.inner.table) {
            self.inner.table = conn
                .query_row(
                    "SELECT tbl_name FROM sqlite_schema WHERE type = 'index' AND name = ?1",
                    params![name],
                    |row| row.get(0),
                )
                .ok();
            return;
        }
        if self.inner.constraint.is_some() || self.inner.unique_columns.is_empty() {
            return;
        }
        let Some(table) = self.inner.table.clone() else {
            return;
        };
        if let Ok(Some(name)) = unique_constraint_name(conn, &table, &self.inner.unique_columns) {
            self.inner.constraint = Some(name);
        }
    }
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner.code {
            Some(code) => write!(f, "{} (SQLSTATE {code})", self.inner.message),
            None => f.write_str(&self.inner.message),
        }
    }
}

impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.inner
            .source
            .as_ref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

impl From<rusqlite::Error> for DbError {
    fn from(error: rusqlite::Error) -> DbError {
        let rusqlite::Error::SqliteFailure(failure, sqlite_message) = &error else {
            let mut out = DbError::other(error.to_string());
            out.inner.source = Some(error);
            return out;
        };
        let extended = failure.extended_code;
        let text = sqlite_message
            .clone()
            .unwrap_or_else(|| failure.to_string());
        let code = sqlstate_for(extended);
        let mut out = DbError::other(text.clone());
        out.inner.code = code;
        out.inner.detail = Some(text.clone());
        match code {
            Some(QUERY_CANCELED) => {
                out.inner.message = "canceling statement due to statement timeout".to_owned();
            }
            Some("23505") => {
                if let Some(rest) = text.strip_prefix("UNIQUE constraint failed: ") {
                    if let Some(name) = rest
                        .strip_prefix("index '")
                        .and_then(|r| r.strip_suffix('\''))
                    {
                        out.inner.constraint = Some(name.to_owned());
                    } else {
                        let (table, columns) = parse_column_list(rest);
                        if extended == ffi::SQLITE_CONSTRAINT_PRIMARYKEY {
                            out.inner.constraint = table.as_ref().map(|t| format!("{t}_pkey"));
                            out.inner.primary_key = true;
                        } else {
                            out.inner.unique_columns = columns;
                        }
                        out.inner.table = table;
                    }
                }
            }
            Some("23502") => {
                if let Some(rest) = text.strip_prefix("NOT NULL constraint failed: ") {
                    out.inner.table = parse_column_list(rest).0;
                }
            }
            Some("23514") => {
                // Named CHECK: `CHECK constraint failed: <name>`; unnamed column CHECK: the
                // column name or the expression text. The raw text stays in `detail`.
                if let Some(rest) = text.strip_prefix("CHECK constraint failed: ") {
                    let name = rest.trim();
                    if !name.is_empty() {
                        out.inner.constraint = Some(name.to_owned());
                    }
                }
            }
            _ => {}
        }
        out.inner.source = Some(error);
        out
    }
}

/// SQLite extended result code -> SQLSTATE (spec 2.4 table).
fn sqlstate_for(extended: i32) -> Option<&'static str> {
    match extended {
        ffi::SQLITE_CONSTRAINT_UNIQUE | ffi::SQLITE_CONSTRAINT_PRIMARYKEY => Some("23505"),
        ffi::SQLITE_CONSTRAINT_FOREIGNKEY => Some("23503"),
        ffi::SQLITE_CONSTRAINT_NOTNULL => Some("23502"),
        ffi::SQLITE_CONSTRAINT_CHECK => Some("23514"),
        _ => match extended & 0xff {
            ffi::SQLITE_INTERRUPT => Some(QUERY_CANCELED),
            ffi::SQLITE_BUSY | ffi::SQLITE_LOCKED => Some("55P03"),
            ffi::SQLITE_TOOBIG => Some("54000"),
            _ => None,
        },
    }
}

/// `T.a, T.b` -> (`Some("T")`, `["a", "b"]`).
fn parse_column_list(list: &str) -> (Option<String>, Vec<String>) {
    let mut table = None;
    let mut columns = Vec::new();
    for item in list.split(", ") {
        if let Some((t, c)) = item.split_once('.') {
            table.get_or_insert_with(|| t.to_owned());
            columns.push(c.to_owned());
        }
    }
    (table, columns)
}

/// The unique index of `table` whose key columns are exactly `columns` (in order).
fn unique_constraint_name(
    conn: &Connection,
    table: &str,
    columns: &[String],
) -> rusqlite::Result<Option<String>> {
    let mut list = conn
        .prepare(r#"SELECT name, origin, partial FROM pragma_index_list(?1) WHERE "unique" = 1"#)?;
    let candidates = list
        .query_map(params![table], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? != 0,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut info = conn.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")?;
    let mut partial_match: Option<String> = None;
    for (index, origin, partial) in candidates {
        let key = info
            .query_map(params![index], |row| row.get::<_, Option<String>>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let same = key.len() == columns.len()
            && key
                .iter()
                .zip(columns)
                .all(|(k, c)| k.as_deref() == Some(c.as_str()));
        if !same {
            continue;
        }
        let name = match origin.as_str() {
            "pk" => format!("{table}_pkey"),
            "u" => named_unique_constraint(conn, table, columns)?.unwrap_or(index),
            _ => index,
        };
        if !partial {
            return Ok(Some(name));
        }
        partial_match.get_or_insert(name);
    }
    Ok(partial_match)
}

/// The name of `CONSTRAINT "<name>" UNIQUE ("a", "b")` in the table SQL, for `columns`.
fn named_unique_constraint(
    conn: &Connection,
    table: &str,
    columns: &[String],
) -> rusqlite::Result<Option<String>> {
    let sql: Option<String> = conn.query_row(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1",
        params![table],
        |row| row.get(0),
    )?;
    let Some(sql) = sql else { return Ok(None) };
    let mut rest = sql.as_str();
    while let Some(start) = rest.find("CONSTRAINT \"") {
        rest = &rest[start + "CONSTRAINT \"".len()..];
        let Some(end) = rest.find('"') else { break };
        let name = &rest[..end];
        let after = rest[end + 1..].trim_start();
        if let Some(list) = after
            .strip_prefix("UNIQUE")
            .map(str::trim_start)
            .and_then(|a| a.strip_prefix('('))
        {
            if let Some(close) = list.find(')') {
                let cols: Vec<&str> = list[..close]
                    .split(',')
                    .map(|c| c.trim().trim_matches('"'))
                    .collect();
                if cols.len() == columns.len()
                    && cols.iter().zip(columns).all(|(a, b)| *a == b.as_str())
                {
                    return Ok(Some(name.to_owned()));
                }
            }
        }
    }
    Ok(None)
}

/// The name of `CONSTRAINT "<name>" PRIMARY KEY` in the SQL of `table`, if it has one.
fn named_primary_key(conn: &Connection, table: &str) -> rusqlite::Result<Option<String>> {
    let sql: Option<String> = conn.query_row(
        "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1",
        params![table],
        |row| row.get(0),
    )?;
    let Some(sql) = sql else { return Ok(None) };
    let mut rest = sql.as_str();
    while let Some(start) = rest.find("CONSTRAINT \"") {
        rest = &rest[start + "CONSTRAINT \"".len()..];
        let Some(end) = rest.find('"') else { break };
        if rest[end + 1..].trim_start().starts_with("PRIMARY KEY") {
            return Ok(Some(rest[..end].to_owned()));
        }
    }
    Ok(None)
}

/// Port of the return value of `describeDatabaseError` (`src/client.ts`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatabaseErrorDescription {
    pub code: Option<String>,
    pub constraint: Option<String>,
    pub detail: Option<String>,
    pub message: String,
    pub table: Option<String>,
}

/// Port of `describeDatabaseError`: walks the `source()` chain (at most 5 levels, as the
/// TypeScript version) to the first [`DbError`] that has a SQLSTATE and reports its
/// fields; without one, only `message` (the top-level `Display`) is set.
pub fn describe_database_error(
    error: &(dyn std::error::Error + 'static),
) -> DatabaseErrorDescription {
    let mut current = Some(error);
    for _ in 0..5 {
        let Some(e) = current else { break };
        if let Some(db) = e.downcast_ref::<DbError>() {
            if let Some(code) = db.sqlstate() {
                return DatabaseErrorDescription {
                    code: Some(code.to_owned()),
                    constraint: db.constraint().map(str::to_owned),
                    detail: db.detail().map(str::to_owned),
                    message: db.message().to_owned(),
                    table: db.table().map(str::to_owned),
                };
            }
        }
        current = e.source();
    }
    DatabaseErrorDescription {
        message: error.to_string(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            r#"CREATE TABLE "T" ("id" INTEGER PRIMARY KEY NOT NULL, "a" TEXT, "b" TEXT, "c" TEXT NOT NULL DEFAULT 'x',
                 "k" TEXT CHECK ("k" IN ('x')), "e" TEXT, CONSTRAINT "T_c_key" UNIQUE ("c")) STRICT;
               CREATE UNIQUE INDEX "T_a_b_key" ON "T" ("a", "b");
               CREATE UNIQUE INDEX "T_a_b_partial_key" ON "T" ("a", "b") WHERE "a" IS NOT NULL;
               CREATE UNIQUE INDEX "T_lower_e_key" ON "T" (lower("e"));
               CREATE TABLE "P" ("x" INTEGER NOT NULL, "y" INTEGER NOT NULL, CONSTRAINT "P_pkey" PRIMARY KEY ("x", "y")) STRICT;
               CREATE TABLE "R" ("x" INTEGER NOT NULL, "y" INTEGER NOT NULL, CONSTRAINT "Rs_pkey" PRIMARY KEY ("x", "y")) STRICT;
               CREATE TABLE "Q" ("id" INTEGER PRIMARY KEY NOT NULL, "a" INTEGER NOT NULL, "b" INTEGER NOT NULL,
                 CONSTRAINT "Q_order_check" CHECK ("a" < "b")) STRICT;"#,
        )
        .expect("schema");
        conn
    }

    fn fail(conn: &Connection, sql: &str) -> DbError {
        let mut err = DbError::from(conn.execute_batch(sql).expect_err("must fail"));
        err.resolve_constraint(conn);
        err
    }

    #[test]
    fn unique_index_name_prefers_the_non_partial_index() {
        let conn = conn();
        let err = fail(
            &conn,
            r#"INSERT INTO "T" ("id","a","b","c") VALUES (1,'a','b','1'), (2,'a','b','2')"#,
        );
        assert_eq!(err.sqlstate(), Some("23505"));
        assert_eq!(err.constraint(), Some("T_a_b_key"));
        assert_eq!(err.table(), Some("T"));
        assert!(err.detail().is_some_and(|d| d.contains("UNIQUE")));
    }

    #[test]
    fn named_unique_constraint_and_primary_keys() {
        let conn = conn();
        let err = fail(
            &conn,
            r#"INSERT INTO "T" ("id","c") VALUES (1,'same'), (2,'same')"#,
        );
        assert_eq!(err.constraint(), Some("T_c_key"));
        let err = fail(
            &conn,
            r#"INSERT INTO "T" ("id","c") VALUES (1,'p'), (1,'q')"#,
        );
        assert_eq!(
            (err.sqlstate(), err.constraint()),
            (Some("23505"), Some("T_pkey"))
        );
        let err = fail(&conn, r#"INSERT INTO "P" VALUES (1, 2), (1, 2)"#);
        assert_eq!((err.constraint(), err.table()), (Some("P_pkey"), Some("P")));
        // A named PRIMARY KEY constraint keeps its own name (Postgres `ContactsOnBroadcast_pkey`).
        let err = fail(&conn, r#"INSERT INTO "R" VALUES (1, 2), (1, 2)"#);
        assert_eq!(
            (err.constraint(), err.table()),
            (Some("Rs_pkey"), Some("R"))
        );
    }

    #[test]
    fn check_errors_carry_the_constraint_name() {
        let conn = conn();
        let err = fail(&conn, r#"INSERT INTO "Q" VALUES (1, 5, 5)"#);
        assert_eq!(
            (err.sqlstate(), err.constraint()),
            (Some("23514"), Some("Q_order_check"))
        );
        assert_eq!(err.detail(), Some("CHECK constraint failed: Q_order_check"));
        // Unnamed column CHECK: SQLite names the column or the expression.
        let err = fail(&conn, r#"INSERT INTO "T" ("id","k") VALUES (1, 'y')"#);
        assert_eq!(err.sqlstate(), Some("23514"));
        assert!(err.constraint().is_some_and(|c| c.contains('k')), "{err}");
        let described =
            describe_database_error(&fail(&conn, r#"INSERT INTO "Q" VALUES (2, 9, 1)"#));
        assert_eq!(described.constraint.as_deref(), Some("Q_order_check"));
    }

    #[test]
    fn expression_index_reports_its_name_and_table() {
        let conn = conn();
        let err = fail(
            &conn,
            r#"INSERT INTO "T" ("id","e","c") VALUES (1,'X','1'), (2,'x','2')"#,
        );
        assert_eq!(err.constraint(), Some("T_lower_e_key"));
        assert_eq!(err.table(), Some("T"));
    }

    #[test]
    fn not_null_check_and_foreign_key_codes() {
        let conn = conn();
        let err = fail(&conn, r#"INSERT INTO "T" ("id","c") VALUES (1, NULL)"#);
        assert_eq!(
            (err.sqlstate(), err.table(), err.constraint()),
            (Some("23502"), Some("T"), None)
        );
        let err = fail(&conn, r#"INSERT INTO "T" ("id","k") VALUES (1, 'y')"#);
        assert_eq!(err.sqlstate(), Some("23514"));
        conn.execute_batch(
            r#"PRAGMA foreign_keys = ON; CREATE TABLE "C" ("p" INTEGER REFERENCES "T" ("id")) STRICT;"#,
        )
        .expect("fk table");
        let err = fail(&conn, r#"INSERT INTO "C" VALUES (99)"#);
        assert!(err.is_foreign_key_violation());
        assert_eq!(err.constraint(), None);
    }

    #[test]
    fn other_errors_have_no_code() {
        let conn = conn();
        let err = fail(&conn, "SELECT * FROM nope");
        assert_eq!(err.sqlstate(), None);
        assert!(err.message().contains("no such table"));
        let err = DbError::from(rusqlite::Error::QueryReturnedNoRows);
        assert_eq!((err.sqlstate(), err.detail()), (None, None));
    }
}
