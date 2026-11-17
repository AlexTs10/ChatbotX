//! `WorkspaceService.purgeDueScheduled` (`workspace/__tests__/service.test.ts`,
//! Q.D11 slice 9) on a real database: SQLite triggers stand in for the
//! mocked failures, the returned count and the stored rows for the mock
//! call checks.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use chatbotx_business::workspace::purge::{
    PURGE_WORKSPACE_MAX_DURATION_MS, PaymentTeardown, PurgeOptions, acquire_purge_fence,
    purge_due_scheduled,
};
use chatbotx_db::DbError;
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::{Connection, params};
use common::{fresh_db, scalar_i64, setup};
use futures::future::BoxFuture;

/// A due workspace `id` owned by user `id + 100`.
fn due(tx: &Connection, id: i64) -> Result<(), DbError> {
    common::workspace(tx, id + 100, id)?;
    tx.execute(
        r#"UPDATE "Workspace" SET "scheduledDeletionAt" = ?2, "deletionRequestedById" = ?3,
           "deletionRequestedAt" = ?4, "name" = 'Acme' WHERE "id" = ?1"#,
        params![id, now_ms() - 60_000, id + 100, 1_793_491_200_000_i64],
    )?;
    Ok(())
}

fn exec(tx: &Connection, sql: &str) -> Result<(), DbError> {
    tx.execute_batch(sql)?;
    Ok(())
}

const EXISTS: &str = r#"SELECT COUNT(*) FROM "Workspace" WHERE "id" = ?1"#;
const LOGS: &str =
    r#"SELECT COUNT(*) FROM "ResourceDeletionLog" WHERE "resourceId" = CAST(?1 AS TEXT)"#;
const COMPLETED_LOGS: &str = r#"SELECT COUNT(*) FROM "ResourceDeletionLog" WHERE "resourceId" = CAST(?1 AS TEXT) AND "status" = 'completed'"#;

/// A `teardownPaymentProviders` hook.
fn hook(
    f: impl Fn(i64) -> BoxFuture<'static, chatbotx_business::Result<()>> + Send + Sync + 'static,
) -> Option<PaymentTeardown> {
    Some(Arc::new(f))
}

fn one_chunk() -> PurgeOptions {
    PurgeOptions {
        chunk_size: 1,
        max_chunks: 1,
        ..PurgeOptions::default()
    }
}

/// "deletes only workspaces that tear down cleanly and never aborts the run
/// on a single failure" / "logs the underlying postgres cause when workspace
/// row delete fails" (the cause is in the log line; the result is checked).
#[tokio::test]
async fn a_failing_workspace_is_kept_and_the_run_continues() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        due(tx, 1)?;
        due(tx, 2)?;
        exec(
            tx,
            r#"CREATE TRIGGER "boom" BEFORE DELETE ON "Workspace" WHEN OLD."id" = 1
               BEGIN SELECT RAISE(ABORT, 'teardown boom'); END;"#,
        )
    })
    .await;
    assert_eq!(
        purge_due_scheduled(&db, PurgeOptions::default())
            .await
            .unwrap(),
        1
    );
    assert_eq!(scalar_i64(&db, EXISTS, 1).await, Some(1));
    assert_eq!(scalar_i64(&db, EXISTS, 2).await, Some(0));
}

/// "defers a workspace while a template uninstall holds a live lease on it".
#[tokio::test]
async fn a_live_template_uninstall_defers_the_workspace() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        due(tx, 1)?;
        tx.execute(
            r#"INSERT INTO "TemplateInstallation" ("id", "workspaceId", "templateName", "sourceWorkspaceId",
               "formatVersion", "permissions", "uninstallState", "uninstallClaimedAt")
               VALUES (1, 1, 't', 1, 1, '{}', 'running', ?1)"#,
            [now_ms()],
        )?;
        Ok(())
    })
    .await;
    assert_eq!(
        purge_due_scheduled(&db, PurgeOptions::default())
            .await
            .unwrap(),
        0
    );
    assert_eq!(scalar_i64(&db, EXISTS, 1).await, Some(1));
}

