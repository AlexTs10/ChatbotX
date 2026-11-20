//! Ports of `packages/business/__tests__/email-tracking.service.test.ts`,
//! `email-topic-click-url.test.ts` and
//! `packages/analytics/__tests__/email-topic-stats.repository.test.ts`
//! (Q.P6-U5). Adapted: the mocked repository / transaction becomes a
//! migrated database; "the counter is the last statement" and "runs on the
//! caller's transaction" are one SQLite transaction (checked as: all rows or
//! none); a failing write is a `RAISE(ABORT)` trigger.

mod common;
mod x1_support;

use chatbotx_business::email_tracking::{
    NewRecipient, Phase, create_recipient, mark_failed, record_transition,
    record_transition_or_enqueue_repair, sign_click_url, transition_job_id, verify_click_token,
};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::QueueName;
use common::{TestDb, WS, contact, setup, two_workspaces};
use serde_json::json;
use x1_support::RecordingEnqueuer;

const TOPIC: i64 = 71;
const CONTACT: i64 = 81;

async fn seeded() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        contact(tx, CONTACT, WS)?;
        tx.execute(
            r#"INSERT INTO "EmailTopic" ("id", "name", "workspaceId") VALUES (?1, 'News', ?2)"#,
            params![TOPIC, WS],
        )?;
        Ok(())
    })
    .await;
    db
}

fn recipient(topic: Option<i64>, contact: Option<i64>) -> NewRecipient {
    NewRecipient {
        topic_id: topic,
        workspace_id: WS,
        email: "ada@example.com".into(),
        contact_id: contact,
        ..NewRecipient::default()
    }
}

