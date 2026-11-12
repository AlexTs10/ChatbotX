//! Shared helpers for the ported repository tests of P3.2 (`conversion/specs/P3.2.md`).
//!
//! Include with:
//! ```ignore
//! mod common;
//! #[macro_use]
//! #[path = "common/repo.rs"]
//! mod repo_common;
//! ```

#![allow(dead_code, unused_macros, unused_imports)]

use chatbotx_db::repo::{self, Error};
use chatbotx_db::rusqlite::types::{ToSql, Value};
use chatbotx_db::rusqlite::{self, Connection};
use chatbotx_db::{Db, DbError};

/// `ins!(tx, "Table", { "col" => value, ... })`: inserts one row (values are owned
/// `ToSql` values). Returns `Result<usize, DbError>`.
macro_rules! ins {
    ($tx:expr, $table:literal, { $($col:literal => $val:expr),* $(,)? }) => {{
        let cols: &[&str] = &[$($col),*];
        let vals: Vec<Box<dyn chatbotx_db::rusqlite::types::ToSql>> = vec![$(Box::new($val)),*];
        $crate::repo_common::insert_row($tx, $table, cols, &vals)
    }};
}

/// Inserts one row of `table` with `cols` = `vals`.
pub fn insert_row(
    conn: &Connection,
    table: &str,
    cols: &[&str],
    vals: &[Box<dyn ToSql>],
) -> Result<usize, DbError> {
    let names: Vec<String> = cols.iter().map(|c| format!("\"{c}\"")).collect();
    let marks: Vec<String> = (1..=cols.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "INSERT INTO \"{table}\" ({}) VALUES ({})",
        names.join(", "),
        marks.join(", ")
    );
    let params: Vec<&dyn ToSql> = vals.iter().map(|v| v.as_ref()).collect();
    Ok(conn.execute(&sql, params.as_slice())?)
}

/// Runs `f` in one committed write job and panics on error (test setup).
pub async fn setup<F, R>(db: &Db, f: F) -> R
where
    F: FnOnce(&Connection) -> Result<R, DbError> + Send + 'static,
    R: Send + 'static,
{
    db.write(move |tx| f(tx)).await.expect("test setup write")
}

/// One integer cell (`SELECT ...` with one row) on a reader; `None` for no row / NULL.
pub async fn scalar_i64(db: &Db, sql: &str, args: Vec<Value>) -> Option<i64> {
    let sql = sql.to_owned();
    db.read(move |conn| {
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(args.iter()))?;
        match rows.next()? {
            Some(row) => Ok(row.get::<_, Option<i64>>(0)?),
            None => Ok(None),
        }
    })
    .await
    .expect("scalar_i64")
}

/// One text cell on a reader; `None` for no row / NULL.
pub async fn scalar_text(db: &Db, sql: &str, args: Vec<Value>) -> Option<String> {
    let sql = sql.to_owned();
    db.read(move |conn| {
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(args.iter()))?;
        match rows.next()? {
            Some(row) => Ok(row.get::<_, Option<String>>(0)?),
            None => Ok(None),
        }
    })
    .await
    .expect("scalar_text")
}

/// All rows of `sql` as `Vec<Vec<Value>>` on a reader.
pub async fn rows(db: &Db, sql: &str, args: Vec<Value>) -> Vec<Vec<Value>> {
    let sql = sql.to_owned();
    db.read(move |conn| {
        let mut stmt = conn.prepare(&sql)?;
        let n = stmt.column_count();
        let out = stmt
            .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                (0..n).map(|i| row.get::<_, Value>(i)).collect()
            })?
            .collect::<rusqlite::Result<Vec<Vec<Value>>>>()?;
        Ok(out)
    })
    .await
    .expect("rows")
}

/// `Value::Integer`.
pub fn int(v: i64) -> Value {
    Value::Integer(v)
}

/// `Value::Text`.
pub fn text(v: &str) -> Value {
    Value::Text(v.to_owned())
}

