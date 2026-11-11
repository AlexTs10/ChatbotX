//! Shared helpers for the ported frozen tests of P2.3 (`conversion/specs/P2.3.md` section 5).
//!
//! Every helper goes through the public API of `chatbotx-db` (`Db::open`, `Db::migrate`,
//! `Db::read`, `Db::write`). The catalog helpers read the SQLite catalog with the
//! table-valued pragma functions.

#![allow(dead_code)]

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chatbotx_db::rusqlite::{self, params};
use chatbotx_db::{Db, DbError};

// ---------------------------------------------------------------------------
// Fresh database
// ---------------------------------------------------------------------------

/// A temporary directory that is removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> TempDir {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "chatbotx-db-test-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A migrated database in a temp file. Derefs to [`Db`].
pub struct TestDb {
    // Field order: `db` is dropped (connections closed) before the directory.
    db: Db,
    _dir: TempDir,
}

impl Deref for TestDb {
    type Target = Db;

    fn deref(&self) -> &Db {
        &self.db
    }
}

/// Opens a new temp-file database (connection setup of spec 2.2) and runs `migrate()`.
pub async fn fresh_db() -> TestDb {
    let dir = TempDir::new();
    let db = Db::open(dir.path().join("test.db")).expect("Db::open");
    db.migrate().await.expect("migrate");
    TestDb { db, _dir: dir }
}

// ---------------------------------------------------------------------------
// Catalog helpers
// ---------------------------------------------------------------------------

/// One row of `pragma_table_xinfo` (shows generated columns; `hidden` = 2/3 for them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub cid: i64,
    pub name: String,
    pub decl_type: String,
    pub notnull: bool,
    pub dflt_value: Option<String>,
    pub pk: i64,
    pub hidden: i64,
}

/// One row of `pragma_index_list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    pub name: String,
    pub unique: bool,
    /// `c` (CREATE INDEX), `u` (UNIQUE constraint), `pk` (PRIMARY KEY constraint).
    pub origin: String,
    pub partial: bool,
}

/// One row of `pragma_foreign_key_list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKeyColumn {
    pub id: i64,
    pub seq: i64,
    pub parent: String,
    pub from: String,
    pub to: Option<String>,
    pub on_update: String,
    pub on_delete: String,
}

/// Columns the Rust code always generates itself (the Rust equivalent of a Drizzle
/// `$defaultFn`): every snowflake `id`.
pub const APP_GENERATED_COLUMNS: &[&str] = &["id"];

