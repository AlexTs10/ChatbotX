//! Ports of `apps/worker/__tests__/capture-contact-profile-snapshot.test.ts`
//! and `apps/worker/__tests__/dispatch-profile-snapshots.test.ts` over a
//! migrated database, a scripted Instagram API and the durable kv.
//! ponytail: the mocked seams without a Rust counterpart are not ported:
//! a messenger claim (the claim SQL only takes snapshot channels), a failing
//! `complete` (one SQL write), and a blocked prefix larger than a run (the
//! list SQL already drops frozen workspaces).

mod channel_fake;
mod common;
mod x1_support;

use std::sync::Arc;

use channel_fake::{FakeHttp, channel_row, graph_error, ok};
use chatbotx_business::contact::profile_snapshot::{
    DUE_CURSOR_KEY, EXHAUSTED_CURSOR_KEY, SnapshotTarget, capture, dispatch,
};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::QueueName;
use chatbotx_kv::DurableKv;
use common::{OTHER_WS, WS, setup, two_workspaces};
use serde_json::{Value, json};
use x1_support::RecordingEnqueuer;

const INBOX: i64 = 51;
const CI: i64 = 61;

fn target(contact_inbox_id: i64) -> SnapshotTarget {
    SnapshotTarget {
        contact_inbox_id,
        inbox_id: INBOX,
        workspace_id: WS,
    }
}

/// A `pending`, due contact inbox `id` (`attempts` so far) of the inbox.
fn pending(
    tx: &chatbotx_db::rusqlite::Connection,
    id: i64,
    inbox: i64,
    ws: i64,
    attempts: Option<i64>,
    at: i64,
) -> Result<(), chatbotx_db::DbError> {
    common::contact(tx, id, ws)?;
    common::contact_inbox(tx, id, id, inbox, "instagram", &format!("igsid-{id}"))?;
    tx.execute(
        r#"UPDATE "ContactInbox" SET "profileSnapshotState" = 'pending', "profileSnapshotAttempts" = ?2,
               "profileSnapshotNextAttemptAt" = ?3 WHERE "id" = ?1"#,
        params![id, attempts, at],
    )?;
    Ok(())
}

async fn seed(attempts: Option<i64>, with_integration: bool) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        if with_integration {
            channel_row(
                tx,
                "IntegrationInstagram",
                INBOX,
                WS,
                "ig-page",
                &json!({ "tokens": { "accessToken": "ig-token" } }),
            )?;
        } else {
            common::inbox(tx, INBOX, WS, "instagram")?;
        }
        pending(tx, CI, INBOX, WS, attempts, 0)
    })
    .await;
    db
}

/// `(state, attempts, nextAttemptAt, followsBusiness, followerCount, accountVerified)`.
type Row = (
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<bool>,
    Option<i64>,
    Option<bool>,
);

async fn row(db: &Db, id: i64) -> Row {
    db.read(move |conn| {
        Ok(conn.query_row(
            r#"SELECT "profileSnapshotState", "profileSnapshotAttempts", "profileSnapshotNextAttemptAt",
                      "followsBusiness", "followerCount", "accountVerified"
               FROM "ContactInbox" WHERE "id" = ?1"#,
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )?)
    })
    .await
    .expect("read contact inbox")
}

fn null_snapshot() -> Value {
    json!({ "id": "igsid-61" })
}

// ---- captureContactProfileSnapshot ---------------------------------------------

#[tokio::test]
async fn does_nothing_when_the_row_cannot_be_claimed() {
    let db = seed(None, true).await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "ContactInbox" SET "profileSnapshotState" = 'captured' WHERE "id" = ?1"#,
            [CI],
        )?;
        Ok(())
    })
    .await;
    let http = FakeHttp::new(|_| ok(null_snapshot()));
    capture(&db, &http, target(CI)).await.unwrap();
    assert!(http.seen().is_empty());
    assert_eq!(row(&db, CI).await.0.as_deref(), Some("captured"));
}

#[tokio::test]
async fn captures_an_all_null_response_as_a_terminal_successful_snapshot() {
    let db = seed(None, true).await;
    let http = FakeHttp::new(|_| ok(null_snapshot()));
    capture(&db, &http, target(CI)).await.unwrap();
    assert_eq!(
        row(&db, CI).await,
        (Some("captured".into()), Some(1), None, None, None, None)
    );
}