/// Seeds one more Inbox of `workspace_id` (`channel`, `sourceId` = `source_id`).
pub fn insert_inbox(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    channel: &str,
    source_id: &str,
) -> Result<usize, DbError> {
    ins!(tx, "Inbox", {
        "id" => id,
        "name" => format!("Inbox {id}"),
        "channel" => channel.to_owned(),
        "sourceId" => source_id.to_owned(),
        "workspaceId" => workspace_id,
    })
}

/// Seeds one Contact of `workspace_id`.
pub fn insert_contact(tx: &Connection, id: i64, workspace_id: i64) -> Result<usize, DbError> {
    ins!(tx, "Contact", { "id" => id, "workspaceId" => workspace_id })
}

/// Seeds one ContactInbox (`contact_id` on `inbox_id`, `channel`, `sourceId`).
pub fn insert_ci(
    tx: &Connection,
    id: i64,
    contact_id: i64,
    inbox_id: i64,
    channel: &str,
    source_id: &str,
) -> Result<usize, DbError> {
    ins!(tx, "ContactInbox", {
        "id" => id,
        "originalContactId" => contact_id,
        "contactId" => contact_id,
        "inboxId" => inbox_id,
        "channel" => channel.to_owned(),
        "source" => channel.to_owned(),
        "sourceId" => source_id.to_owned(),
    })
}

/// Seeds one Conversation of `contact_id` in `workspace_id` (`sourceId` NULL = DM).
pub fn insert_conv(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    contact_id: i64,
    source_id: Option<&str>,
) -> Result<usize, DbError> {
    ins!(tx, "Conversation", {
        "id" => id,
        "workspaceId" => workspace_id,
        "contactId" => contact_id,
        "sourceId" => source_id.map(str::to_owned),
    })
}

/// Seeds one Flow of `workspace_id`.
pub fn insert_flow(tx: &Connection, id: i64, workspace_id: i64) -> Result<usize, DbError> {
    ins!(tx, "Flow", { "id" => id, "name" => format!("Flow {id}"), "workspaceId" => workspace_id })
}

/// Unwraps a repository result inside a test.
pub fn ok<T>(r: Result<T, Error>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("repository call failed: {e}"),
    }
}

/// The message of a repository error (panics on `Ok`).
pub fn err_message<T: std::fmt::Debug>(r: Result<T, Error>) -> String {
    match r {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => e.message(),
    }
}

/// A fixed "now" for tests: `2026-06-01T00:00:00.000Z`.
pub const NOW: i64 = 1_780_272_000_000;

/// `Date.parse(iso)` (panics on bad input).
pub fn ms(iso: &str) -> i64 {
    repo::parse_iso(iso).unwrap_or_else(|| panic!("bad iso {iso}"))
}

/// Fixture of the AI file embedding ports (two P3.2 test files).
pub mod ai_file {

    use chatbotx_db::Db;
    use chatbotx_db::rusqlite::Connection;
    use serde_json::{Value, json};

    pub const NOW: &str = "2026-10-24T10:00:00.000Z";
    pub const STALE_BEFORE: &str = "2026-10-24T09:00:00.000Z";
    pub const OLD: &str = "2026-10-24T08:00:00.000Z";
    pub const FRESH: &str = "2026-10-24T09:30:00.000Z";

    /// The seeded workspace and file.
    pub const WS: i64 = 2001;
    pub const FILE: i64 = 70;

    /// `buildRun(overrides)`.
    pub fn build_run(overrides: Value) -> Value {
        let mut run = json!({
            "id": "run-active",
            "status": "queued",
            "error": null,
            "queuedAt": NOW,
            "startedAt": null,
            "heartbeatAt": null,
            "finishedAt": null,
        });
        if let (Some(base), Some(extra)) = (run.as_object_mut(), overrides.as_object()) {
            for (k, v) in extra {
                base.insert(k.clone(), v.clone());
            }
        }
        run
    }

