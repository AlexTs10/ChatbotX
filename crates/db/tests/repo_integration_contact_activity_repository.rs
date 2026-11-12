//! Port of `packages/database/__tests__/integration/contact-activity-repository.test.ts`
//! (P3.2). Runs against a migrated SQLite temp file: "own" = workspace 2001 (contacts
//! 10, 11), "other" = 2002 (contacts 20, 21).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::Db;
use chatbotx_db::repo::contact_activity::{
    self as ca, ContactActivityRecord, ContactKeywordMessageRecord,
};
use chatbotx_db::repo::{self};
use chatbotx_db::rusqlite::types::Value;
use repo_common::{int, ms, ok};

const OWN: i64 = 2001;
const OTHER: i64 = 2002;

fn early() -> i64 {
    ms("2026-01-01T00:00:00.000Z")
}
fn late() -> i64 {
    ms("2026-01-02T00:00:00.000Z")
}

async fn db() -> common::TestDb {
    let db = common::fresh_db().await;
    common::seed(&db).await;
    common::seed_workspace(&db, 1002, OTHER).await;
    repo_common::setup(&db, |tx| {
        for (id, ws) in [(10, OWN), (11, OWN), (20, OTHER), (21, OTHER)] {
            repo_common::insert_contact(tx, id, ws)?;
        }
        Ok(())
    })
    .await;
    db
}

fn flow(contact_id: i64, occurred_at: i64, reference_id: i64) -> ContactActivityRecord {
    ContactActivityRecord {
        activity_type: "flowExecuted".into(),
        contact_id,
        occurred_at,
        parent_reference_id: 0,
        reference_id,
        reference_type: "flow".into(),
    }
}

async fn upsert(db: &Db, ws: i64, records: Vec<ContactActivityRecord>) -> repo::Result<usize> {
    repo::write(db, move |tx| ca::upsert_activities(tx, ws, &records)).await
}

