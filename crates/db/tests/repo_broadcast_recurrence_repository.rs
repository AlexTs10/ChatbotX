//! Port of `packages/database/__tests__/broadcast-recurrence-repository.test.ts` (P3.2).
//! The mocked thenable chain becomes real rows of a migrated SQLite file (adapted).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::broadcast_recurrence::{self as rec, LedgerRowInput, RecurrencePatch};
use chatbotx_db::repo::queries::date_bucket::Granularity;
use chatbotx_db::repo::{self, now_ms};
use common::{fresh_db, seed};
use repo_common::bcast::{WS, insert};
use repo_common::{ms, ok};

/// Broadcasts 1..=3 with series 11..=13 (`nextRunAt` 2026-10-13T02:00Z for 11, NULL for 12,
/// 2026-10-13T01:00Z for 13).
async fn series() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    let due = ms("2026-10-13T02:00:00.000Z");
    repo_common::setup(&db, move |tx| {
        for id in 1..=3_i64 {
            insert(tx, id, "t", "recurring")?;
        }
        ins!(tx, "BroadcastRecurrence", { "id" => 11_i64, "workspaceId" => WS, "broadcastId" => 1_i64, "rule" => "{}", "nextRunAt" => due })?;
        ins!(tx, "BroadcastRecurrence", { "id" => 12_i64, "workspaceId" => WS, "broadcastId" => 2_i64, "rule" => "{}" })?;
        ins!(tx, "BroadcastRecurrence", { "id" => 13_i64, "workspaceId" => WS, "broadcastId" => 3_i64, "rule" => "{}", "nextRunAt" => due - 3_600_000 })?;
        Ok(())
    })
    .await;
    db
}

fn ledger(
    slot: &str,
    at: &str,
    outcome: &str,
    child: Option<i64>,
    superseded: Option<i64>,
) -> LedgerRowInput {
    LedgerRowInput {
        workspace_id: WS,
        recurrence_id: 11,
        local_slot: slot.to_owned(),
        occurrence_at: ms(at),
        outcome: outcome.to_owned(),
        reason: if outcome == "created" {
            None
        } else {
            Some("paused".into())
        },
        child_broadcast_id: child,
        materialized_at: ms(at) + 5000,
        superseded_count: superseded,
    }
}

async fn insert_ledger(db: &Db, rows: Vec<LedgerRowInput>) {
    ok(repo::write(db, move |tx| rec::insert_ledger_rows(tx, &rows)).await);
}

/// Port of: packages/database/__tests__/broadcast-recurrence-repository.test.ts:58 "broadcastRecurrenceRepository > selectDbNow reads the database clock"
#[tokio::test]
async fn broadcast_recurrence_repository_select_db_now_reads_the_database_clock() {
    let db = fresh_db().await;
    let before = now_ms();
    let now = ok(repo::read(&db, rec::select_db_now).await);
    assert!(now >= before - 1000 && now <= now_ms() + 1000, "{now}");
}

/// Port of: packages/database/__tests__/broadcast-recurrence-repository.test.ts:67 "broadcastRecurrenceRepository > listDue returns due series oldest first and ignores a row without nextRunAt"
#[tokio::test]
async fn broadcast_recurrence_repository_list_due_returns_due_series_oldest_first_and_ignores_a_row_without_next_run_at()
 {
    let db = series().await;
    let now = ms("2026-10-13T03:00:00.000Z");
    let due = ok(repo::read(&db, move |c| rec::list_due(c, 5000, None, now)).await);
    assert_eq!(due.iter().map(|d| d.id).collect::<Vec<_>>(), vec![13, 11]);
    assert_eq!(due[1].next_run_at, ms("2026-10-13T02:00:00.000Z"));
    let limited = ok(repo::read(&db, move |c| rec::list_due(c, 1, None, now)).await);
    assert_eq!(limited.len(), 1);
}

