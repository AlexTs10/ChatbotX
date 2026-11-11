//! Port of `packages/database/__tests__/integration/broadcast-recurrence-constraints.test.ts`
//! (P2.4 spec section 3.1, rows 8.1-8.7). Class C: runs against a migrated SQLite file.
//!
//! The original runs all cases in file order inside one rolled-back transaction on the first
//! workspace of a real database; later cases see rows of earlier ones (8.4 relies on the run of
//! 8.3). Here each test seeds its own database (`seed()` + the 2 parent broadcasts + the
//! recurrence of `beforeAll`) and adds the rows it relies on itself (spec doubt 7), so no
//! test depends on order. Every statement that must fail runs as its own write job (the job
//! rolls back, like `ROLLBACK TO SAVEPOINT attempt`); its `DbError` carries the mapped
//! SQLSTATE and constraint name. The assertions are the original ones.

mod common;

use chatbotx_db::rusqlite::{self, params};
use chatbotx_db::{Db, DbError};
use common::{TestDb, fresh_db, seed};

const PARENT_ID: i64 = 990_000_000_000_001;
const OTHER_PARENT_ID: i64 = 990_000_000_000_002;
const RECURRENCE_ID: i64 = 990_000_000_000_011;
/// `2026-10-15T02:00:00.000Z` in unix ms.
const SLOT: i64 = 1_792_029_600_000;
/// `2026-10-16T02:00:00.000Z` in unix ms.
const NEXT_DAY_SLOT: i64 = 1_792_116_000_000;
/// `2026-10-17T13:00:00.000Z` in unix ms.
const OTHER_ZONE_SLOT: i64 = 1_792_242_000_000;
/// `now()` of the original, as unix ms.
const NOW_MS: &str = "CAST(unixepoch('subsec') * 1000 AS INTEGER)";

/// The `insertBroadcast` of the original (`schedulesAt = now()`).
fn insert_broadcast(
    tx: &rusqlite::Connection,
    workspace_id: i64,
    id: i64,
    parent: Option<i64>,
    occurrence_at: Option<i64>,
) -> Result<(), DbError> {
    tx.execute(
        &format!(
            r#"INSERT INTO "Broadcast"
                 ("id", "name", "workspaceId", "status", "schedulesType", "schedulesAt",
                  "subaction", "channel", "parentBroadcastId", "occurrenceAt")
               VALUES (?1, ?2, ?3, 'scheduled', 'future', {NOW_MS}, 'allContacts',
                  'omnichannel', ?4, ?5)"#
        ),
        params![id, format!("b-{id}"), workspace_id, parent, occurrence_at],
    )?;
    Ok(())
}

/// The `insertLedgerRow` of the original (`outcome = 'skipped'`, `materializedAt = now()`).
fn insert_ledger_row(
    tx: &rusqlite::Connection,
    workspace_id: i64,
    id: i64,
    local_slot: &str,
    occurrence_at: i64,
) -> Result<(), DbError> {
    tx.execute(
        &format!(
            r#"INSERT INTO "BroadcastRecurrenceOccurrence"
                 ("id", "workspaceId", "recurrenceId", "localSlot", "occurrenceAt",
                  "outcome", "materializedAt")
               VALUES (?1, ?2, ?3, ?4, ?5, 'skipped', {NOW_MS})"#
        ),
        params![id, workspace_id, RECURRENCE_ID, local_slot, occurrence_at],
    )?;
    Ok(())
}

/// The `beforeAll` of the original: a workspace, 2 parent broadcasts, 1 recurrence of `PARENT_ID`.
async fn setup() -> (TestDb, i64) {
    let db = fresh_db().await;
    let ws = seed(&db).await.workspace_id;
    db.write(move |tx| {
        insert_broadcast(tx, ws, PARENT_ID, None, None)?;
        insert_broadcast(tx, ws, OTHER_PARENT_ID, None, None)?;
        tx.execute(
            r#"INSERT INTO "BroadcastRecurrence" ("id", "workspaceId", "broadcastId", "rule")
               VALUES (?1, ?2, ?3, '{}')"#,
            params![RECURRENCE_ID, ws, PARENT_ID],
        )?;
        Ok(())
    })
    .await
    .expect("seed broadcasts and recurrence");
    (db, ws)
}

