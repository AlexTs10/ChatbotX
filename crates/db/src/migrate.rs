//! Embedded migration runner (spec section 4).

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

use crate::DbError;

/// One embedded migration file `crates/db/migrations/<NNNN>_<name>.sql`.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    /// The file name under `crates/db/migrations/`.
    pub file: &'static str,
    pub sql: &'static str,
}

impl Migration {
    /// SHA-256 of the exact embedded bytes, lowercase hex.
    pub fn checksum(&self) -> String {
        sha256_hex(self.sql.as_bytes())
    }
}

macro_rules! migration {
    ($version:literal, $name:literal, $file:literal) => {
        Migration {
            version: $version,
            name: $name,
            file: $file,
            sql: include_str!(concat!("../migrations/", $file)),
        }
    };
}

/// The migrations of this binary, in order. No files are read at run time.
pub const MIGRATIONS: &[Migration] = &[
    migration!(1, "core", "0001_core.sql"),
    migration!(2, "messaging", "0002_messaging.sql"),
    migration!(3, "jobs", "0003_jobs.sql"),
    migration!(4, "rest", "0004_rest.sql"),
];

const CREATE_SCHEMA_MIGRATIONS: &str = r#"CREATE TABLE IF NOT EXISTS "schema_migrations" (
  "version"   INTEGER PRIMARY KEY NOT NULL,
  "name"      TEXT NOT NULL,
  "checksum"  TEXT NOT NULL CHECK (length("checksum") = 64),
  "appliedAt" INTEGER NOT NULL
) STRICT"#;

/// What one [`migrate`] run applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MigrateReport {
    /// Versions applied by this run, in order. Empty on a second run.
    pub applied: Vec<u32>,
}

/// Why [`migrate`] refused to start or failed (spec 4.3).
#[derive(Debug)]
pub enum MigrationError {
    /// The database has a version this binary does not know.
    UnknownVersion { version: u32 },
    /// An applied version has a different name or checksum.
    ChecksumMismatch {
        version: u32,
        expected: String,
        found: String,
    },
    /// Applied versions have a gap.
    Gap { missing: u32 },
    /// A migration failed and was rolled back.
    Failed {
        version: u32,
        name: String,
        source: DbError,
    },
    /// The runner could not read or write `schema_migrations`.
    Db(DbError),
}

impl fmt::Display for MigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrationError::UnknownVersion { version } => {
                write!(
                    f,
                    "database has migration {version}, which this binary does not know (database is newer than this binary)"
                )
            }
            MigrationError::ChecksumMismatch {
                version,
                expected,
                found,
            } => {
                write!(
                    f,
                    "migration {version} was changed after it was applied: expected {expected}, found {found}"
                )
            }
            MigrationError::Gap { missing } => {
                write!(f, "applied migrations have a gap: {missing} is missing")
            }
            MigrationError::Failed {
                version,
                name,
                source,
            } => {
                write!(
                    f,
                    "migration {version} ({name}) failed and was rolled back: {source}"
                )
            }
            MigrationError::Db(e) => write!(f, "migration runner: {e}"),
        }
    }
}

impl std::error::Error for MigrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MigrationError::Failed { source, .. } | MigrationError::Db(source) => Some(source),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for MigrationError {
    fn from(e: rusqlite::Error) -> MigrationError {
        MigrationError::Db(e.into())
    }
}

/// Applies the embedded migrations (`crates/db/migrations/NNNN_*.sql`) on `conn` (spec 4.3).
pub fn migrate(conn: &mut Connection) -> Result<MigrateReport, MigrationError> {
    migrate_with(conn, MIGRATIONS)
}