/// `(sends, delivereds, seens, clicks)` of the topic.
async fn counters(db: &Db) -> (i64, i64, i64, i64) {
    db.read(|conn| {
        Ok(conn.query_row(
            r#"SELECT "sendsTotal", "deliveredsTotal", "seensTotal", "clicksTotal" FROM "EmailTopic" WHERE "id" = ?1"#,
            [TOPIC],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?)
    })
    .await
    .unwrap()
}

/// The contact's `(activityType, referenceId)` rows.
async fn activities(db: &Db) -> Vec<(String, i64)> {
    db.read(|conn| {
        let mut stmt = conn.prepare(
            r#"SELECT "activityType", "referenceId" FROM "ContactActivity" WHERE "contactId" = ?1 AND "referenceType" = 'emailTopic' ORDER BY "activityType""#,
        )?;
        let rows = stmt
            .query_map([CONTACT], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
    .unwrap()
}

/// `(seenCount, clickCount, firstSeenAt set, firstClickedAt set, deliveredAt set, failedAt set)`.
async fn recipient_row(db: &Db, token: &str) -> (i64, i64, bool, bool, bool, bool) {
    let token = token.to_owned();
    db.read(move |conn| {
        Ok(conn.query_row(
            r#"SELECT "seenCount", "clickCount", "firstSeenAt" IS NOT NULL, "firstClickedAt" IS NOT NULL, "deliveredAt" IS NOT NULL, "failedAt" IS NOT NULL
               FROM "AnalyticsEmailTopic" WHERE "token" = ?1"#,
            [token],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )?)
    })
    .await
    .unwrap()
}

async fn break_updates(db: &Db) {
    setup(db, |tx| {
        tx.execute_batch(
            r#"CREATE TRIGGER "test_down" BEFORE UPDATE ON "AnalyticsEmailTopic" BEGIN SELECT RAISE(ABORT, 'database down'); END;"#,
        )?;
        Ok(())
    })
    .await;
}

// ---- createRecipient ----------------------------------------------------------

#[tokio::test]
async fn creates_the_recipient_and_records_sent_in_one_transaction() {
    let db = seeded().await;
    let token = create_recipient(&db, recipient(Some(TOPIC), Some(CONTACT)))
        .await
        .unwrap();
    // `crypto.randomUUID()`.
    assert_eq!(token.len(), 36);
    assert_eq!(token.matches('-').count(), 4);
    assert_eq!(activities(&db).await, [("messageSent".to_string(), TOPIC)]);
    assert_eq!(counters(&db).await, (1, 0, 0, 0));
    let snapshot: (Option<i64>, Option<i64>, String) = db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "topicId", "topicReferenceId", "email" FROM "AnalyticsEmailTopic" WHERE "token" = ?1"#,
                [token],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(
        snapshot,
        (Some(TOPIC), Some(TOPIC), "ada@example.com".into())
    );
}

#[tokio::test]
async fn records_sent_under_no_reference_and_moves_no_counter_without_a_topic() {
    let db = seeded().await;
    create_recipient(&db, recipient(None, Some(CONTACT)))
        .await
        .unwrap();
    assert_eq!(activities(&db).await, [("messageSent".to_string(), -1)]);
    assert_eq!(counters(&db).await, (0, 0, 0, 0));
}

#[tokio::test]
async fn writes_no_activity_for_a_recipient_without_a_contact() {
    let db = seeded().await;
    create_recipient(&db, recipient(Some(TOPIC), None))
        .await
        .unwrap();
    assert!(activities(&db).await.is_empty());
    // The counter still moves.
    assert_eq!(counters(&db).await, (1, 0, 0, 0));
}

#[tokio::test]
async fn fails_rolling_the_recipient_back_when_recording_fails() {
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute_batch(
            r#"CREATE TRIGGER "test_down" BEFORE INSERT ON "ContactActivity" BEGIN SELECT RAISE(ABORT, 'activity down'); END;"#,
        )?;
        Ok(())
    })
    .await;
    assert!(
        create_recipient(&db, recipient(Some(TOPIC), Some(CONTACT)))
            .await
            .is_err()
    );
    let rows: i64 = db
        .read(|conn| {
            Ok(
                conn.query_row(r#"SELECT count(*) FROM "AnalyticsEmailTopic""#, [], |r| {
                    r.get(0)
                })?,
            )
        })
        .await
        .unwrap();
    assert_eq!(rows, 0);
    // No counter without the activity write.
    assert_eq!(counters(&db).await, (0, 0, 0, 0));
}

// ---- recordTransition / the repository transitions ------------------------------

#[tokio::test]
async fn first_open_and_click_bump_the_topic_once_and_repeats_only_the_raw_counters() {
    let db = seeded().await;
    let token = create_recipient(&db, recipient(Some(TOPIC), Some(CONTACT)))
        .await
        .unwrap();
    for _ in 0..2 {
        record_transition(&db, Phase::Opened, &token).await.unwrap();
        record_transition(&db, Phase::Clicked, &token)
            .await
            .unwrap();
        record_transition(&db, Phase::Delivered, &token)
            .await
            .unwrap();
    }
    assert_eq!(counters(&db).await, (1, 1, 1, 1));
    assert_eq!(
        recipient_row(&db, &token).await,
        (2, 2, true, true, true, false)
    );
    assert_eq!(
        activities(&db).await,
        [
            ("messageClicked".to_string(), TOPIC),
            ("messageDelivered".to_string(), TOPIC),
            ("messageOpened".to_string(), TOPIC),
            ("messageSent".to_string(), TOPIC),
        ]
    );
}

#[tokio::test]
async fn keeps_the_original_topic_after_the_topic_was_deleted_and_owes_no_counter() {
    let db = seeded().await;
    let token = create_recipient(&db, recipient(Some(TOPIC), Some(CONTACT)))
        .await
        .unwrap();
    setup(&db, |tx| {
        tx.execute(r#"DELETE FROM "EmailTopic" WHERE "id" = ?1"#, [TOPIC])?;
        Ok(())
    })
    .await;
    record_transition(&db, Phase::Opened, &token).await.unwrap();
    assert!(
        activities(&db)
            .await
            .contains(&("messageOpened".to_string(), TOPIC))
    );
}

#[tokio::test]
async fn falls_back_to_the_live_topic_for_a_row_created_before_the_snapshot_column() {
    let db = seeded().await;
    let token = create_recipient(&db, recipient(Some(TOPIC), Some(CONTACT)))
        .await
        .unwrap();
    let t = token.clone();
    setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "AnalyticsEmailTopic" SET "topicReferenceId" = NULL WHERE "token" = ?1"#,
            [t],
        )?;
        Ok(())
    })
    .await;
    record_transition(&db, Phase::Clicked, &token)
        .await
        .unwrap();
    assert!(
        activities(&db)
            .await
            .contains(&("messageClicked".to_string(), TOPIC))
    );
}