/// `(contactId, referenceId, firstOccurredAt)` of the workspace.
async fn activity_rows(db: &Db, ws: i64) -> Vec<(i64, i64, i64)> {
    repo_common::rows(db, r#"SELECT "contactId", "referenceId", "firstOccurredAt" FROM "ContactActivity" WHERE "workspaceId" = ?1 ORDER BY "contactId", "referenceId""#, vec![int(ws)])
        .await
        .into_iter()
        .map(|r| match (&r[0], &r[1], &r[2]) {
            (Value::Integer(a), Value::Integer(b), Value::Integer(c)) => (*a, *b, *c),
            other => panic!("{other:?}"),
        })
        .collect()
}

/// Port of: packages/database/__tests__/integration/contact-activity-repository.test.ts:151 "contact activity repository > upsertActivities is idempotent and only moves the time earlier"
#[tokio::test]
async fn contact_activity_repository_upsert_activities_is_idempotent_and_only_moves_the_time_earlier()
 {
    let db = db().await;
    ok(upsert(&db, OWN, vec![flow(10, late(), 5)]).await);
    ok(upsert(&db, OWN, vec![flow(10, late(), 5)]).await);
    assert_eq!(activity_rows(&db, OWN).await.len(), 1);
    ok(upsert(&db, OWN, vec![flow(10, early(), 5)]).await);
    assert_eq!(activity_rows(&db, OWN).await[0].2, early());
    ok(upsert(&db, OWN, vec![flow(10, late(), 5)]).await);
    assert_eq!(activity_rows(&db, OWN).await[0].2, early());
}

/// Port of: packages/database/__tests__/integration/contact-activity-repository.test.ts:177 "contact activity repository > upsertActivities keeps references and contacts apart"
#[tokio::test]
async fn contact_activity_repository_upsert_activities_keeps_references_and_contacts_apart() {
    let db = db().await;
    ok(upsert(
        &db,
        OWN,
        vec![
            flow(10, early(), 5),
            flow(10, early(), 6),
            flow(11, early(), 5),
        ],
    )
    .await);
    assert_eq!(activity_rows(&db, OWN).await.len(), 3);
}

/// Port of: packages/database/__tests__/integration/contact-activity-repository.test.ts:194 "contact activity repository > upsertActivities never attaches a foreign workspace's contact"
#[tokio::test]
async fn contact_activity_repository_upsert_activities_never_attaches_a_foreign_workspace_s_contact()
 {
    let db = db().await;
    ok(upsert(&db, OWN, vec![flow(20, early(), 5)]).await);
    assert!(activity_rows(&db, OWN).await.is_empty());
    assert!(activity_rows(&db, OTHER).await.is_empty());
}

/// Port of: packages/database/__tests__/integration/contact-activity-repository.test.ts:207 "contact activity repository > upsertActivities writes nothing for a deleted contact"
#[tokio::test]
async fn contact_activity_repository_upsert_activities_writes_nothing_for_a_deleted_contact() {
    let db = db().await;
    repo_common::handover::exec(&db, r#"DELETE FROM "Contact" WHERE "id" = 10"#, vec![]).await;
    ok(upsert(&db, OWN, vec![flow(10, early(), 5)]).await);
    assert!(activity_rows(&db, OWN).await.is_empty());
}

/// Port of: packages/database/__tests__/integration/contact-activity-repository.test.ts:224 "contact activity repository > accepts a flow node scoped to its parent flow"
#[tokio::test]
async fn contact_activity_repository_accepts_a_flow_node_scoped_to_its_parent_flow() {
    let db = db().await;
    let r = ContactActivityRecord {
        activity_type: "flowNodeExecuted".into(),
        contact_id: 10,
        occurred_at: early(),
        parent_reference_id: 77,
        reference_id: 1_778_663_027_294,
        reference_type: "flowNode".into(),
    };
    ok(upsert(&db, OWN, vec![r]).await);
    assert_eq!(activity_rows(&db, OWN).await.len(), 1);
}

/// Port of: packages/database/__tests__/integration/contact-activity-repository.test.ts:245 "contact activity repository > the database rejects an unregistered type/reference pair"
#[tokio::test]
async fn contact_activity_repository_the_database_rejects_an_unregistered_type_reference_pair() {
    let db = db().await;
    let mut r = flow(10, early(), 5);
    r.reference_type = "emailTopic".into();
    assert!(upsert(&db, OWN, vec![r]).await.is_err());
}

/// Port of: packages/database/__tests__/integration/contact-activity-repository.test.ts:266 "contact activity repository > insertKeywordMessage is idempotent by messageId and workspace scoped"
#[tokio::test]
async fn contact_activity_repository_insert_keyword_message_is_idempotent_by_message_id_and_workspace_scoped()
 {
    let db = db().await;
    let record = |contact_id| ContactKeywordMessageRecord {
        automated_response_id: 7,
        contact_id,
        message_id: 100,
        normalized_text: "xin chao".into(),
        occurred_at: early(),
        text: "Xin Chào".into(),
    };
    let (a, b, c) = (record(10), record(10), record(20));
    assert!(ok(repo::write(&db, move |tx| ca::insert_keyword_message(
        tx, OWN, &a
    ))
    .await));
    assert!(!ok(repo::write(&db, move |tx| ca::insert_keyword_message(
        tx, OWN, &b
    ))
    .await));
    assert!(!ok(repo::write(&db, move |tx| ca::insert_keyword_message(
        tx, OWN, &c
    ))
    .await));
    let n = repo_common::scalar_i64(
        &db,
        r#"SELECT count(*) FROM "ContactKeywordMessage" WHERE "workspaceId" = ?1"#,
        vec![int(OWN)],
    )
    .await;
    assert_eq!(n, Some(1));
}

/// Port of: packages/database/__tests__/integration/contact-activity-repository.test.ts:302 "contact activity repository > upsertLastDelivered only moves forward"
#[tokio::test]
async fn contact_activity_repository_upsert_last_delivered_only_moves_forward() {
    let db = db().await;
    let up = |at: i64| {
        repo::write(&db, move |tx| {
            ca::upsert_last_delivered(tx, OWN, &[(10, at)])
        })
    };
    let read = || {
        repo_common::scalar_i64(
            &db,
            r#"SELECT "lastDeliveredAt" FROM "ContactState" WHERE "workspaceId" = 2001 AND "contactId" = 10"#,
            vec![],
        )
    };
    ok(up(early()).await);
    assert_eq!(read().await, Some(early()));
    ok(up(late()).await);
    assert_eq!(read().await, Some(late()));
    assert_eq!(ok(up(early()).await), 0);
    assert_eq!(read().await, Some(late()));
}

/// Port of: packages/database/__tests__/integration/contact-activity-repository.test.ts:328 "contact activity repository > deleteByContactIds removes only the given contacts' rows"
#[tokio::test]
async fn contact_activity_repository_delete_by_contact_ids_removes_only_the_given_contacts_rows() {
    let db = db().await;
    ok(upsert(&db, OWN, vec![flow(10, early(), 5), flow(11, early(), 5)]).await);
    ok(repo::write(&db, |tx| {
        ca::upsert_last_delivered(tx, OWN, &[(10, early())])
    })
    .await);
    let deleted = ok(repo::write(&db, |tx| ca::delete_by_contact_ids(tx, OWN, &[10])).await);
    assert_eq!(deleted, 2);
    assert_eq!(
        activity_rows(&db, OWN)
            .await
            .into_iter()
            .map(|r| r.0)
            .collect::<Vec<_>>(),
        vec![11]
    );
}

/// Port of: packages/database/__tests__/integration/contact-activity-repository.test.ts:356 "contact activity repository > purges a workspace in bounded batches without touching another"
#[tokio::test]
async fn contact_activity_repository_purges_a_workspace_in_bounded_batches_without_touching_another()
 {
    let db = db().await;
    ok(upsert(
        &db,
        OWN,
        vec![
            flow(10, early(), 1),
            flow(10, early(), 2),
            flow(10, early(), 3),
        ],
    )
    .await);
    ok(upsert(&db, OTHER, vec![flow(20, early(), 5)]).await);
    assert_eq!(
        ok(ca::delete_workspace_batch(&db, "ContactActivity", OWN, 2).await),
        2
    );
    assert!(ok(repo::read(&db, |c| ca::has_workspace_rows(
        c,
        "ContactActivity",
        OWN
    ))
    .await));
    assert_eq!(
        ok(ca::delete_workspace_batch(&db, "ContactActivity", OWN, 2).await),
        1
    );
    assert!(!ok(repo::read(&db, |c| ca::has_workspace_rows(
        c,
        "ContactActivity",
        OWN
    ))
    .await));
    assert_eq!(activity_rows(&db, OTHER).await.len(), 1);
}

// not-applicable: packages/database/__tests__/integration/contact-activity-repository.test.ts:405 "contact activity repository > the workspace fence closes once the purge starts" — it tests `workspaceRepository.lockWorkspaceForChildWrite` (repositories/workspace, the second half of the repositories, P3.3 scope); ported there.
// not-applicable: packages/database/__tests__/integration/contact-activity-repository.test.ts:427 "contact activity repository > workspace-scoped reads prune to one partition for custom and generic plans" — Postgres partition pruning (EXPLAIN ANALYZE of ContactActivity_p* partitions, plan_cache_mode); the SQLite table is not partitioned (P2.5).