/// [`migrate`] with an explicit migration list (the runner tests use their own lists).
pub(crate) fn migrate_with(
    conn: &mut Connection,
    migrations: &[Migration],
) -> Result<MigrateReport, MigrationError> {
    // 1. New file: incremental auto-vacuum before the first table. Always WAL.
    let objects: i64 = conn.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))?;
    if objects == 0 {
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL")?;
    }
    let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;

    // 2.
    conn.execute_batch(CREATE_SCHEMA_MIGRATIONS)?;

    // 3. Validate what is applied.
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let rows = read_applied(conn);
    end_transaction(conn, rows.is_ok())?;
    let rows = rows?;
    for (version, name, checksum) in &rows {
        let Some(m) = migrations.iter().find(|m| m.version == *version) else {
            return Err(MigrationError::UnknownVersion { version: *version });
        };
        if m.name != name {
            return Err(MigrationError::ChecksumMismatch {
                version: *version,
                expected: m.name.to_owned(),
                found: name.clone(),
            });
        }
        let expected = m.checksum();
        if expected != *checksum {
            return Err(MigrationError::ChecksumMismatch {
                version: *version,
                expected,
                found: checksum.clone(),
            });
        }
    }
    // 5. Gap check (rows are ordered by version).
    let first = migrations.first().map_or(1, |m| m.version);
    for (expected, (version, _, _)) in (first..).zip(&rows) {
        if *version != expected {
            return Err(MigrationError::Gap { missing: expected });
        }
    }
    let highest = rows.last().map_or(0, |r| r.0);

    // 4. Apply the newer versions, one transaction each.
    let mut applied = Vec::new();
    for m in migrations.iter().filter(|m| m.version > highest) {
        conn.execute_batch("PRAGMA foreign_keys = OFF")?;
        let result = apply_one(conn, m);
        if result.is_err() && !conn.is_autocommit() {
            let _ = conn.execute_batch("ROLLBACK");
        }
        conn.execute_batch("PRAGMA foreign_keys = ON")?;
        match result {
            Ok(true) => {
                tracing::info!(version = m.version, name = m.name, "applied migration");
                applied.push(m.version);
            }
            Ok(false) => {}
            Err(source) => {
                return Err(MigrationError::Failed {
                    version: m.version,
                    name: m.name.to_owned(),
                    source,
                });
            }
        }
    }

    // 6.
    conn.execute_batch("PRAGMA optimize=0x10002")?;
    Ok(MigrateReport { applied })
}

