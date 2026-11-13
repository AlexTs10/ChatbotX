//! Port of `packages/business/__tests__/tag-service-soft-delete.test.ts`.
//! Adapted: the `db.update` chunk spy becomes the stored `deletedAt` of every
//! id (chunks of 200 are internal); `enqueueDelete` (tag sync), the cache
//! invalidation and the template lock test are n-a.

mod common;

use chatbotx_business::tag as svc;
use common::{OTHER_WS, WS, scalar_i64, setup, tag, two_workspaces};

async fn tags(count: i64) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        for id in 1..=count {
            tag(tx, id, WS, &format!("t{id}"))?;
        }
        tag(tx, 9_000, OTHER_WS, "foreign")?;
        Ok(())
    })
    .await;
    db
}

async fn live(db: &chatbotx_db::Db) -> i64 {
    scalar_i64(
        db,
        r#"SELECT count(*) FROM "Tag" WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL"#,
        WS,
    )
    .await
    .unwrap_or_default()
}

#[tokio::test]
async fn chunks_ids_201_ids_triggers_exactly_2_db_update_calls() {
    let db = tags(201).await;
    let ids: Vec<i64> = (1..=201).collect();
    let deleted = svc::soft_delete(&db, WS, &ids).await.expect("delete");
    assert_eq!(deleted.len(), 201);
    assert_eq!(live(&db).await, 0);
}

#[tokio::test]
async fn enqueues_only_for_rows_actually_returned_by_db_update() {
    let db = tags(3).await;
    svc::soft_delete(&db, WS, &[1]).await.expect("first");
    let deleted = svc::soft_delete(&db, WS, &[1, 2, 9_000, 77])
        .await
        .expect("second");
    assert_eq!(deleted, vec![2]);
    assert_eq!(live(&db).await, 1);
}

#[tokio::test]
async fn empty_ids_zero_db_update_calls_and_zero_enqueue_calls() {
    let db = tags(2).await;
    let deleted = svc::soft_delete(&db, WS, &[]).await.expect("delete");
    assert!(deleted.is_empty());
    assert_eq!(live(&db).await, 2);
}

#[tokio::test]
async fn set_includes_deleted_at() {
    let db = tags(1).await;
    svc::soft_delete(&db, WS, &[1]).await.expect("delete");
    let at = scalar_i64(&db, r#"SELECT "deletedAt" FROM "Tag" WHERE "id" = ?1"#, 1).await;
    assert!(at.is_some_and(|ms| ms > 0));
}

#[tokio::test]
async fn soft_deletes_and_enqueues_when_tag_is_found_single_id() {
    let db = tags(1).await;
    assert_eq!(
        svc::soft_delete(&db, WS, &[1]).await.expect("delete"),
        vec![1]
    );
    assert!(
        svc::find_by_key(&db, WS, "1")
            .await
            .expect("find")
            .is_none()
    );
}

#[tokio::test]
async fn does_not_enqueue_when_row_not_found_or_already_soft_deleted() {
    let db = tags(1).await;
    svc::soft_delete(&db, WS, &[1]).await.expect("first");
    assert!(
        svc::soft_delete(&db, WS, &[1, 42])
            .await
            .expect("again")
            .is_empty()
    );
}