    /// A fresh database with the common seed and one AI file (`processingRun` = `run`).
    pub async fn fixture(run: Option<Value>) -> crate::common::TestDb {
        let db = crate::common::fresh_db().await;
        crate::common::seed(&db).await;
        crate::repo_common::setup(&db, move |tx| {
            ins!(tx, "AIFile", {
                "id" => FILE, "name" => "prices.csv", "path" => "ai-file-run/prices.csv",
                "size" => 10_i64, "mimeType" => "text/csv", "workspaceId" => WS,
                "processingRun" => run.map(|r| r.to_string()),
            })?;
            Ok(())
        })
        .await;
        db
    }

    /// Inserts chunks `(id, chunkIndex, content)` with `status` of the seeded file.
    pub fn insert_chunks(tx: &Connection, rows: &[(i64, Option<i64>, &str, &str)]) {
        for (id, index, content, status) in rows {
            ins!(tx, "AIEmbedding", {
                "id" => *id, "chunkIndex" => *index, "status" => (*status).to_owned(),
                "content" => (*content).to_owned(), "aiFileId" => FILE, "workspaceId" => WS,
            })
            .expect("chunk");
        }
    }

    /// `processingRun` of the seeded file.
    pub async fn read_run(db: &Db) -> Option<Value> {
        crate::repo_common::scalar_text(
            db,
            r#"SELECT "processingRun" FROM "AIFile" WHERE "id" = 70"#,
            vec![],
        )
        .await
        .map(|t| serde_json::from_str(&t).expect("json"))
    }

    /// `chunkingConfig` of the seeded file.
    pub async fn read_config(db: &Db) -> Option<Value> {
        crate::repo_common::scalar_text(
            db,
            r#"SELECT "chunkingConfig" FROM "AIFile" WHERE "id" = 70"#,
            vec![],
        )
        .await
        .map(|t| serde_json::from_str(&t).expect("json"))
    }

    /// `(id, chunkIndex, status)` of every chunk of the seeded file, by id.
    pub async fn chunk_rows(db: &Db) -> Vec<(i64, Option<i64>, String)> {
        crate::repo_common::rows(
            db,
            r#"SELECT "id", "chunkIndex", "status" FROM "AIEmbedding" WHERE "aiFileId" = 70 ORDER BY "id""#,
            vec![],
        )
        .await
        .into_iter()
        .map(|r| {
            use chatbotx_db::rusqlite::types::Value as V;
            let id = match &r[0] {
                V::Integer(i) => *i,
                other => panic!("{other:?}"),
            };
            let index = match &r[1] {
                V::Integer(i) => Some(*i),
                _ => None,
            };
            let status = match &r[2] {
                V::Text(t) => t.clone(),
                other => panic!("{other:?}"),
            };
            (id, index, status)
        })
        .collect()
    }
}

/// Fixture of the AI handover ports (three P3.2 test files): the common seed workspace
/// 2001 with its connected messenger inbox 3001 (the "Page").
pub mod handover {
    use chatbotx_db::Db;
    use chatbotx_db::repo::ai_handover_bulk_run::{self as runs, CreateAiHandoverBulkRunInput};
    use chatbotx_db::repo::models::AiHandoverBulkRun;
    use chatbotx_db::repo::{self, now_ms};

    pub const WS: i64 = 2001;
    pub const INBOX: i64 = 3001;
    pub const MINUTE: i64 = 60_000;

    pub async fn page() -> crate::common::TestDb {
        let db = crate::common::fresh_db().await;
        crate::common::seed(&db).await;
        db
    }

    pub fn create_input(revision: i64, action: &str) -> CreateAiHandoverBulkRunInput {
        CreateAiHandoverBulkRunInput {
            workspace_id: WS,
            inbox_id: INBOX,
            channel: "messenger".into(),
            revision,
            action: action.into(),
            message: (action == "disable").then(|| "hello".to_owned()),
            requested_by_user_id: None,
            requested_at: now_ms(),
        }
    }

    /// `create(tx, ref, revision, action)`.
    pub async fn create(db: &Db, revision: i64, action: &str) -> Option<AiHandoverBulkRun> {
        let input = create_input(revision, action);
        crate::repo_common::ok(
            repo::write(db, move |tx| runs::create_for_revision(tx, &input)).await,
        )
    }