/// `pragma_table_xinfo(t)`, in column order.
pub async fn columns(db: &Db, table: &str) -> Vec<Column> {
    let table = table.to_owned();
    db.read(move |conn| {
        let mut stmt = conn.prepare(
            r#"SELECT cid, name, type, "notnull", dflt_value, pk, hidden FROM pragma_table_xinfo(?1) ORDER BY cid"#,
        )?;
        let rows = stmt
            .query_map(params![table], |row| {
                Ok(Column {
                    cid: row.get(0)?,
                    name: row.get(1)?,
                    decl_type: row.get(2)?,
                    notnull: row.get::<_, i64>(3)? != 0,
                    dflt_value: row.get(4)?,
                    pk: row.get(5)?,
                    hidden: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
    .expect("pragma_table_xinfo")
}

/// The column `name` of `table`; panics if it does not exist.
pub async fn column(db: &Db, table: &str, name: &str) -> Column {
    columns(db, table)
        .await
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("column {table}.{name} does not exist"))
}

/// Primary key columns of `table`, in key order (`pk > 0` ordered by `pk`).
pub async fn primary_key(db: &Db, table: &str) -> Vec<String> {
    let mut cols: Vec<Column> = columns(db, table)
        .await
        .into_iter()
        .filter(|c| c.pk > 0)
        .collect();
    cols.sort_by_key(|c| c.pk);
    cols.into_iter().map(|c| c.name).collect()
}

/// `pragma_index_list(t)`.
pub async fn indexes(db: &Db, table: &str) -> Vec<Index> {
    let table = table.to_owned();
    db.read(move |conn| {
        let mut stmt = conn.prepare(
            r#"SELECT name, "unique", origin, partial FROM pragma_index_list(?1) ORDER BY name"#,
        )?;
        let rows = stmt
            .query_map(params![table], |row| {
                Ok(Index {
                    name: row.get(0)?,
                    unique: row.get::<_, i64>(1)? != 0,
                    origin: row.get(2)?,
                    partial: row.get::<_, i64>(3)? != 0,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
    .expect("pragma_index_list")
}

/// The index `name` of `table`; panics if it does not exist.
pub async fn index(db: &Db, table: &str, name: &str) -> Index {
    indexes(db, table)
        .await
        .into_iter()
        .find(|i| i.name == name)
        .unwrap_or_else(|| panic!("index {name} on {table} does not exist"))
}

/// Key columns of index `name` (`pragma_index_info`), in key order.
/// An expression key is `None`.
pub async fn index_columns(db: &Db, name: &str) -> Vec<Option<String>> {
    let name = name.to_owned();
    db.read(move |conn| {
        let mut stmt = conn.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")?;
        let rows = stmt
            .query_map(params![name], |row| row.get::<_, Option<String>>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
    .expect("pragma_index_info")
}

/// `pragma_foreign_key_list(t)`, ordered by (`id`, `seq`).
pub async fn fks(db: &Db, table: &str) -> Vec<ForeignKeyColumn> {
    let table = table.to_owned();
    db.read(move |conn| {
        let mut stmt = conn.prepare(
            r#"SELECT id, seq, "table", "from", "to", on_update, on_delete FROM pragma_foreign_key_list(?1) ORDER BY id, seq"#,
        )?;
        let rows = stmt
            .query_map(params![table], |row| {
                Ok(ForeignKeyColumn {
                    id: row.get(0)?,
                    seq: row.get(1)?,
                    parent: row.get(2)?,
                    from: row.get(3)?,
                    to: row.get(4)?,
                    on_update: row.get(5)?,
                    on_delete: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
    .expect("pragma_foreign_key_list")
}

/// The FKs of `table` as column lists (one entry per FK, columns in `seq` order).
pub async fn fk_column_lists(db: &Db, table: &str) -> Vec<Vec<String>> {
    let mut out: Vec<(i64, Vec<String>)> = Vec::new();
    for fk in fks(db, table).await {
        match out.last_mut() {
            Some((id, cols)) if *id == fk.id => cols.push(fk.from),
            _ => out.push((fk.id, vec![fk.from])),
        }
    }
    out.into_iter().map(|(_, cols)| cols).collect()
}

/// `sqlite_schema.sql` of the table (or virtual table) `name`.
pub async fn table_sql(db: &Db, name: &str) -> Option<String> {
    schema_sql(db, "table", name).await
}

/// `sqlite_schema.sql` of the object `name` of `kind` (`table`, `index`, `trigger`, `view`).
pub async fn schema_sql(db: &Db, kind: &str, name: &str) -> Option<String> {
    let kind = kind.to_owned();
    let name = name.to_owned();
    db.read(move |conn| {
        let mut stmt =
            conn.prepare("SELECT sql FROM sqlite_schema WHERE type = ?1 AND name = ?2")?;
        let mut rows = stmt.query(params![kind, name])?;
        match rows.next()? {
            Some(row) => Ok(row.get::<_, Option<String>>(0)?),
            None => Ok(None),
        }
    })
    .await
    .expect("sqlite_schema")
}

/// The `sql` of every explicit index on `table` (autoindexes have no SQL).
pub async fn index_sqls(db: &Db, table: &str) -> Vec<(String, String)> {
    let table = table.to_owned();
    db.read(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT name, sql FROM sqlite_schema WHERE type = 'index' AND tbl_name = ?1 AND sql IS NOT NULL ORDER BY name",
        )?;
        let rows = stmt
            .query_map(params![table], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .await
    .expect("sqlite_schema indexes")
}

/// Names of all ordinary tables (no virtual tables, no `sqlite_*`, no FTS5 shadow tables,
/// no `schema_migrations`).
pub async fn table_names(db: &Db) -> Vec<String> {
    db.read(|conn| {
        let mut stmt = conn.prepare(
            "SELECT name FROM sqlite_schema \
             WHERE type = 'table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
               AND name <> 'schema_migrations' \
               AND sql NOT LIKE 'CREATE VIRTUAL TABLE%' \
             ORDER BY name",
        )?;
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        // FTS5 shadow tables: <fts>_data, _idx, _content, _docsize, _config.
        let mut vstmt = conn.prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND sql LIKE 'CREATE VIRTUAL TABLE%'")?;
        let virtuals = vstmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(names
            .into_iter()
            .filter(|n| !virtuals.iter().any(|v| n.starts_with(&format!("{v}_"))))
            .collect())
    })
    .await
    .expect("table names")
}

/// Columns an insert must supply: `notnull = 1 AND dflt_value IS NULL AND hidden = 0`,
/// minus [`APP_GENERATED_COLUMNS`]. Sorted (same as `requiredColumns` in
/// `insert-required-columns.test.ts`).
pub async fn required_columns(db: &Db, table: &str) -> Vec<String> {
    let mut out: Vec<String> = columns(db, table)
        .await
        .into_iter()
        .filter(|c| c.notnull && c.dflt_value.is_none() && c.hidden == 0)
        .filter(|c| !APP_GENERATED_COLUMNS.contains(&c.name.as_str()))
        .map(|c| c.name)
        .collect();
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// SQL text helpers
// ---------------------------------------------------------------------------

/// The text inside the parentheses of `CONSTRAINT "<name>" CHECK (...)` in `sql`
/// (balanced parentheses, quotes respected). `None` if the constraint is not there.
pub fn named_check_text(sql: &str, name: &str) -> Option<String> {
    let head = format!("CONSTRAINT \"{name}\" CHECK");
    let start = sql.find(&head)? + head.len();
    balanced_parens(&sql[start..])
}

/// The text of the `IN (...)` list of the column CHECK `"<column>" IN (...)` in `sql`.
pub fn column_in_list(sql: &str, column: &str) -> Option<String> {
    let head = format!("\"{column}\" IN");
    let start = sql.find(&head)? + head.len();
    balanced_parens(&sql[start..])
}

/// From the first `(` in `s`, returns the text up to its matching `)` (exclusive).
fn balanced_parens(s: &str) -> Option<String> {
    let open = s.find('(')?;
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for (i, ch) in s[open..].char_indices() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                }
            }
            None => match ch {
                '\'' | '"' => quote = Some(ch),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(s[open + 1..open + i].to_owned());
                    }
                }
                _ => {}
            },
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// Ids of a minimal parent graph: User -> Workspace -> Inbox, Contact, OAuthClient.
#[derive(Debug, Clone, Copy)]
pub struct Seed {
    pub user_id: i64,
    pub workspace_id: i64,
    pub inbox_id: i64,
    pub contact_id: i64,
}

/// `OAuthClient.clientId` of the seeded OAuth client.
pub const SEED_CLIENT_ID: &str = "client-1";

/// Inserts (and commits) one User, Workspace, Inbox, Contact and OAuthClient
/// with only the columns that have no default.
pub async fn seed(db: &Db) -> Seed {
    let ids = Seed {
        user_id: 1001,
        workspace_id: 2001,
        inbox_id: 3001,
        contact_id: 4001,
    };
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "email") VALUES (?1, 'owner@example.com')"#,
            params![ids.user_id],
        )?;
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'Workspace', ?2)"#,
            params![ids.workspace_id, ids.user_id],
        )?;
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES (?1, 'Inbox', 'messenger', 'page-1', ?2)"#,
            params![ids.inbox_id, ids.workspace_id],
        )?;
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (?1, 'Ada', ?2)"#,
            params![ids.contact_id, ids.workspace_id],
        )?;
        tx.execute(
            r#"INSERT INTO "OAuthClient" ("id", "clientId", "redirectUris") VALUES (5001, ?1, '[]')"#,
            params![SEED_CLIENT_ID],
        )?;
        Ok(())
    })
    .await
    .expect("seed rows");
    ids
}

/// Inserts one `ContactInbox` row of the seeded contact/inbox.
pub fn insert_contact_inbox(
    tx: &rusqlite::Connection,
    seed: Seed,
    id: i64,
    source_id: &str,
    source_parent_user_id: Option<&str>,
    source_identity_history: Option<&str>,
) -> Result<usize, DbError> {
    Ok(tx.execute(
        r#"INSERT INTO "ContactInbox"
             ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId",
              "sourceParentUserId", "sourceIdentityHistory")
           VALUES (?1, ?2, ?2, ?3, 'messenger', 'messenger', ?4, ?5, ?6)"#,
        params![
            id,
            seed.contact_id,
            seed.inbox_id,
            source_id,
            source_parent_user_id,
            source_identity_history
        ],
    )?)
}

/// Runs `sql` with one integer/text result row on a reader.
pub async fn query_text(db: &Db, sql: &str, args: Vec<rusqlite::types::Value>) -> Option<String> {
    let sql = sql.to_owned();
    db.read(move |conn| {
        Ok(
            conn.query_row(&sql, rusqlite::params_from_iter(args.iter()), |row| {
                row.get::<_, Option<String>>(0)
            })?,
        )
    })
    .await
    .expect("query_text")
}

/// Runs `sql` with one integer result row on a reader.
pub async fn query_i64(db: &Db, sql: &str, args: Vec<rusqlite::types::Value>) -> i64 {
    let sql = sql.to_owned();
    db.read(move |conn| {
        Ok(
            conn.query_row(&sql, rusqlite::params_from_iter(args.iter()), |row| {
                row.get::<_, i64>(0)
            })?,
        )
    })
    .await
    .expect("query_i64")
}

// ---------------------------------------------------------------------------
// Part-2 rows (P2.4)
// ---------------------------------------------------------------------------

/// Inserts one `Conversation` of the seeded contact (P2.4; table of `0002_messaging.sql`).
/// `source_id = None` is the contact's DM conversation.
pub fn insert_conversation(
    tx: &rusqlite::Connection,
    seed: Seed,
    id: i64,
    source_id: Option<&str>,
) -> Result<usize, DbError> {
    Ok(tx.execute(
        r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "sourceId")
           VALUES (?1, ?2, ?3, ?4)"#,
        params![id, seed.workspace_id, seed.contact_id, source_id],
    )?)
}

/// Inserts (and commits) one more `User` and a `Workspace` that it owns. Returns
/// `(user_id, workspace_id)`.
pub async fn seed_workspace(db: &Db, user_id: i64, workspace_id: i64) -> (i64, i64) {
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "email") VALUES (?1, ?2)"#,
            params![user_id, format!("owner-{user_id}@example.com")],
        )?;
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, ?2, ?3)"#,
            params![workspace_id, format!("Workspace {workspace_id}"), user_id],
        )?;
        Ok(())
    })
    .await
    .expect("seed workspace");
    (user_id, workspace_id)
}

/// The values of a CHECK `IN (...)` list text (from [`column_in_list`]), in order:
/// `'a', 'b'` -> `["a", "b"]`. Panics on an item that is not a single-quoted literal.
pub fn in_list_values(list: &str) -> Vec<String> {
    list.split(',')
        .map(str::trim)
        .map(|item| {
            item.strip_prefix('\'')
                .and_then(|i| i.strip_suffix('\''))
                .unwrap_or_else(|| panic!("not a string literal in the IN list: {item}"))
                .replace("''", "'")
        })
        .collect()
}
