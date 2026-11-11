//! Port of `packages/scheduler/__tests__/smoke.test.ts` (spec 5.2,
//! "smoke"): the SQLite `DispatchSchedule` (`SchedulerClient`).

#[path = "support/sequence.rs"]
mod sequence;
mod support;

use std::sync::Arc;

use chatbotx_jobs::sequence::repo::TxObserver;
use chatbotx_jobs::sequence::schedule::{
    DispatchSchedule, ScheduleEntry, ScheduleError, ScheduleKind,
};
use chatbotx_jobs::sequence::sqlite_schedule::SqliteDispatchSchedule;

use sequence::{DISPATCH_1, DISPATCH_2, id};

fn entries() -> Vec<ScheduleEntry> {
    vec![
        ScheduleEntry {
            bucket: 1,
            dispatch_id: id(DISPATCH_1),
            at_ms: 100,
        },
        ScheduleEntry {
            bucket: 2,
            dispatch_id: id(DISPATCH_2),
            at_ms: 200,
        },
    ]
}

/// Port of: packages/scheduler/__tests__/smoke.test.ts:10 "SchedulerClient.batchAddToSchedule > throws aggregate errors returned by individual pipeline commands"
///
/// Adapted (DECISIONS P2.6): the batch is one write transaction, so a failing
/// second entry fails the whole batch and **no** row is written (the Redis
/// pipeline applied command 1). The `zadd` key/score/member assertions become
/// the rows the same batch writes once it can succeed. Ids are integers
/// (`sequence_schedule.dispatch_id` = the `SequenceDispatch` id):
/// "dispatch-1" = 801, "dispatch-2" = 802.
#[tokio::test]
async fn throws_aggregate_errors_returned_by_individual_pipeline() {
    let db = support::migrated_db().await;
    let schedule = SqliteDispatchSchedule::open(Arc::clone(&db), None)
        .await
        .expect("open schedule");
    // Fault injection for entry 2 (the `[commandError, null]` pipeline result).
    db.write(|tx| {
        tx.execute_batch(
            "CREATE TRIGGER fail_dispatch_2 BEFORE INSERT ON sequence_schedule \
             WHEN NEW.dispatch_id = 802 \
             BEGIN SELECT RAISE(ABORT, 'zadd failed'); END;",
        )?;
        Ok(())
    })
    .await
    .expect("create trigger");

    let result = schedule.batch_add_to_schedule(entries()).await;

    match result {
        Err(ScheduleError::Aggregate(errors)) => assert!(!errors.is_empty()),
        other => panic!("expected ScheduleError::Aggregate, got {other:?}"),
    }
    // All-or-nothing: entry 1 was not written either.
    assert!(
        schedule
            .members(ScheduleKind::Schedule, 1)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        schedule
            .members(ScheduleKind::Schedule, 2)
            .await
            .unwrap()
            .is_empty()
    );

    // The same batch without the fault writes (bucket 1, 100, dispatch-1)
    // and (bucket 2, 200, dispatch-2).
    db.write(|tx| {
        tx.execute_batch("DROP TRIGGER fail_dispatch_2;")?;
        Ok(())
    })
    .await
    .expect("drop trigger");
    schedule
        .batch_add_to_schedule(entries())
        .await
        .expect("batch");
    assert_eq!(
        schedule
            .get_due(ScheduleKind::Schedule, 1, 100, 100)
            .await
            .unwrap(),
        vec![id(DISPATCH_1)]
    );
    assert!(
        schedule
            .get_due(ScheduleKind::Schedule, 1, 99, 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        schedule
            .get_due(ScheduleKind::Schedule, 2, 200, 100)
            .await
            .unwrap(),
        vec![id(DISPATCH_2)]
    );
    assert!(
        schedule
            .get_due(ScheduleKind::Schedule, 2, 199, 100)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Port of: packages/scheduler/__tests__/smoke.test.ts:43 "SchedulerClient.batchAddToSchedule > does not create a pipeline when no schedules are supplied"
///
/// Adapted: "no pipeline" = no write transaction (the `TxProbe` counts 0).
#[tokio::test]
async fn does_not_create_a_pipeline_when_no_schedules_are_supplied() {
    let db = support::migrated_db().await;
    let order = sequence::order_log();
    let probe = sequence::TxProbe::new(&order);
    let observer: Arc<dyn TxObserver> = probe.clone();
    let schedule = SqliteDispatchSchedule::open(Arc::clone(&db), Some(observer))
        .await
        .expect("open schedule");
    let begins_after_open = probe.begins();

    schedule
        .batch_add_to_schedule(Vec::new())
        .await
        .expect("empty batch");

    assert_eq!(probe.begins(), begins_after_open);
    assert_eq!(probe.commits(), begins_after_open);
}