/// Port of: packages/database/__tests__/broadcast-recurrence-repository.test.ts:80 "broadcastRecurrenceRepository > lockForWrite does nothing for an empty id list and locks FOR UPDATE otherwise"
#[tokio::test]
async fn broadcast_recurrence_repository_lock_for_write_does_nothing_for_an_empty_id_list_and_locks_for_update_otherwise()
 {
    let db = series().await;
    assert!(ok(repo::write(&db, |tx| rec::lock_for_write(tx, WS, &[])).await).is_empty());
    let rows = ok(repo::write(&db, |tx| rec::lock_for_write(tx, WS, &[2, 1])).await);
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![11, 12]);
}

/// Port of: packages/database/__tests__/broadcast-recurrence-repository.test.ts:90 "broadcastRecurrenceRepository > update reports whether the compare-and-set matched a row"
#[tokio::test]
async fn broadcast_recurrence_repository_update_reports_whether_the_compare_and_set_matched_a_row()
{
    let db = series().await;
    let expected = ms("2026-10-13T02:00:00.000Z");
    let patch = RecurrencePatch {
        next_run_at: Some(None),
        ..Default::default()
    };
    let p = patch.clone();
    assert!(ok(repo::write(&db, move |tx| rec::update(
        tx,
        11,
        &p,
        Some(expected)
    ))
    .await));
    assert!(!ok(repo::write(&db, move |tx| rec::update(
        tx,
        11,
        &patch,
        Some(expected)
    ))
    .await));
}

/// Port of: packages/database/__tests__/broadcast-recurrence-repository.test.ts:110 "broadcastRecurrenceRepository > findForMaterialize only finds a series of the workspace the job was enqueued for"
#[tokio::test]
async fn broadcast_recurrence_repository_find_for_materialize_only_finds_a_series_of_the_workspace_the_job_was_enqueued_for()
 {
    let db = series().await;
    assert!(ok(repo::write(&db, |tx| rec::find_for_materialize(tx, 11, 9999)).await).is_none());
    let (r, t) =
        ok(repo::write(&db, |tx| rec::find_for_materialize(tx, 11, WS)).await).expect("found");
    assert_eq!((r.id, t.id), (11, 1));
}

/// Port of: packages/database/__tests__/broadcast-recurrence-repository.test.ts:125 "broadcastRecurrenceRepository > insertLedgerRows is idempotent on (recurrenceId, localSlot, occurrenceAt) and skips an empty batch"
#[tokio::test]
async fn broadcast_recurrence_repository_insert_ledger_rows_is_idempotent_on_recurrence_id_local_slot_occurrence_at_and_skips_an_empty_batch()
 {
    let db = series().await;
    insert_ledger(&db, vec![]).await;
    let row = ledger(
        "2026-10-13 09:00",
        "2026-10-13T02:00:00.000Z",
        "created",
        Some(1),
        None,
    );
    insert_ledger(&db, vec![row.clone(), row.clone()]).await;
    // Same wall-clock text, another instant (after a time zone edit): a new row.
    let moved = ledger(
        "2026-10-13 09:00",
        "2026-10-13T03:00:00.000Z",
        "missed",
        None,
        None,
    );
    insert_ledger(&db, vec![moved]).await;
    let n = repo_common::scalar_i64(
        &db,
        r#"SELECT count(*) FROM "BroadcastRecurrenceOccurrence""#,
        vec![],
    )
    .await;
    assert_eq!(n, Some(2));
}