#[tokio::test]
async fn a_deleted_contact_only_moves_the_tracking_counters() {
    let db = seeded().await;
    let token = create_recipient(&db, recipient(Some(TOPIC), None))
        .await
        .unwrap();
    record_transition(&db, Phase::Opened, &token).await.unwrap();
    assert!(activities(&db).await.is_empty());
    assert_eq!(counters(&db).await, (1, 0, 1, 0));
}

#[tokio::test]
async fn mark_failed_sets_failed_at_without_a_counter_bump() {
    let db = seeded().await;
    let token = create_recipient(&db, recipient(Some(TOPIC), Some(CONTACT)))
        .await
        .unwrap();
    mark_failed(&db, &token).await.unwrap();
    assert!(recipient_row(&db, &token).await.5);
    assert_eq!(counters(&db).await, (1, 0, 0, 0));
}

#[tokio::test]
async fn surfaces_a_recording_failure_so_the_caller_can_enqueue_the_repair() {
    let db = seeded().await;
    let token = create_recipient(&db, recipient(Some(TOPIC), Some(CONTACT)))
        .await
        .unwrap();
    break_updates(&db).await;
    assert!(record_transition(&db, Phase::Opened, &token).await.is_err());
}

// ---- recordTransitionOrEnqueueRepair -------------------------------------------

#[tokio::test]
async fn does_not_enqueue_a_repair_when_the_transition_succeeds() {
    let db = seeded().await;
    let token = create_recipient(&db, recipient(Some(TOPIC), Some(CONTACT)))
        .await
        .unwrap();
    let jobs = RecordingEnqueuer::new();
    record_transition_or_enqueue_repair(
        &db,
        Some(jobs.as_ref()),
        Phase::Opened,
        &token,
        Some(WS),
        None,
    )
    .await;
    assert!(jobs.calls().is_empty());
    assert_eq!(counters(&db).await.2, 1);
}

#[tokio::test]
async fn enqueues_a_deduplicated_repair_with_the_link_and_never_fails() {
    let db = seeded().await;
    let token = create_recipient(&db, recipient(Some(TOPIC), Some(CONTACT)))
        .await
        .unwrap();
    break_updates(&db).await;
    let jobs = RecordingEnqueuer::new();
    record_transition_or_enqueue_repair(
        &db,
        Some(jobs.as_ref()),
        Phase::Clicked,
        &token,
        Some(WS),
        Some("https://example.com/a"),
    )
    .await;
    let calls = jobs.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].queue, QueueName::Integration);
    assert_eq!(calls[0].name, "recordEmailTrackingTransition");
    assert_eq!(
        calls[0].data,
        json!({
            "type": "recordEmailTrackingTransition",
            "data": {
                "phase": "clicked", "token": token, "workspaceId": WS.to_string(),
                "link": "https://example.com/a",
            },
        })
    );
    assert_eq!(
        calls[0].opts.job_id.as_deref(),
        Some(transition_job_id(&token, Phase::Clicked).as_str())
    );
    assert_eq!(calls[0].opts.attempts, Some(5));

    // Still never fails when the repair can not be enqueued.
    jobs.fail_next("redis down");
    record_transition_or_enqueue_repair(
        &db,
        Some(jobs.as_ref()),
        Phase::Opened,
        &token,
        None,
        None,
    )
    .await;
    assert_eq!(jobs.calls().len(), 2);
}

// ---- emailTrackingTransitionJobId -----------------------------------------------

#[test]
fn the_repair_job_id_is_stable_per_recipient_and_phase_and_hides_the_token() {
    let token = "1f0e2b8c-6d7a-4c3b-9a1e-5f4d3c2b1a00";
    let id = transition_job_id(token, Phase::Opened);
    assert_eq!(id, transition_job_id(token, Phase::Opened));
    assert_ne!(id, transition_job_id(token, Phase::Clicked));
    assert_ne!(id, transition_job_id("another-token", Phase::Opened));
    assert!(!id.contains(token));
    assert!(!id.contains(':'));
    assert!(id.starts_with("email-tracking-"));
    assert!(id.ends_with("-opened"));
    assert_eq!(id.len(), "email-tracking-".len() + 32 + "-opened".len());
}