/// Runs one statement that must succeed, as its own committed write job.
async fn accepts<F>(db: &Db, what: &str, statement: F)
where
    F: FnOnce(&rusqlite::Connection) -> Result<(), DbError> + Send + 'static,
{
    db.write(move |tx| statement(tx))
        .await
        .unwrap_or_else(|e| panic!("{what} must succeed: {e}"));
}

/// Runs one statement that must fail, as its own write job (rolled back). Returns the error.
async fn rejects<F>(db: &Db, what: &str, statement: F) -> DbError
where
    F: FnOnce(&rusqlite::Connection) -> Result<(), DbError> + Send + 'static,
{
    db.write(move |tx| statement(tx))
        .await
        .err()
        .unwrap_or_else(|| panic!("{what} must be rejected"))
}

/// Port of: packages/database/__tests__/integration/broadcast-recurrence-constraints.test.ts:102 "repeating broadcast constraints > a run needs the slot it was created for"
#[tokio::test]
async fn a_run_needs_the_slot_it_was_created_for() {
    let (db, ws) = setup().await;
    let error = rejects(&db, "a run without occurrenceAt", move |tx| {
        insert_broadcast(tx, ws, 990_000_000_000_101, Some(PARENT_ID), None)
    })
    .await;
    assert_eq!(error.sqlstate(), Some("23514"), "{error}");
    assert_eq!(
        error.constraint(),
        Some("Broadcast_occurrence_pair_check"),
        "{error}"
    );
}

/// Port of: packages/database/__tests__/integration/broadcast-recurrence-constraints.test.ts:113 "repeating broadcast constraints > a one-off, and a run whose parent was purged, both pass the check"
#[tokio::test]
async fn a_one_off_and_a_run_whose_parent_was_purged_both_pass_the_check() {
    let (db, ws) = setup().await;
    accepts(&db, "a one-off", move |tx| {
        insert_broadcast(tx, ws, 990_000_000_000_102, None, None)
    })
    .await;
    accepts(&db, "a run whose parent was purged", move |tx| {
        insert_broadcast(tx, ws, 990_000_000_000_103, None, Some(SLOT))
    })
    .await;
}

/// Port of: packages/database/__tests__/integration/broadcast-recurrence-constraints.test.ts:121 "repeating broadcast constraints > a slot gets one run: the same parent and instant is rejected"
#[tokio::test]
async fn a_slot_gets_one_run_the_same_parent_and_instant_is_rejected() {
    let (db, ws) = setup().await;
    accepts(&db, "the first run of the slot", move |tx| {
        insert_broadcast(tx, ws, 990_000_000_000_104, Some(PARENT_ID), Some(SLOT))
    })
    .await;

    let error = rejects(&db, "a second run of the same slot", move |tx| {
        insert_broadcast(tx, ws, 990_000_000_000_105, Some(PARENT_ID), Some(SLOT))
    })
    .await;
    assert_eq!(error.sqlstate(), Some("23505"), "{error}");
    assert_eq!(
        error.constraint(),
        Some("Broadcast_parentBroadcastId_occurrenceAt_key"),
        "{error}"
    );
}

