//! Shared fixtures of the P3.2b coexist-sync-run slice (on top of `common/mod.rs`,
//! `common/repo.rs` and `common/repo3.rs`; those are locked).
//!
//! Include after `repo_common` and `repo3`:
//! ```ignore
//! #[path = "common/repo3.rs"]
//! mod repo3;
//! #[path = "common/repo10.rs"]
//! mod repo10;
//! ```

#![allow(dead_code)]

use chatbotx_db::repo::models::CoexistSyncRun;
use chatbotx_db::repo::{self, Model};
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};

use crate::common::{TestDb, fresh_db, seed, seed_workspace};

/// The seeded workspace.
pub const WS: i64 = 2001;
/// A second workspace (owner = user 1002).
pub const OTHER_WS: i64 = 2002;
/// `IntegrationMessenger` of `WS` (inbox 3061).
pub const MESSENGER: i64 = 61;
/// `IntegrationInstagram` of `WS` (inbox 3062).
pub const INSTAGRAM: i64 = 62;
/// `IntegrationWhatsapp` of `WS` (inbox 3063, `phoneNumberId` = `phone-63`).
pub const WHATSAPP: i64 = 63;
/// A second `IntegrationWhatsapp` of `WS` (inbox 3064, `phoneNumberId` = `phone-64`).
pub const WHATSAPP_2: i64 = 64;
/// `IntegrationWhatsapp` of `OTHER_WS` (inbox 3065, `phoneNumberId` = `phone-65`).
pub const WHATSAPP_OTHER: i64 = 65;

/// A fresh file with the seed graph, `OTHER_WS` and the integrations above.
pub async fn coexist_db() -> TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1002, OTHER_WS).await;
    db.write(|tx| {
        crate::repo3::integration_messenger(tx, MESSENGER, WS, 3061, "[]")?;
        crate::repo3::integration_instagram(tx, INSTAGRAM, WS, 3062)?;
        crate::repo3::integration_whatsapp(tx, WHATSAPP, WS, 3063, "waba-63")?;
        crate::repo3::integration_whatsapp(tx, WHATSAPP_2, WS, 3064, "waba-64")?;
        crate::repo3::integration_whatsapp(tx, WHATSAPP_OTHER, OTHER_WS, 3065, "waba-65")?;
        Ok(())
    })
    .await
    .expect("coexist fixture");
    db
}

/// Inserts one run of `WS` on `integration_id` / `channel` (`kind` = `(type, status)`,
/// `triggerSource` = `manual`, `createdAt` = `updatedAt` = `at`), then applies `set` (an SQL
/// SET list, empty for none).
pub fn put_run(
    tx: &Connection,
    id: i64,
    integration_id: i64,
    channel: &str,
    kind: (&str, &str),
    at: i64,
    set: &str,
) -> Result<usize, DbError> {
    let (run_type, status) = kind;
    tx.execute(
        r#"INSERT INTO "CoexistSyncRun" ("id", "createdAt", "updatedAt", "workspaceId",
               "integrationId", "channel", "type", "status", "triggerSource")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, 'manual')"#,
        params![id, at, WS, integration_id, channel, run_type, status],
    )?;
    if set.is_empty() {
        return Ok(1);
    }
    Ok(tx.execute(
        &format!(r#"UPDATE "CoexistSyncRun" SET {set} WHERE "id" = ?1"#),
        params![id],
    )?)
}

/// One [`put_run`] row: `(id, integrationId, channel, (type, status), at, set)`.
pub type RunSpec<'a> = (i64, i64, &'a str, (&'a str, &'a str), i64, &'a str);

/// An owned [`RunSpec`].
type OwnedSpec = (i64, i64, String, (String, String), i64, String);

/// Inserts `rows` with [`put_run`] in one committed write.
pub async fn put_runs(db: &Db, rows: &[RunSpec<'_>]) {
    let owned: Vec<OwnedSpec> = rows
        .iter()
        .map(|(id, integration, channel, (t, s), at, set)| {
            (
                *id,
                *integration,
                (*channel).to_owned(),
                ((*t).to_owned(), (*s).to_owned()),
                *at,
                (*set).to_owned(),
            )
        })
        .collect();
    db.write(move |tx| {
        for (id, integration, channel, (t, s), at, set) in owned {
            put_run(tx, id, integration, &channel, (&t, &s), at, &set)?;
        }
        Ok(())
    })
    .await
    .expect("put runs");
}

/// A repository read on a reader (panics on error).
pub async fn read<T: Send + 'static>(
    db: &Db,
    f: impl FnOnce(&Connection) -> repo::Result<T> + Send + 'static,
) -> T {
    crate::repo_common::ok(repo::read(db, f).await)
}

/// A repository write in one committed job (panics on error).
pub async fn write<T: Send + 'static>(
    db: &Db,
    f: impl FnOnce(&Connection) -> repo::Result<T> + Send + 'static,
) -> T {
    crate::repo_common::ok(repo::write(db, move |tx| f(tx)).await)
}

/// Inserts one `WhatsappCoexistStaging` row of `phone_number_id`; `state` is `pending`,
/// `processed` or `parse_failed`.
pub fn put_staging(
    tx: &Connection,
    id: i64,
    phone_number_id: &str,
    state: &str,
) -> Result<usize, DbError> {
    let (processed, failed): (Option<i64>, Option<i64>) = match state {
        "processed" => (Some(1), None),
        "parse_failed" => (None, Some(1)),
        _ => (None, None),
    };
    Ok(tx.execute(
        r#"INSERT INTO "WhatsappCoexistStaging" ("id", "phoneNumberId", "payload", "payloadHash",
               "processedAt", "parseFailedAt")
           VALUES (?1, ?2, '{}', ?3, ?4, ?5)"#,
        params![id, phone_number_id, format!("hash-{id}"), processed, failed],
    )?)
}

/// The `CoexistSyncRun` row `id`, if any.
pub async fn run_row(db: &Db, id: i64) -> Option<CoexistSyncRun> {
    repo::read(db, move |c| {
        repo::find_one::<CoexistSyncRun>(c, "\"id\" = ?1", &[&id])
    })
    .await
    .expect("read run")
}

/// Every `CoexistSyncRun` row, by id.
pub async fn run_rows(db: &Db) -> Vec<CoexistSyncRun> {
    repo::read(db, |c| {
        repo::find_many::<CoexistSyncRun>(c, "ORDER BY \"id\"", &[])
    })
    .await
    .expect("read runs")
}

/// Runs one SQL statement in a committed write (test setup).
pub async fn exec(db: &Db, sql: &'static str) {
    db.write(move |tx| Ok(tx.execute_batch(sql)?))
        .await
        .expect("exec");
}

/// `CoexistSyncRun::TABLE` (keeps the `Model` import used).
pub fn run_table() -> &'static str {
    CoexistSyncRun::TABLE
}
