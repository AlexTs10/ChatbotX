//! Port of `packages/database/__tests__/facebook-marketing-message-repository.test.ts`
//! (P3.2b). The TS renders the SQL of each call (a proxy over the Drizzle builders) and
//! matches the predicate text; here each call runs on a migrated temp file (adapted):
//! every predicate is a set of rows it must pick or spare.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo11.rs"]
mod repo11;

use chatbotx_db::repo::facebook_marketing_message::{self as campaigns, UpdateInput};
use chatbotx_db::repo::facebook_marketing_message_recipient::{
    self as recipients, FailRemainingInput,
};
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::{Json, patch::Patch};
use chatbotx_db::rusqlite::types::Value;
use repo11::{WS, campaign, put_campaign, put_recipient, read, recipient, two_ws_db, write};
use serde_json::json;

#[tokio::test]
async fn recipient_claim_for_send_only_claims_a_due_pending_row_of_a_campaign_that_is_still_sending()
 {
    let db = two_ws_db().await;
    let now = now_ms();
    put_campaign(&db, 1, WS, "sending", "").await;
    put_campaign(&db, 2, WS, "paused", "").await;
    put_recipient(&db, 11, 1, "pending", "").await;
    put_recipient(
        &db,
        12,
        1,
        "pending",
        &format!("\"nextAttemptAt\" = {}", now + 60_000),
    )
    .await;
    put_recipient(&db, 13, 1, "pending", &format!("\"nextAttemptAt\" = {now}")).await;
    put_recipient(&db, 14, 1, "unconfirmed", "").await;
    put_recipient(&db, 15, 2, "pending", "").await;
    // "id" = $ and "status" = $ (pending); nextAttemptAt is null or <= now; the campaign
    // EXISTS with status 'sending'.
    for id in [12, 14, 15] {
        assert!(
            write(&db, move |c| recipients::claim_for_send(c, id, now))
                .await
                .is_none(),
            "recipient {id} must not be claimed"
        );
    }
    let row = write(&db, move |c| recipients::claim_for_send(c, 11, now))
        .await
        .expect("claimed");
    assert_eq!(row.status, "unconfirmed");
    assert_eq!(row.sent_at, Some(now));
    assert_eq!(row.attempts, 1);
    assert!(
        write(&db, move |c| recipients::claim_for_send(c, 13, now))
            .await
            .is_some()
    );
    assert_eq!(recipient(&db, 12).await.status, "pending");
    assert_eq!(recipient(&db, 15).await.status, "pending");
    // A claimed row is no longer pending.
    assert!(
        write(&db, move |c| recipients::claim_for_send(c, 11, now))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn recipient_settle_send_only_updates_a_row_that_is_still_unconfirmed_a_webhook_result_wins()
{
    let db = two_ws_db().await;
    put_campaign(&db, 1, WS, "sending", "").await;
    put_recipient(&db, 11, 1, "unconfirmed", "").await;
    put_recipient(&db, 12, 1, "delivered", "").await;
    for id in [11, 12] {
        let patch: Patch = vec![("status", Value::Text("sent".to_owned()))];
        write(&db, move |c| recipients::settle_send(c, id, &patch)).await;
    }
    assert_eq!(recipient(&db, 11).await.status, "sent");
    assert_eq!(recipient(&db, 12).await.status, "delivered");
}

#[tokio::test]
async fn recipient_fail_remaining_can_be_limited_to_given_statuses() {
    let db = two_ws_db().await;
    put_campaign(&db, 1, WS, "sending", "").await;
    put_campaign(&db, 2, WS, "sending", "").await;
    put_recipient(&db, 11, 1, "pending", "").await;
    put_recipient(&db, 12, 1, "unconfirmed", "").await;
    put_recipient(&db, 13, 1, "sent", "").await;
    put_recipient(&db, 14, 2, "unconfirmed", "").await;
    let now = now_ms();
    let input = FailRemainingInput {
        campaign_row_id: 1,
        statuses: Some(vec!["unconfirmed".to_owned()]),
        error_code: None,
        error_message: "outcomeUnknown".to_owned(),
        error_source: None,
        now,
    };
    let count = write(&db, move |c| recipients::fail_remaining(c, &input)).await;
    assert_eq!(count, 1);
    let failed = recipient(&db, 12).await;
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.error_message.as_deref(), Some("outcomeUnknown"));
    assert_eq!((failed.error_code, failed.failed_at), (None, Some(now)));
    assert_eq!(recipient(&db, 11).await.status, "pending");
    assert_eq!(recipient(&db, 13).await.status, "sent");
    assert_eq!(recipient(&db, 14).await.status, "unconfirmed");
}

fn base(id: i64) -> UpdateInput {
    UpdateInput {
        id,
        workspace_id: WS,
        name: "n".to_owned(),
        budget_type: "daily".to_owned(),
        budget_minor_units: 100,
        ..UpdateInput::default()
    }
}

#[tokio::test]
async fn campaign_update_a_content_change_only_applies_while_nothing_was_sent_and_the_campaign_has_not_started()
 {
    let db = two_ws_db().await;
    put_campaign(&db, 1, WS, "notSent", "").await;
    put_campaign(&db, 2, WS, "scheduled", "").await;
    put_campaign(&db, 3, WS, "scheduled", "\"firstSentAt\" = 1000").await;
    put_campaign(&db, 4, WS, "sending", "").await;
    let edit = |id: i64| UpdateInput {
        content: Some(json!({ "a": 1 })),
        lock_content: true,
        ..base(id)
    };
    for id in [1, 2] {
        let input = edit(id);
        let row = write(&db, move |c| campaigns::update(c, &input))
            .await
            .expect("updated");
        assert_eq!(row.content, Json(json!({ "a": 1 })));
        assert_eq!(row.name, "n");
    }
    // "firstSentAt" is null; "status" in ('notSent', 'scheduled').
    for id in [3, 4] {
        let input = edit(id);
        assert!(
            write(&db, move |c| campaigns::update(c, &input))
                .await
                .is_none()
        );
        let row = campaign(&db, id).await;
        assert_eq!(
            (row.content, row.name),
            (Json(json!({ "v": 0 })), format!("c{id}"))
        );
    }
}

#[tokio::test]
async fn campaign_update_a_name_budget_only_edit_is_unconditional_and_leaves_content_alone() {
    let db = two_ws_db().await;
    put_campaign(&db, 1, WS, "sending", "\"firstSentAt\" = 1000").await;
    let input = UpdateInput {
        budget_type: "lifetime".to_owned(),
        budget_minor_units: 900,
        ..base(1)
    };
    let row = write(&db, move |c| campaigns::update(c, &input))
        .await
        .expect("updated");
    assert_eq!(row.name, "n");
    assert_eq!(
        (row.budget_type.as_str(), row.budget_minor_units),
        ("lifetime", 900)
    );
    assert_eq!(row.content, Json(json!({ "v": 0 })));
}

#[tokio::test]
async fn campaign_list_due_to_start_a_scheduled_campaign_waits_for_its_retry_backoff_next_day_at() {
    let db = two_ws_db().await;
    let now = now_ms();
    let past = now - 60_000;
    let future = now + 60_000;
    let rows: [(i64, &str, String); 8] = [
        (1, "scheduled", format!("\"startsAt\" = {past}")),
        (
            2,
            "scheduled",
            format!("\"startsAt\" = {past}, \"nextDayAt\" = {future}"),
        ),
        (
            3,
            "scheduled",
            format!("\"startsAt\" = {past}, \"nextDayAt\" = {now}"),
        ),
        (4, "scheduled", format!("\"startsAt\" = {future}")),
        (5, "waitingNextDay", format!("\"nextDayAt\" = {past}")),
        (6, "waitingNextDay", format!("\"nextDayAt\" = {future}")),
        (7, "waitingNextDay", String::new()),
        (
            8,
            "sending",
            format!("\"startsAt\" = {past}, \"nextDayAt\" = {past}"),
        ),
    ];
    for (id, status, set) in rows {
        put_campaign(&db, id, WS, status, &set).await;
    }
    let due = read(&db, move |c| campaigns::list_due_to_start(c, now)).await;
    let mut ids: Vec<i64> = due.iter().map(|r| r.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 3, 5]);
}
