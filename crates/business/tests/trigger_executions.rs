//! Port of `packages/business/__tests__/trigger-service-executions.test.ts`
//! (Q.D5). Adapted: the mocked drizzle chain becomes stored rows; the asserted
//! `where` / `values` objects are checked as the rows returned or written.
//! `listActiveWithConditions`, `recordContactHistory`, `incrementStats` and
//! `tryAcquireContactRun` are the trigger queue's (`chatbotx_worker::trigger`,
//! Q.W2: matching, history + stats, in-process run limit in `trigger_tests`).

mod common;

use chatbotx_business::trigger;
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, TestDb, WS, contact, scalar_i64, setup, two_workspaces};

/// Triggers 51 (active, `WS`, condition 61), 52 (inactive), 53 (active,
/// `OTHER_WS`, timezone Asia/Ho_Chi_Minh); contacts 81 (`WS`), 82 (`OTHER_WS`).
async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Workspace" SET "timezone" = 'Asia/Ho_Chi_Minh' WHERE "id" = ?1"#,
            [OTHER_WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Trigger" ("id", "name", "active", "actions", "workspaceId") VALUES
               (51, 'a', 1, '[]', ?1), (52, 'b', 0, '[]', ?1), (53, 'c', 1, '[]', ?2)"#,
            params![WS, OTHER_WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Condition" ("id", "triggerId", "type", "sourceId") VALUES
               (61, 51, 'dateTimeBasedTrigger', '7')"#,
            [],
        )?;
        contact(tx, 81, WS)?;
        contact(tx, 82, OTHER_WS)?;
        Ok(())
    })
    .await;
    db
}

fn ids(rows: &[trigger::ActiveTriggerPageRow]) -> Vec<i64> {
    rows.iter().map(|r| r.trigger.trigger.id).collect()
}

// describe("listActiveWithConditionsPage")

// Adapted: the mocked full page becomes two stored active triggers; the page
// also carries the conditions and the workspace timezone (`with: { workspace }`).
#[tokio::test]
async fn returns_next_cursor_only_when_the_page_is_full() {
    let db = db().await;
    let (rows, next) = trigger::list_active_with_conditions_page(&db, None, 2)
        .await
        .unwrap();
    assert_eq!(ids(&rows), vec![51, 53]);
    assert_eq!(next, Some(53));
    assert_eq!(rows[0].trigger.conditions.len(), 1);
    assert_eq!(rows[0].timezone, "UTC");
    assert_eq!(rows[1].timezone, "Asia/Ho_Chi_Minh");
}

#[tokio::test]
async fn omits_next_cursor_when_the_page_is_short_drained() {
    let db = db().await;
    let (rows, next) = trigger::list_active_with_conditions_page(&db, Some(51), 2)
        .await
        .unwrap();
    assert_eq!(ids(&rows), vec![53]);
    assert_eq!(next, None);
}

// describe("listExecutedPairs")

#[tokio::test]
async fn returns_empty_without_querying_when_either_id_list_is_empty() {
    let db = db().await;
    trigger::record_execution(&db, 51, 81, WS).await.unwrap();
    assert!(
        trigger::list_executed_pairs(&db, vec![], vec![81])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        trigger::list_executed_pairs(&db, vec![51, 53], vec![81, 82])
            .await
            .unwrap(),
        vec![(51, 81)]
    );
}

// describe("recordExecution")

#[tokio::test]
async fn on_conflict_do_nothing_inserts_the_execution_row_with_trigger_contact_and_workspace() {
    let db = db().await;
    trigger::record_execution(&db, 51, 81, WS).await.unwrap();
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "workspaceId" FROM "TriggerExecution" WHERE "triggerId" = 51 AND "contactId" = ?1"#,
            81
        )
        .await,
        Some(WS)
    );
}

// describe("purgeExecutionsOlderThan")

// Adapted: the mocked rowCount becomes stored rows on both sides of the cutoff.
#[tokio::test]
async fn returns_the_affected_row_count() {
    let db = db().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "TriggerExecution" ("id", "triggerId", "contactId", "workspaceId", "executedAt") VALUES
               (91, 51, 81, ?1, 1000), (92, 51, 81, ?1, 2000), (93, 53, 82, ?2, 9000)"#,
            params![WS, OTHER_WS],
        )?;
        Ok(())
    })
    .await;
    assert_eq!(
        trigger::purge_executions_older_than(&db, 5000)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "TriggerExecution" WHERE "id" >= ?1"#,
            0
        )
        .await,
        Some(1)
    );
}

#[tokio::test]
async fn returns_0_when_row_count_is_null() {
    let db = db().await;
    assert_eq!(
        trigger::purge_executions_older_than(&db, 5000)
            .await
            .unwrap(),
        0
    );
}