    /// Runs one SQL statement (test setup, like the TS `tx.execute(sql...)`).
    pub async fn exec(db: &Db, sql: &str, args: Vec<chatbotx_db::rusqlite::types::Value>) {
        let sql = sql.to_owned();
        db.write(move |tx| {
            Ok(tx.execute(&sql, chatbotx_db::rusqlite::params_from_iter(args.iter()))?)
        })
        .await
        .expect("exec");
    }

    pub async fn claim(db: &Db, run_id: i64, workspace_id: i64) -> Option<AiHandoverBulkRun> {
        crate::repo_common::ok(
            repo::write(db, move |tx| {
                runs::claim(tx, run_id, workspace_id, now_ms())
            })
            .await,
        )
    }

    pub async fn find(db: &Db, revision: i64) -> Option<AiHandoverBulkRun> {
        crate::repo_common::ok(
            repo::read(db, move |c| runs::find_by_revision(c, WS, INBOX, revision)).await,
        )
    }
}

/// Fixture of the broadcast ports (four P3.2 test files): workspace 2001 (common seed).
pub mod bcast {
    use chatbotx_db::DbError;
    use chatbotx_db::rusqlite::Connection;

    pub const WS: i64 = 2001;

    /// One Broadcast row (`now` schedule, webchat) with the given status.
    pub fn insert(tx: &Connection, id: i64, name: &str, status: &str) -> Result<usize, DbError> {
        ins!(tx, "Broadcast", {
            "id" => id, "name" => name.to_owned(), "workspaceId" => WS, "status" => status.to_owned(),
            "schedulesType" => "now", "schedulesAt" => 1_000_i64, "subaction" => "webchatMessage",
            "channel" => "webchat", "createdAt" => id,
        })
    }

    /// `n` recipients (Contact + ContactInbox + Conversation + ContactOnBroadcast) of the
    /// broadcast on inbox `inbox`; ids from `first`.
    pub fn recipients(
        tx: &Connection,
        broadcast: i64,
        inbox: i64,
        first: i64,
        n: i64,
    ) -> Result<(), DbError> {
        for i in 0..n {
            let base = first + i * 3;
            crate::repo_common::insert_contact(tx, base, WS)?;
            crate::repo_common::insert_ci(
                tx,
                base + 1,
                base,
                inbox,
                "webchat",
                &format!("purge-{broadcast}-{base}"),
            )?;
            crate::repo_common::insert_conv(tx, base + 2, WS, base, None)?;
            ins!(tx, "ContactOnBroadcast", {
                "broadcastId" => broadcast, "contactId" => base, "contactInboxId" => base + 1, "conversationId" => base + 2,
            })?;
        }
        Ok(())
    }
}

/// Fixture of the connect-session ports (three P3.2 test files).
pub mod csess {
    use chatbotx_db::DbError;
    use chatbotx_db::repo::now_ms;
    use chatbotx_db::rusqlite::Connection;

    /// `FUTURE` (10 minutes from now).
    pub fn future() -> i64 {
        now_ms() + 10 * 60 * 1000
    }

    /// `seedSession`: an awaiting-selection messenger session with targets t1, t2.
    pub fn seed_session(
        tx: &Connection,
        id: i64,
        workspace_id: i64,
        actor: i64,
        nonce: &str,
    ) -> Result<i64, DbError> {
        ins!(tx, "ConnectSession", {
            "id" => id, "workspaceId" => workspace_id, "provider" => "messenger", "purpose" => "connect",
            "actorUserId" => actor, "stateNonceHash" => nonce.to_owned(), "status" => "awaiting_selection",
            "step" => "select",
            "targets" => r#"[{"id":"t1","name":"A","selectable":true},{"id":"t2","name":"B","selectable":true}]"#,
            "expiresAt" => future(),
        })?;
        Ok(id)
    }
}

/// Seeds one `Integration` (googleSheets) of `workspace_id`.
pub fn insert_integration(tx: &Connection, id: i64, workspace_id: i64) -> Result<i64, DbError> {
    ins!(tx, "Integration", { "id" => id, "workspaceId" => workspace_id, "integrationType" => "googleSheets" })?;
    Ok(id)
}