/// "skips a workspace that was rescheduled before its fence is acquired" /
/// "resumes a workspace with an existing purge fence" (the fence is checked
/// directly: a single-process run cannot reschedule between claim and fence).
#[tokio::test]
async fn the_fence_skips_a_rescheduled_workspace_and_keeps_a_started_one() {
    let db = fresh_db().await;
    let started = 1_790_000_000_000_i64;
    setup(&db, move |tx| {
        due(tx, 1)?;
        due(tx, 2)?;
        tx.execute(
            r#"UPDATE "Workspace" SET "scheduledDeletionAt" = NULL WHERE "id" = 1"#,
            [],
        )?;
        tx.execute(
            r#"UPDATE "Workspace" SET "scheduledDeletionAt" = NULL, "purgeStartedAt" = ?1 WHERE "id" = 2"#,
            [started],
        )?;
        Ok(())
    })
    .await;
    assert_eq!(acquire_purge_fence(&db, 1).await.unwrap(), None);
    let fenced = acquire_purge_fence(&db, 2).await.unwrap().unwrap();
    assert_eq!((fenced.id, fenced.owner_id), (2, 102));
    let fence = r#"SELECT "purgeStartedAt" FROM "Workspace" WHERE "id" = ?1"#;
    assert_eq!(scalar_i64(&db, fence, 2).await, Some(started));

    let torn_down = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = Arc::clone(&torn_down);
    let options = PurgeOptions {
        teardown_payment_providers: hook(move |id| {
            seen.lock().unwrap().push(id);
            Box::pin(async { Ok(()) })
        }),
        ..PurgeOptions::default()
    };
    assert_eq!(purge_due_scheduled(&db, options).await.unwrap(), 1);
    assert_eq!(*torn_down.lock().unwrap(), vec![2]);
    assert_eq!(scalar_i64(&db, EXISTS, 1).await, Some(1));
}

/// "tears down up to five claimed workspaces concurrently".
#[tokio::test]
async fn up_to_five_workspaces_tear_down_concurrently() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        for id in 1..=6 {
            due(tx, id)?;
        }
        Ok(())
    })
    .await;
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let (a, m) = (Arc::clone(&active), Arc::clone(&max_active));
    let options = PurgeOptions {
        chunk_size: 6,
        max_chunks: 1,
        teardown_payment_providers: hook(move |_| {
            let (a, m) = (Arc::clone(&a), Arc::clone(&m));
            Box::pin(async move {
                let now = a.fetch_add(1, Ordering::SeqCst) + 1;
                m.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                a.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            })
        }),
        ..PurgeOptions::default()
    };
    assert_eq!(purge_due_scheduled(&db, options).await.unwrap(), 6);
    assert_eq!(max_active.load(Ordering::SeqCst), 5);
    let left = r#"SELECT COUNT(*) FROM "Workspace" WHERE "id" <= ?1"#;
    assert_eq!(scalar_i64(&db, left, 6).await, Some(0));
}

/// "defers a fenced workspace when its post drain is incomplete" / "… when
/// its contact activity drain is incomplete" (one-row batches leave rows).
#[tokio::test]
async fn an_incomplete_post_or_activity_drain_defers_the_workspace() {
    for table in ["ContactInboxPost", "ContactState"] {
        let db = fresh_db().await;
        setup(&db, move |tx| {
            due(tx, 1)?;
            tx.execute(
                r#"INSERT INTO "UserQuota" ("id", "userId", "workspacesUsed") VALUES (1, 101, 1)"#,
                [],
            )?;
            for n in 1..=3 {
                if table == "ContactInboxPost" {
                    tx.execute(
                        r#"INSERT INTO "ContactInboxPost" ("workspaceId", "contactInboxId", "postId", "commentedAt") VALUES (1, ?1, 1, 0)"#,
                        [n],
                    )?;
                } else {
                    tx.execute(
                        r#"INSERT INTO "ContactState" ("workspaceId", "contactId") VALUES (1, ?1)"#,
                        [n],
                    )?;
                }
            }
            Ok(())
        })
        .await;
        let options = PurgeOptions {
            post_batches: (1, 1),
            activity_batches: (1, 1),
            ..one_chunk()
        };
        assert_eq!(
            purge_due_scheduled(&db, options).await.unwrap(),
            0,
            "{table}"
        );
        assert_eq!(scalar_i64(&db, EXISTS, 1).await, Some(1), "{table}");
        // The workspaces seat is not released.
        let used = r#"SELECT "workspacesUsed" FROM "UserQuota" WHERE "userId" = ?1"#;
        assert_eq!(scalar_i64(&db, used, 101).await, Some(1), "{table}");
    }
}

fn pending_log(tx: &Connection, id: i64) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "ResourceDeletionLog" ("id", "workspaceId", "resourceType", "resourceId", "status", "requestedAt")
           VALUES (50, ?1, 'workspace', CAST(?1 AS TEXT), 'pending', 1)"#,
        [id],
    )?;
    Ok(())
}

