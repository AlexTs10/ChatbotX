//! Shared fixtures of the P3.2b slices of implementer 6 (google-ads-conversion-event and
//! later), on top of `common/mod.rs` and `common/repo.rs` (both locked).
//!
//! Include after `repo_common`:
//! ```ignore
//! #[path = "common/repo11.rs"]
//! mod repo11;
//! ```

#![allow(dead_code)]

use std::sync::atomic::{AtomicI64, Ordering};

use chatbotx_db::Db;
use chatbotx_db::repo::models::GoogleAdsConversionEvent;
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::repo::{self};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value;

use crate::common::{
    ColumnOverride, GOOGLE_ADS_EVENT_OPTIONS, TestDb, fresh_db, google_ads_event_row,
    insert_google_ads_event, seed, seed_workspace,
};

/// The seeded workspace (`seed`, owner = user 1001).
pub const WS: i64 = 2001;
/// A second workspace (owner = user 1002).
pub const OTHER_WS: i64 = 2002;

/// A fresh file with the seed graph and `OTHER_WS`.
pub async fn two_ws_db() -> TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1002, OTHER_WS).await;
    db
}

/// Unix ms of an RFC 3339 instant.
pub fn ms(iso: &str) -> i64 {
    repo::parse_iso(iso).expect("RFC 3339 instant")
}

/// A text override value.
pub fn t(value: &str) -> Option<Value> {
    Some(Value::Text(value.to_owned()))
}

/// An integer override value.
pub fn i(value: i64) -> Option<Value> {
    Some(Value::Integer(value))
}

/// A NULL override value.
pub fn null() -> Option<Value> {
    Some(Value::Null)
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

/// Runs SQL statements in a committed write (test setup).
pub async fn exec(db: &Db, sql: impl Into<String>) {
    let sql = sql.into();
    db.write(move |tx| Ok(tx.execute_batch(&sql)?))
        .await
        .expect("exec");
}

/// Inserts one `GoogleAdsConversionEvent` (`common::google_ads_event_row` with
/// `transactionId` = `t-<id>`, status `pending`) with `overrides`.
pub async fn put_event(db: &Db, id: i64, workspace_id: i64, overrides: Vec<ColumnOverride>) {
    db.write(move |tx| {
        insert_google_ads_event(tx, id, workspace_id, &format!("t-{id}"), &overrides)?;
        Ok(())
    })
    .await
    .expect("put event");
}

/// Inserts `count` events with ids `first..first + count` of `workspace_id`, each with
/// `overrides`, in one write.
pub async fn put_events(
    db: &Db,
    first: i64,
    count: i64,
    workspace_id: i64,
    overrides: Vec<ColumnOverride>,
) {
    db.write(move |tx| {
        for id in first..first + count {
            insert_google_ads_event(tx, id, workspace_id, &format!("t-{id}"), &overrides)?;
        }
        Ok(())
    })
    .await
    .expect("put events");
}

static STATS_SEQUENCE: AtomicI64 = AtomicI64::new(10_000);

/// The TS `insertEvent` of `integration/google-ads-conversion-event-stats.test.ts`:
/// status `processed`, action `111` "Purchase" / `PURCHASE`, `occurredAt`
/// `2026-10-02T12:00:00Z`; a `sent` / `processed` row gets `requestId` `req-<n>` and
/// `sentAt`. Returns the new id.
pub async fn stats_event(db: &Db, workspace_id: i64, overrides: Vec<ColumnOverride>) -> i64 {
    let id = STATS_SEQUENCE.fetch_add(1, Ordering::SeqCst);
    let status = overrides
        .iter()
        .find(|(name, _)| *name == "status")
        .and_then(|(_, v)| match v {
            Some(Value::Text(s)) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "processed".to_owned());
    let sent_like = status == "sent" || status == "processed";
    let mut all: Vec<ColumnOverride> = vec![
        ("status", t(&status)),
        ("conversionActionName", t("Purchase")),
        ("conversionActionCategory", t("PURCHASE")),
        ("occurredAt", i(ms("2026-10-02T12:00:00Z"))),
        (
            "requestId",
            if sent_like {
                t(&format!("req-{id}"))
            } else {
                null()
            },
        ),
        (
            "sentAt",
            if sent_like {
                i(ms("2026-10-02T12:00:00Z"))
            } else {
                null()
            },
        ),
    ];
    all.extend(overrides);
    db.write(move |tx| {
        insert_google_ads_event(tx, id, workspace_id, &format!("stats-{id}"), &all)?;
        Ok(())
    })
    .await
    .expect("stats event");
    id
}

/// The stored event `id`.
pub async fn event(db: &Db, id: i64) -> Option<GoogleAdsConversionEvent> {
    read(db, move |c| repo::find_one(c, "\"id\" = ?1", &[&id])).await
}

/// The `insertIgnoreDuplicate` values of `common::google_ads_event_row` (no `id`; the
/// `options` snapshot of `GOOGLE_ADS_EVENT_OPTIONS`) with `overrides`.
pub fn event_values(
    workspace_id: i64,
    transaction_id: &str,
    overrides: &[ColumnOverride],
) -> Patch {
    let (columns, values) = google_ads_event_row(0, workspace_id, transaction_id, overrides);
    columns
        .into_iter()
        .zip(values)
        .filter(|(name, _)| *name != "id")
        .collect()
}

/// `GOOGLE_ADS_EVENT_OPTIONS` parsed.
pub fn event_options() -> serde_json::Value {
    serde_json::from_str(GOOGLE_ADS_EVENT_OPTIONS).expect("options")
}