fn read_applied(conn: &Connection) -> Result<Vec<(u32, String, String)>, MigrationError> {
    let mut stmt = conn.prepare(
        r#"SELECT "version", "name", "checksum" FROM "schema_migrations" ORDER BY "version""#,
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn end_transaction(conn: &Connection, commit: bool) -> rusqlite::Result<()> {
    if conn.is_autocommit() {
        return Ok(());
    }
    conn.execute_batch(if commit { "COMMIT" } else { "ROLLBACK" })
}

/// Steps 4.2-4.6. `Ok(false)`: another process applied this version meanwhile.
fn apply_one(conn: &Connection, m: &Migration) -> Result<bool, DbError> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let current: i64 = conn.query_row(
        r#"SELECT COALESCE(MAX("version"), 0) FROM "schema_migrations""#,
        [],
        |r| r.get(0),
    )?;
    if current >= i64::from(m.version) {
        conn.execute_batch("COMMIT")?;
        return Ok(false);
    }
    conn.execute_batch(m.sql)?;
    let violations = foreign_key_violations(conn)?;
    if !violations.is_empty() {
        return Err(DbError::other(format!(
            "PRAGMA foreign_key_check failed: {}",
            violations.join("; ")
        )));
    }
    conn.execute(
        r#"INSERT INTO "schema_migrations" ("version", "name", "checksum", "appliedAt") VALUES (?1, ?2, ?3, ?4)"#,
        params![m.version, m.name, m.checksum(), now_ms()],
    )?;
    conn.execute_batch(&format!("PRAGMA user_version = {}", m.version))?;
    conn.execute_batch("COMMIT")?;
    Ok(true)
}

/// The first rows of `PRAGMA foreign_key_check` as `table rowid -> parent`.
fn foreign_key_violations(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let table: String = row.get(0)?;
        let rowid: Option<i64> = row.get(1)?;
        let parent: String = row.get(2)?;
        out.push(format!(
            "{table} rowid {} -> {parent}",
            rowid.map_or_else(|| "?".to_owned(), |r| r.to_string())
        ));
        if out.len() >= 5 {
            break;
        }
    }
    Ok(out)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest.iter() {
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        crate::setup_connection(&conn, crate::Role::Writer).expect("setup");
        conn
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).expect(sql)
    }

    const GOOD: Migration = Migration {
        version: 1,
        name: "one",
        file: "0001_one.sql",
        sql: r#"CREATE TABLE "a" ("x" INTEGER) STRICT;"#,
    };
    const SECOND: Migration = Migration {
        version: 2,
        name: "two",
        file: "0002_two.sql",
        sql: r#"CREATE TABLE "b" ("x" INTEGER) STRICT;"#,
    };
    const THIRD: Migration = Migration {
        version: 3,
        name: "three",
        file: "0003_three.sql",
        sql: r#"CREATE TABLE "c" ("x" INTEGER) STRICT;"#,
    };

    #[test]
    fn embedded_migrations_are_well_formed() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/migrations");
        let mut files: Vec<String> = std::fs::read_dir(dir)
            .expect("migrations dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".sql"))
            .collect();
        files.sort();
        assert_eq!(
            files,
            MIGRATIONS
                .iter()
                .map(|m| m.file.to_owned())
                .collect::<Vec<_>>()
        );
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(
                m.version as usize,
                i + 1,
                "versions start at 1 with no gaps"
            );
            assert_eq!(m.file, format!("{:04}_{}.sql", m.version, m.name));
            assert!(
                !m.sql.contains('\r'),
                "{} must use LF line endings (the checksum covers the bytes)",
                m.file
            );
            for line in m.sql.lines() {
                let code = line
                    .split("--")
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_uppercase();
                let forbidden = [
                    "VACUUM",
                    "PRAGMA JOURNAL_MODE",
                    "PRAGMA FOREIGN_KEYS",
                    "ATTACH",
                    "DETACH",
                    "COMMIT",
                    "ROLLBACK",
                    "END TRANSACTION",
                ]
                .iter()
                .any(|p| code.starts_with(p))
                    || code == "BEGIN;"
                    || code.starts_with("BEGIN TRANSACTION")
                    || code.starts_with("BEGIN IMMEDIATE")
                    || code.starts_with("BEGIN DEFERRED")
                    || code.starts_with("BEGIN EXCLUSIVE");
                assert!(
                    !forbidden,
                    "{}: statement cannot run inside a transaction: {line}",
                    m.file
                );
            }
        }
    }

    #[test]
    fn fresh_apply_then_second_run_is_a_no_op() {
        let mut conn = conn();
        assert_eq!(migrate(&mut conn).expect("migrate").applied, [1, 2, 3, 4]);
        assert_eq!(count(&conn, "PRAGMA user_version"), 4);
        assert_eq!(
            count(&conn, r#"SELECT count(*) FROM "schema_migrations""#),
            4
        );
        let checksums: Vec<String> = conn
            .prepare(r#"SELECT "checksum" FROM "schema_migrations" ORDER BY "version""#)
            .expect("prepare")
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<rusqlite::Result<_>>()
            .expect("rows");
        assert_eq!(
            checksums,
            MIGRATIONS[0..4]
                .iter()
                .map(Migration::checksum)
                .collect::<Vec<_>>()
        );
        assert!(checksums.iter().all(|c| c.len() == 64));
        assert_eq!(
            count(&conn, r#"SELECT count(*) FROM "Tenant" WHERE "id" = 1"#),
            1
        );
        let tables = count(
            &conn,
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name <> 'schema_migrations' \
             AND sql NOT LIKE 'CREATE VIRTUAL TABLE%' AND name NOT LIKE '%\\_fts\\_%' ESCAPE '\\' \
             AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'",
        );
        // 260 app tables (P2.3 + P2.4 + P2.5: 126 + 4 rollup tables) + 9 jobs tables (P2.6).
        assert_eq!(tables, 269);
        assert_eq!(
            count(&conn, "SELECT count(*) FROM pragma_foreign_key_check"),
            0
        );
        assert_eq!(count(&conn, "PRAGMA foreign_keys"), 1);

        assert_eq!(
            migrate(&mut conn).expect("second run").applied,
            Vec::<u32>::new()
        );
        assert_eq!(count(&conn, "PRAGMA user_version"), 4);
        assert_eq!(
            count(&conn, r#"SELECT count(*) FROM "schema_migrations""#),
            4
        );
    }

    #[test]
    fn applies_only_newer_versions() {
        let mut conn = conn();
        assert_eq!(migrate_with(&mut conn, &[GOOD]).expect("v1").applied, [1]);
        assert_eq!(
            migrate_with(&mut conn, &[GOOD, SECOND, THIRD])
                .expect("v2, v3")
                .applied,
            [2, 3]
        );
        assert_eq!(count(&conn, "PRAGMA user_version"), 3);
    }

    #[test]
    fn checksum_mismatch_refuses_to_start() {
        let mut conn = conn();
        migrate_with(&mut conn, &[GOOD]).expect("v1");
        let changed = Migration {
            sql: r#"CREATE TABLE "a" ("y" INTEGER) STRICT;"#,
            ..GOOD
        };
        let err = migrate_with(&mut conn, &[changed]).expect_err("changed file");
        assert!(
            matches!(err, MigrationError::ChecksumMismatch { version: 1, .. }),
            "{err}"
        );
        let renamed = Migration {
            name: "uno",
            ..GOOD
        };
        assert!(matches!(
            migrate_with(&mut conn, &[renamed]),
            Err(MigrationError::ChecksumMismatch { version: 1, .. })
        ));
    }

    #[test]
    fn unknown_version_refuses_to_start() {
        let mut conn = conn();
        migrate_with(&mut conn, &[GOOD, SECOND]).expect("v1, v2");
        let err = migrate_with(&mut conn, &[GOOD]).expect_err("newer database");
        assert!(
            matches!(err, MigrationError::UnknownVersion { version: 2 }),
            "{err}"
        );
    }

    #[test]
    fn gap_is_refused() {
        let mut conn = conn();
        migrate_with(&mut conn, &[GOOD]).expect("v1");
        conn.execute(
            r#"INSERT INTO "schema_migrations" VALUES (3, 'three', ?1, 0)"#,
            params![THIRD.checksum()],
        )
        .expect("fake v3");
        let err = migrate_with(&mut conn, &[GOOD, SECOND, THIRD]).expect_err("gap");
        assert!(matches!(err, MigrationError::Gap { missing: 2 }), "{err}");
    }

    #[test]
    fn failing_sql_rolls_back_and_leaves_schema_migrations_unchanged() {
        let mut conn = conn();
        let bad = Migration {
            sql: r#"CREATE TABLE "b" ("x" INTEGER) STRICT; SELECT * FROM "nope";"#,
            ..SECOND
        };
        let err = migrate_with(&mut conn, &[GOOD, bad, THIRD]).expect_err("bad v2");
        assert!(
            matches!(&err, MigrationError::Failed { version: 2, .. }),
            "{err}"
        );
        assert_eq!(
            count(
                &conn,
                "SELECT count(*) FROM sqlite_schema WHERE name IN ('b', 'c')"
            ),
            0
        );
        assert_eq!(
            count(&conn, r#"SELECT count(*) FROM "schema_migrations""#),
            1
        );
        assert_eq!(count(&conn, "PRAGMA user_version"), 1);
        assert_eq!(count(&conn, "PRAGMA foreign_keys"), 1);
        assert!(conn.is_autocommit());
        // A fixed binary re-runs from that version.
        assert_eq!(
            migrate_with(&mut conn, &[GOOD, SECOND, THIRD])
                .expect("fixed")
                .applied,
            [2, 3]
        );
    }

    #[test]
    fn foreign_key_check_failure_rolls_back() {
        let mut conn = conn();
        let broken = Migration {
            sql: r#"CREATE TABLE "p" ("id" INTEGER PRIMARY KEY NOT NULL) STRICT;
                    CREATE TABLE "q" ("pid" INTEGER REFERENCES "p" ("id")) STRICT;
                    INSERT INTO "q" VALUES (5);"#,
            ..GOOD
        };
        let err = migrate_with(&mut conn, &[broken]).expect_err("fk violation");
        match &err {
            MigrationError::Failed {
                version: 1, source, ..
            } => {
                assert!(source.message().contains("foreign_key_check"), "{source}");
            }
            other => panic!("unexpected {other}"),
        }
        assert_eq!(
            count(
                &conn,
                "SELECT count(*) FROM sqlite_schema WHERE name IN ('p', 'q')"
            ),
            0
        );
        assert_eq!(
            count(&conn, r#"SELECT count(*) FROM "schema_migrations""#),
            0
        );
        assert_eq!(count(&conn, "PRAGMA foreign_keys"), 1);
    }
}