// ---- email-topic-click-url.test.ts --------------------------------------------

#[test]
fn the_click_token_round_trips_is_url_safe_and_rejects_tampering() {
    let url = "https://example.com/landing?a=1&b=two";
    let token = sign_click_url(url, WS).unwrap();
    assert!(
        token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    );
    let payload = verify_click_token(&token).unwrap();
    assert_eq!(payload.url, url);
    assert_eq!(
        payload.workspace_id.as_deref(),
        Some(WS.to_string().as_str())
    );

    let mut tampered = token.into_bytes();
    let i = tampered.len() / 2;
    tampered[i] = if tampered[i] == b'A' { b'B' } else { b'A' };
    assert!(verify_click_token(&String::from_utf8(tampered).unwrap()).is_err());
    assert!(verify_click_token("not-a-token").is_err());
}

// ---- leaderboard/email-sources.ts (`awardLeaderboardEmailEngagement`) -----------

#[tokio::test]
async fn scores_a_topic_emails_first_open_and_each_link_once_on_the_smtp_inbox() {
    use chatbotx_business::email_tracking::award_leaderboard_engagement;
    let db = seeded().await;
    setup(&db, |tx| {
        common::inbox(tx, 91, WS, "smtp")?;
        tx.execute(
            r#"INSERT INTO "IntegrationSmtp" ("id", "auth", "name", "fromAddress", "workspaceId", "inboxId") VALUES (92, '{}', 'Mail', 'a@b.c', ?1, 91)"#,
            [WS],
        )?;
        let now = chatbotx_db::repo::now_ms();
        tx.execute(
            r#"INSERT INTO "Leaderboard" ("id", "workspaceId", "name", "startsAt", "endsAt", "status", "platform", "inboxId", "scope", "rules", "tagIds", "rewardSettings")
               VALUES (93, ?1, 'Board', ?2, ?3, 'active', 'email', 91, '{}', ?4, '[]', '{}')"#,
            params![
                WS,
                now - 86_400_000,
                now + 86_400_000,
                json!({ "emailOpen": { "enabled": true, "points": 1 }, "emailClick": { "enabled": true, "points": 2 } }).to_string()
            ],
        )?;
        Ok(())
    })
    .await;
    let jobs = RecordingEnqueuer::new();
    let token = create_recipient(&db, recipient(Some(TOPIC), Some(CONTACT)))
        .await
        .unwrap();
    // Before the open is recorded there is no event time: nothing scores.
    award_leaderboard_engagement(&db, Some(jobs.as_ref()), &token, true, None).await;
    assert!(jobs.calls().is_empty());

    record_transition(&db, Phase::Opened, &token).await.unwrap();
    record_transition(&db, Phase::Clicked, &token)
        .await
        .unwrap();
    award_leaderboard_engagement(&db, Some(jobs.as_ref()), &token, true, None).await;
    award_leaderboard_engagement(
        &db,
        Some(jobs.as_ref()),
        &token,
        false,
        Some("https://x.test/a"),
    )
    .await;
    let calls = jobs.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].data["data"]["rule"], "emailOpen");
    assert_eq!(calls[0].data["data"]["inboxId"], "91");
    let open_ref = calls[0].data["data"]["sourceRef"]
        .as_str()
        .unwrap()
        .to_owned();
    let click_ref = calls[1].data["data"]["sourceRef"].as_str().unwrap();
    assert_eq!(calls[1].data["data"]["rule"], "emailClick");
    // `<recipient id>:<16 hex of sha256(link)>`.
    assert!(click_ref.starts_with(&format!("{open_ref}:")));
    assert_eq!(click_ref.len(), open_ref.len() + 17);

    // A topic-less email never scores.
    let plain = create_recipient(&db, recipient(None, Some(CONTACT)))
        .await
        .unwrap();
    record_transition(&db, Phase::Opened, &plain).await.unwrap();
    award_leaderboard_engagement(&db, Some(jobs.as_ref()), &plain, true, None).await;
    assert_eq!(jobs.calls().len(), 2);
}