/// Port of: packages/database/__tests__/broadcast-recurrence-repository.test.ts:150 "broadcastRecurrenceRepository > grouped bucket queries group by ordinal, never by the parameterised bucket expression"
///
/// The SQLite form binds the zone in both the SELECT and the WHERE copies of the bucket
/// expression; the observable result: buckets grouped per day, newest first, filtered by
/// the cursor, and per-bucket cells.
#[tokio::test]
async fn broadcast_recurrence_repository_grouped_bucket_queries_group_by_ordinal_never_by_the_parameterised_bucket_expression()
 {
    let db = series().await;
    insert_ledger(
        &db,
        vec![
            ledger("a", "2026-10-08T10:00:00.000Z", "created", Some(1), None),
            ledger("b", "2026-10-09T10:00:00.000Z", "created", Some(1), None),
            ledger("c", "2026-10-09T11:00:00.000Z", "missed", None, Some(3)),
            ledger("d", "2026-10-11T10:00:00.000Z", "missed", None, None),
        ],
    )
    .await;
    let keys = ok(repo::read(&db, |c| {
        rec::list_ledger_bucket_keys(c, 11, Granularity::Day, "UTC", 31, Some("2026-10-10"))
    })
    .await);
    assert_eq!(keys, vec!["2026-10-09".to_owned(), "2026-10-08".to_owned()]);
    let cells = ok(repo::read(&db, |c| {
        rec::list_ledger_bucket_cells(c, 11, Granularity::Week, "UTC", &["2026-10-05".to_owned()])
    })
    .await);
    let mut slots: Vec<(String, i64)> = cells.into_iter().map(|c| (c.outcome, c.slots)).collect();
    slots.sort();
    assert_eq!(
        slots,
        vec![("created".to_owned(), 2), ("missed".to_owned(), 4)]
    );
}

/// Port of: packages/database/__tests__/broadcast-recurrence-repository.test.ts:184 "broadcastRecurrenceRepository > countLedgerSlots sums superseded slots so totals agree with the bucket counts"
#[tokio::test]
async fn broadcast_recurrence_repository_count_ledger_slots_sums_superseded_slots_so_totals_agree_with_the_bucket_counts()
 {
    let db = series().await;
    assert_eq!(
        ok(repo::read(&db, |c| rec::count_ledger_slots(c, 11)).await),
        0
    );
    insert_ledger(
        &db,
        vec![
            ledger("a", "2026-10-08T10:00:00.000Z", "created", Some(1), None),
            ledger("b", "2026-10-09T10:00:00.000Z", "missed", None, Some(602)),
        ],
    )
    .await;
    assert_eq!(
        ok(repo::read(&db, |c| rec::count_ledger_slots(c, 11)).await),
        603
    );
}

/// Port of: packages/database/__tests__/broadcast-recurrence-repository.test.ts:191 "broadcastRecurrenceRepository > bucket cells query nothing when no bucket is requested"
#[tokio::test]
async fn broadcast_recurrence_repository_bucket_cells_query_nothing_when_no_bucket_is_requested() {
    let db = series().await;
    insert_ledger(
        &db,
        vec![ledger(
            "a",
            "2026-10-08T10:00:00.000Z",
            "created",
            Some(1),
            None,
        )],
    )
    .await;
    let cells = ok(repo::read(&db, |c| {
        rec::list_ledger_bucket_cells(c, 11, Granularity::Day, "UTC", &[])
    })
    .await);
    assert!(cells.is_empty());
}

/// Port of: packages/database/__tests__/broadcast-recurrence-repository.test.ts:206 "broadcastRecurrenceRepository > ledger rows carry the run each created slot produced, and null for a skipped one"
#[tokio::test]
async fn broadcast_recurrence_repository_ledger_rows_carry_the_run_each_created_slot_produced_and_null_for_a_skipped_one()
 {
    let db = series().await;
    repo_common::handover::exec(
        &db,
        r#"UPDATE "Broadcast" SET "status" = 'sent' WHERE "id" = 2"#,
        vec![],
    )
    .await;
    insert_ledger(
        &db,
        vec![
            ledger("a", "2026-10-09T10:00:00.000Z", "created", Some(2), None),
            ledger("b", "2026-10-08T10:00:00.000Z", "missed", None, None),
        ],
    )
    .await;
    let rows = ok(repo::read(&db, |c| rec::list_ledger_rows(c, 11, None, None, None, 10)).await);
    assert_eq!(rows.len(), 2);
    let run = rows[0].run.as_ref().expect("run");
    assert_eq!((run.id, run.status.as_str()), (2, "sent"));
    assert!(rows[1].run.is_none());
}