/// Port of: packages/database/__tests__/integration/broadcast-recurrence-constraints.test.ts:142 "repeating broadcast constraints > another series may run at the same instant"
///
/// The run of `PARENT_ID` at `SLOT` (from `:121` in the original's shared transaction) is
/// inserted first, so the other series really shares the instant.
#[tokio::test]
async fn another_series_may_run_at_the_same_instant() {
    let (db, ws) = setup().await;
    accepts(&db, "the run of PARENT_ID at SLOT", move |tx| {
        insert_broadcast(tx, ws, 990_000_000_000_104, Some(PARENT_ID), Some(SLOT))
    })
    .await;
    accepts(&db, "the run of OTHER_PARENT_ID at SLOT", move |tx| {
        insert_broadcast(
            tx,
            ws,
            990_000_000_000_106,
            Some(OTHER_PARENT_ID),
            Some(SLOT),
        )
    })
    .await;
}

/// Port of: packages/database/__tests__/integration/broadcast-recurrence-constraints.test.ts:152 "repeating broadcast constraints > purging the parent keeps the run, still marked as one"
#[tokio::test]
async fn purging_the_parent_keeps_the_run_still_marked_as_one() {
    let (db, ws) = setup().await;
    accepts(&db, "a run of OTHER_PARENT_ID", move |tx| {
        insert_broadcast(
            tx,
            ws,
            990_000_000_000_107,
            Some(OTHER_PARENT_ID),
            Some(NEXT_DAY_SLOT),
        )
    })
    .await;

    accepts(&db, "deleting the parent", |tx| {
        tx.execute(
            r#"DELETE FROM "Broadcast" WHERE "id" = ?1"#,
            params![OTHER_PARENT_ID],
        )?;
        Ok(())
    })
    .await;

    let (parent, occurrence_at): (Option<i64>, Option<i64>) = db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "parentBroadcastId", "occurrenceAt" FROM "Broadcast" WHERE "id" = ?1"#,
                params![990_000_000_000_107_i64],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .await
        .expect("the run must still exist");
    assert_eq!(parent, None, "parentBroadcastId must be NULL");
    assert!(occurrence_at.is_some(), "occurrenceAt must not be NULL");
}

/// Port of: packages/database/__tests__/integration/broadcast-recurrence-constraints.test.ts:173 "repeating broadcast constraints > the ledger holds one row per logical slot"
#[tokio::test]
async fn the_ledger_holds_one_row_per_logical_slot() {
    let (db, ws) = setup().await;
    accepts(&db, "the first ledger row", move |tx| {
        insert_ledger_row(tx, ws, 990_000_000_001_001, "2026-10-15 09:00", SLOT)
    })
    .await;

    let error = rejects(&db, "a second row of the same slot", move |tx| {
        insert_ledger_row(tx, ws, 990_000_000_001_002, "2026-10-15 09:00", SLOT)
    })
    .await;
    assert_eq!(error.sqlstate(), Some("23505"), "{error}");

    accepts(&db, "a row of another slot", move |tx| {
        insert_ledger_row(tx, ws, 990_000_000_001_003, "2026-10-16 09:00", SLOT)
    })
    .await;
}

/// Port of: packages/database/__tests__/integration/broadcast-recurrence-constraints.test.ts:185 "repeating broadcast constraints > the same wall-clock text at another instant is a different slot (a timezone edit)"
#[tokio::test]
async fn the_same_wall_clock_text_at_another_instant_is_a_different_slot_a_timezone_edit() {
    let (db, ws) = setup().await;
    accepts(&db, "the first ledger row", move |tx| {
        insert_ledger_row(tx, ws, 990_000_000_001_004, "2026-10-17 09:00", SLOT)
    })
    .await;

    // Same text, now in another zone: a different instant must still be recorded.
    accepts(&db, "the same text at another instant", move |tx| {
        insert_ledger_row(
            tx,
            ws,
            990_000_000_001_005,
            "2026-10-17 09:00",
            OTHER_ZONE_SLOT,
        )
    })
    .await;

    let error = rejects(&db, "a repeat of the same text and instant", move |tx| {
        insert_ledger_row(
            tx,
            ws,
            990_000_000_001_006,
            "2026-10-17 09:00",
            OTHER_ZONE_SLOT,
        )
    })
    .await;
    assert_eq!(error.sqlstate(), Some("23505"), "{error}");
}