#[tokio::test]
async fn resolves_the_integration_from_the_claimed_channel_and_calls_the_snapshot_handler() {
    let db = seed(None, true).await;
    let http = FakeHttp::new(|_| {
        ok(
            json!({ "is_user_follow_business": true, "follower_count": 7, "is_verified_user": true }),
        )
    });
    capture(&db, &http, target(CI)).await.unwrap();
    let seen = http.seen();
    assert_eq!(seen.len(), 1);
    assert!(
        seen[0]
            .url
            .starts_with("https://graph.instagram.com/v22.0/igsid-61?fields="),
        "{}",
        seen[0].url
    );
    assert_eq!(
        seen[0].headers,
        vec![("Authorization".to_owned(), "Bearer ig-token".to_owned())]
    );
    assert_eq!(
        row(&db, CI).await,
        (
            Some("captured".into()),
            Some(1),
            None,
            Some(true),
            Some(7),
            Some(true)
        )
    );
}

#[tokio::test]
async fn ends_a_missing_integration_as_unavailable_instead_of_burning_the_retry_budget() {
    let db = seed(None, false).await;
    let http = FakeHttp::new(|_| ok(null_snapshot()));
    capture(&db, &http, target(CI)).await.unwrap();
    assert!(http.seen().is_empty());
    assert_eq!(row(&db, CI).await.0.as_deref(), Some("unavailable"));
}

#[tokio::test]
async fn reschedules_a_transport_failure_and_an_unknown_failure() {
    let db = seed(None, true).await;
    let before = chatbotx_db::repo::now_ms();
    capture(&db, &FakeHttp::offline(), target(CI))
        .await
        .unwrap();
    let (state, attempts, next, ..) = row(&db, CI).await;
    assert_eq!((state.as_deref(), attempts), (Some("pending"), Some(1)));
    let next = next.expect("next attempt");
    assert!(next >= before + 30_000 && next < before + 60_000, "{next}");
}

#[tokio::test]
async fn fails_the_row_when_the_retry_budget_is_exhausted() {
    let db = seed(Some(4), true).await;
    let http = FakeHttp::new(|_| graph_error(400, "rate limited", 4, None));
    capture(&db, &http, target(CI)).await.unwrap();
    let (state, attempts, next, ..) = row(&db, CI).await;
    assert_eq!(
        (state.as_deref(), attempts, next),
        (Some("failed"), Some(5), None)
    );
}

#[tokio::test]
async fn completes_a_provider_policy_error_as_terminal_failed() {
    let db = seed(None, true).await;
    let http = FakeHttp::new(|_| graph_error(400, "invalid field", 100, None));
    capture(&db, &http, target(CI)).await.unwrap();
    assert_eq!(
        row(&db, CI).await,
        (Some("failed".into()), Some(1), None, None, None, None)
    );
}

#[tokio::test]
async fn completes_230_and_100_33_as_unavailable_without_spending_another_retry() {
    for (code, sub) in [(230, None), (100, Some(33))] {
        let db = seed(None, true).await;
        let http = FakeHttp::new(move |_| graph_error(400, "unavailable", code, sub));
        capture(&db, &http, target(CI)).await.unwrap();
        assert_eq!(
            row(&db, CI).await.0.as_deref(),
            Some("unavailable"),
            "#{code}"
        );
    }
}

#[tokio::test]
async fn reschedules_a_graph_rate_limit_and_a_provider_5xx() {
    for (status, code) in [(400, 4), (503, 2)] {
        let db = seed(None, true).await;
        let http = FakeHttp::new(move |_| graph_error(status, "busy", code, None));
        capture(&db, &http, target(CI)).await.unwrap();
        let (state, attempts, next, ..) = row(&db, CI).await;
        assert_eq!(
            (state.as_deref(), attempts),
            (Some("pending"), Some(1)),
            "{status}"
        );
        assert!(next.is_some());
    }
}

// ---- dispatchProfileSnapshots ----------------------------------------------------