/// "deletion log > resolves the pending row before the final transaction and
/// completes it by id inside it".
#[tokio::test]
async fn the_pending_log_row_is_completed() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        due(tx, 1)?;
        pending_log(tx, 1)
    })
    .await;
    assert_eq!(purge_due_scheduled(&db, one_chunk()).await.unwrap(), 1);
    assert_eq!(scalar_i64(&db, LOGS, 1).await, Some(1));
    assert_eq!(scalar_i64(&db, COMPLETED_LOGS, 1).await, Some(1));
    let record = r#"SELECT COUNT(*) FROM "WorkspaceDeletionRecord" WHERE "workspaceId" = ?1 AND "purgedAt" IS NOT NULL"#;
    assert_eq!(scalar_i64(&db, record, 1).await, Some(1));
}

/// "records a workspace scheduled before the log existed as completed, from
/// what it stored".
#[tokio::test]
async fn a_workspace_without_a_log_row_is_recorded_as_completed() {
    let db = fresh_db().await;
    setup(&db, |tx| due(tx, 1)).await;
    assert_eq!(purge_due_scheduled(&db, one_chunk()).await.unwrap(), 1);
    assert_eq!(scalar_i64(&db, COMPLETED_LOGS, 1).await, Some(1));
    let row = r#"SELECT COUNT(*) FROM "ResourceDeletionLog" WHERE "resourceId" = CAST(?1 AS TEXT)
                 AND "resourceName" = 'Acme' AND "requestedByUserId" = 101 AND "requestedAt" = 1793491200000"#;
    assert_eq!(scalar_i64(&db, row, 1).await, Some(1));
}

/// "does not duplicate a row whose pending request is no longer pending"
/// (a trigger ignores the completing update).
#[tokio::test]
async fn a_row_that_is_no_longer_pending_is_not_duplicated() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        due(tx, 1)?;
        pending_log(tx, 1)?;
        exec(
            tx,
            r#"CREATE TRIGGER "keep" BEFORE UPDATE ON "ResourceDeletionLog"
               BEGIN SELECT RAISE(IGNORE); END;"#,
        )
    })
    .await;
    assert_eq!(purge_due_scheduled(&db, one_chunk()).await.unwrap(), 1);
    assert_eq!(scalar_i64(&db, LOGS, 1).await, Some(1));
}

/// "leaves the workspace for the next run when completing the log fails".
#[tokio::test]
async fn a_failing_log_completion_keeps_the_workspace() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        due(tx, 1)?;
        pending_log(tx, 1)?;
        exec(
            tx,
            r#"CREATE TRIGGER "fail" BEFORE UPDATE ON "ResourceDeletionLog"
               BEGIN SELECT RAISE(ABORT, 'log update failed'); END;"#,
        )
    })
    .await;
    assert_eq!(purge_due_scheduled(&db, one_chunk()).await.unwrap(), 0);
    assert_eq!(scalar_i64(&db, EXISTS, 1).await, Some(1));
}

/// "does not start the final delete when the pending lookup finishes after
/// the deadline": the clock passes the deadline at its fifth read (start,
/// chunk, teardown start and after the heavy drain read the real time; the
/// check after the lookup is the fifth), so the final delete never starts.
#[tokio::test]
async fn the_deadline_after_the_lookup_stops_the_delete() {
    let db = fresh_db().await;
    setup(&db, |tx| due(tx, 1)).await;
    let reads = Arc::new(AtomicI64::new(0));
    let start = now_ms();
    let options = PurgeOptions {
        clock: Arc::new(move || {
            let n = reads.fetch_add(1, Ordering::SeqCst);
            if n >= 4 {
                start + PURGE_WORKSPACE_MAX_DURATION_MS + 60_000
            } else {
                start
            }
        }),
        ..one_chunk()
    };
    assert_eq!(purge_due_scheduled(&db, options).await.unwrap(), 0);
    assert_eq!(scalar_i64(&db, EXISTS, 1).await, Some(1));
}

/// "writes nothing to the log when the workspace row was not deleted".
#[tokio::test]
async fn nothing_is_logged_when_the_row_is_not_deleted() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        due(tx, 1)?;
        pending_log(tx, 1)?;
        exec(
            tx,
            r#"CREATE TRIGGER "stay" BEFORE DELETE ON "Workspace"
               BEGIN SELECT RAISE(IGNORE); END;"#,
        )
    })
    .await;
    assert_eq!(purge_due_scheduled(&db, one_chunk()).await.unwrap(), 0);
    assert_eq!(scalar_i64(&db, COMPLETED_LOGS, 1).await, Some(0));
}