/// A migrated database behind an `Arc` (the durable kv shares it).
async fn shared_db() -> (Arc<Db>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-snapshot-dispatch-{}-{}",
        std::process::id(),
        chatbotx_core::utils::id::create_id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let db = Db::open(dir.join("test.db")).expect("open");
    db.migrate().await.expect("migrate");
    setup(&db, |tx| {
        common::workspace(tx, 1001, WS)?;
        common::workspace(tx, 1002, OTHER_WS)?;
        Ok(())
    })
    .await;
    (Arc::new(db), dir)
}

fn kv(db: &Arc<Db>) -> DurableKv {
    DurableKv::new(Arc::clone(db), Arc::new(chatbotx_rt::SystemClock))
}

fn captures(jobs: &RecordingEnqueuer) -> Vec<String> {
    jobs.calls()
        .into_iter()
        .filter(|c| c.queue == QueueName::ProfileSnapshot)
        .map(|c| {
            c.data["data"]["contactInboxId"]
                .as_str()
                .unwrap_or("")
                .to_owned()
        })
        .collect()
}

#[tokio::test]
async fn terminalizes_exhausted_work_only_after_its_lease_expired() {
    let (db, dir) = shared_db().await;
    let future = chatbotx_db::repo::now_ms() + 60_000;
    setup(&db, move |tx| {
        common::inbox(tx, INBOX, WS, "instagram")?;
        pending(tx, 71, INBOX, WS, Some(5), 0)?;
        // Still leased: left alone.
        pending(tx, 72, INBOX, WS, Some(5), future)?;
        Ok(())
    })
    .await;
    let jobs = RecordingEnqueuer::new();
    dispatch(&db, &*jobs, &kv(&db)).await.unwrap();
    assert_eq!(row(&db, 71).await.0.as_deref(), Some("failed"));
    assert_eq!(row(&db, 72).await.0.as_deref(), Some("pending"));
    assert!(captures(&jobs).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn continues_past_a_blocked_workspace_and_dispatches_later_eligible_rows() {
    let (db, dir) = shared_db().await;
    setup(&db, |tx| {
        common::inbox(tx, INBOX, WS, "instagram")?;
        common::inbox(tx, INBOX + 1, OTHER_WS, "instagram")?;
        pending(tx, 81, INBOX + 1, OTHER_WS, None, 0)?;
        pending(tx, 82, INBOX, WS, None, 1)?;
        tx.execute(
            r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = ?1"#,
            [OTHER_WS],
        )?;
        Ok(())
    })
    .await;
    let jobs = RecordingEnqueuer::new();
    dispatch(&db, &*jobs, &kv(&db)).await.unwrap();
    assert_eq!(captures(&jobs), vec!["82".to_owned()]);
    let call = &jobs.calls()[0];
    assert_eq!(
        call.data["data"],
        json!({ "contactInboxId": "82", "inboxId": INBOX.to_string(), "workspaceId": WS.to_string() })
    );
    assert_eq!(call.opts.job_id.as_deref(), Some("profile-snapshot-82"));
    let _ = std::fs::remove_dir_all(dir);
}

#[tokio::test]
async fn paginates_exhausted_and_due_rows_independently() {
    let (db, dir) = shared_db().await;
    setup(&db, |tx| {
        common::inbox(tx, INBOX, WS, "instagram")?;
        for i in 0..100 {
            pending(tx, 1_000 + i, INBOX, WS, Some(5), i)?;
            pending(tx, 2_000 + i, INBOX, WS, None, i)?;
        }
        Ok(())
    })
    .await;
    let jobs = RecordingEnqueuer::new();
    let store = kv(&db);
    dispatch(&db, &*jobs, &store).await.unwrap();
    assert_eq!(row(&db, 1_099).await.0.as_deref(), Some("failed"));
    assert_eq!(captures(&jobs).len(), 100);
    // Both walks reached their end: the cursors are cleared.
    assert_eq!(store.get::<String>(DUE_CURSOR_KEY).await.unwrap(), None);
    assert_eq!(
        store.get::<String>(EXHAUSTED_CURSOR_KEY).await.unwrap(),
        None
    );
    let _ = std::fs::remove_dir_all(dir);
}
