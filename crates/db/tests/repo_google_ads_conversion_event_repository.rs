//! Port of `packages/database/__tests__/google-ads-conversion-event-repository.test.ts`
//! (P3.2b). The TS captures the mocked `set` object and renders the `where` SQL text; here
//! each repository function runs on a migrated temp file (adapted): the fence of each
//! `"col" = $n` fragment is a row it spares, the `set` object is the stored row after the
//! call.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo11.rs"]
mod repo11;

use chatbotx_db::repo::google_ads_conversion_event::{
    self as gace, ApplyProcessingResultInput, ClaimInput, EventRef, FinishSendingInput,
    FinishSendingProcessedInput, ListInput, RedriveInput, StatsFilters, StrandedInput,
};
use chatbotx_db::repo::{Json, iso};
use repo11::{
    OTHER_WS, WS, event, event_options, event_values, i, ms, null, put_event, put_events, read, t,
    two_ws_db, write,
};
use serde_json::json;

fn claim(id: i64, workspace_id: i64, attempt: i64, token: &str) -> ClaimInput {
    ClaimInput {
        id,
        workspace_id,
        claim_token: token.to_owned(),
        attempt,
    }
}

fn r#ref(id: i64, workspace_id: i64) -> EventRef {
    EventRef { id, workspace_id }
}

async fn put_sending(db: &chatbotx_db::Db, id: i64, attempt: i64) {
    put_event(
        db,
        id,
        WS,
        vec![
            ("status", t("sending")),
            ("attempt", i(attempt)),
            ("claimToken", t("token")),
            ("claimedAt", i(ms("2026-10-05T00:00:00Z"))),
        ],
    )
    .await;
}

// ---------------------------------------------------------------------------
// leases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn google_ads_conversion_event_repository_leases_claim_for_sending_only_claims_a_pending_row_of_the_job_s_generation()
 {
    let db = two_ws_db().await;
    put_event(&db, 1, WS, vec![("attempt", i(3))]).await;
    put_event(&db, 2, WS, vec![("status", t("failed")), ("attempt", i(3))]).await;
    // "workspaceId" = $, "status" = $, "attempt" = $: each fence spares a row.
    assert!(
        write(&db, |c| gace::claim_for_sending(
            c,
            &claim(1, OTHER_WS, 3, "token")
        ))
        .await
        .is_none()
    );
    assert!(
        write(&db, |c| gace::claim_for_sending(
            c,
            &claim(2, WS, 3, "token")
        ))
        .await
        .is_none()
    );
    assert!(
        write(&db, |c| gace::claim_for_sending(
            c,
            &claim(1, WS, 2, "token")
        ))
        .await
        .is_none()
    );
    let claimed = write(&db, |c| {
        gace::claim_for_sending(c, &claim(1, WS, 3, "token"))
    })
    .await
    .expect("claimed");
    assert_eq!(claimed.id, 1);
    assert_eq!(claimed.status, "sending");
    assert_eq!(claimed.claim_token.as_deref(), Some("token"));
    assert!(claimed.claimed_at.is_some());
    assert_eq!(event(&db, 1).await.expect("row"), claimed);
    assert_eq!(event(&db, 2).await.expect("row").status, "failed");
}

#[tokio::test]
async fn google_ads_conversion_event_repository_leases_claim_for_sending_returns_null_when_nothing_matched_stale_generation()
 {
    let db = two_ws_db().await;
    put_event(&db, 1, WS, vec![("attempt", i(1))]).await;
    assert!(
        write(&db, |c| gace::claim_for_sending(
            c,
            &claim(1, WS, 0, "token")
        ))
        .await
        .is_none()
    );
    let row = event(&db, 1).await.expect("row");
    assert_eq!((row.status.as_str(), row.claim_token), ("pending", None));
}

#[tokio::test]
async fn google_ads_conversion_event_repository_leases_finish_sending_is_fenced_by_the_claim_token_and_clears_the_lease()
 {
    let db = two_ws_db().await;
    put_sending(&db, 1, 0).await;
    let input = |token: &str| FinishSendingInput {
        id: 1,
        workspace_id: WS,
        claim_token: token.to_owned(),
        to: "sent".to_owned(),
        request_id: Some("req".to_owned()),
        sent_at: Some(ms("2026-10-05T00:00:00Z")),
        processing_status: Some("processing".to_owned()),
        ..FinishSendingInput::default()
    };
    let wrong = input("other");
    assert!(
        write(&db, move |c| gace::finish_sending(c, &wrong))
            .await
            .is_none()
    );
    assert_eq!(event(&db, 1).await.expect("row").status, "sending");
    let right = input("token");
    let row = write(&db, move |c| gace::finish_sending(c, &right))
        .await
        .expect("finished");
    assert_eq!(row.status, "sent");
    assert_eq!((row.claim_token, row.claimed_at), (None, None));
    assert_eq!(row.request_id.as_deref(), Some("req"));
    assert_eq!(row.processing_status.as_deref(), Some("processing"));
    assert_eq!(row.sent_at, Some(ms("2026-10-05T00:00:00Z")));
    // "status" = $: a finished row is no longer `sending`, so a second finish is a no-op.
    let again = input("token");
    assert!(
        write(&db, move |c| gace::finish_sending(c, &again))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn google_ads_conversion_event_repository_leases_release_claim_moves_to_the_next_generation_under_the_same_token()
 {
    let db = two_ws_db().await;
    put_sending(&db, 1, 3).await;
    assert!(
        write(&db, |c| gace::release_claim(c, r#ref(1, WS), "other", 4))
            .await
            .is_none()
    );
    let row = write(&db, |c| gace::release_claim(c, r#ref(1, WS), "token", 4))
        .await
        .expect("released");
    assert_eq!(row.status, "pending");
    assert_eq!(row.attempt, 4);
    assert_eq!((row.claim_token, row.claimed_at), (None, None));
}

#[tokio::test]
async fn google_ads_conversion_event_repository_leases_touch_stranded_bumps_updated_at_only_for_the_row_s_id_workspace_status_and_generation()
 {
    let db = two_ws_db().await;
    put_event(&db, 1, WS, vec![("attempt", i(4)), ("updatedAt", i(1_000))]).await;
    put_event(&db, 2, WS, vec![("attempt", i(4)), ("updatedAt", i(1_000))]).await;
    for (ws, status, attempt) in [
        (OTHER_WS, "pending", 4),
        (WS, "sending", 4),
        (WS, "pending", 3),
    ] {
        write(&db, move |c| {
            gace::touch_stranded(c, r#ref(1, ws), status, attempt)
        })
        .await;
        assert_eq!(event(&db, 1).await.expect("row").updated_at, 1_000);
    }
    let mut expected = event(&db, 1).await.expect("row");
    write(&db, |c| gace::touch_stranded(c, r#ref(1, WS), "pending", 4)).await;
    let touched = event(&db, 1).await.expect("row");
    assert!(touched.updated_at > 1_000);
    expected.updated_at = touched.updated_at;
    assert_eq!(touched, expected);
    // The set is `{ updatedAt }` only, and only row 1 matched its id.
    assert_eq!(event(&db, 2).await.expect("row").updated_at, 1_000);
    assert_eq!(touched.status, "pending");
    assert_eq!(touched.attempt, 4);
}

#[tokio::test]
async fn google_ads_conversion_event_repository_leases_redrive_increments_the_generation_and_only_matches_the_expected_one()
 {
    let db = two_ws_db().await;
    let failed = |detail: &str| {
        vec![
            ("status", t("failed")),
            ("attempt", i(1)),
            ("uploadMethod", t("legacy")),
            ("failureStage", t("delivery")),
            ("error", t("boom")),
            ("requestId", t("legacy:1")),
            ("sentAt", i(ms("2026-10-05T00:00:00Z"))),
            ("processingStatus", t("failed")),
            ("processingCheckedAt", i(ms("2026-10-05T00:01:00Z"))),
            ("processingAttempts", i(2)),
            ("nextProcessingCheckAt", i(ms("2026-10-05T00:02:00Z"))),
            ("processingDetail", t(detail)),
        ]
    };
    put_event(
        &db,
        1,
        WS,
        failed(r#"{"sendAttemptedAt":"2026-10-05T00:00:00.000Z","requestStatus":"FAILED"}"#),
    )
    .await;
    put_event(&db, 2, WS, failed(r#"{"requestStatus":"FAILED"}"#)).await;
    let input = |id: i64, from: &str, expected_attempt: i64| RedriveInput {
        id,
        workspace_id: WS,
        from_statuses: vec![from.to_owned()],
        expected_attempt,
        ..RedriveInput::default()
    };
    // "status" in ($..), "attempt" = $.
    let stale = input(1, "failed", 0);
    assert!(
        write(&db, move |c| gace::redrive(c, &stale))
            .await
            .is_none()
    );
    let wrong_status = input(1, "sent", 1);
    assert!(
        write(&db, move |c| gace::redrive(c, &wrong_status))
            .await
            .is_none()
    );
    let first = input(1, "failed", 1);
    let row = write(&db, move |c| gace::redrive(c, &first))
        .await
        .expect("redriven");
    assert_eq!(row.status, "pending");
    assert_eq!(row.attempt, 2);
    assert_eq!(row.claim_token, None);
    assert_eq!(row.failure_stage, None);
    assert_eq!(row.error, None);
    assert_eq!(row.processing_status, None);
    assert_eq!(row.request_id, None);
    assert_eq!(row.sent_at, None);
    assert_eq!(row.processing_checked_at, None);
    assert_eq!(row.processing_attempts, 0);
    assert_eq!(row.next_processing_check_at, None);
    // Only the "an upload may have left" stamp outlives a redrive (ELSE NULL otherwise).
    assert_eq!(
        row.processing_detail,
        Some(Json(
            json!({ "sendAttemptedAt": "2026-10-05T00:00:00.000Z" })
        ))
    );
    // The transport is pinned at record time and never changed by a redrive.
    assert_eq!(row.upload_method, "legacy");
    let second = input(2, "failed", 1);
    let row = write(&db, move |c| gace::redrive(c, &second))
        .await
        .expect("redriven");
    assert_eq!(row.processing_detail, None);
}

fn processing_input(
    to: &str,
    status: &str,
    expected_request_id: Option<&str>,
    expected_attempt: i64,
) -> ApplyProcessingResultInput {
    ApplyProcessingResultInput {
        id: 1,
        workspace_id: WS,
        to: to.to_owned(),
        processing_status: status.to_owned(),
        next_processing_check_at: None,
        processing_attempts: 2,
        expected_request_id: expected_request_id.map(str::to_owned),
        expected_attempt,
        ..ApplyProcessingResultInput::default()
    }
}

#[tokio::test]
async fn google_ads_conversion_event_repository_leases_apply_processing_result_only_updates_a_still_sent_event()
 {
    let db = two_ws_db().await;
    let sent = vec![
        ("status", t("sent")),
        ("attempt", i(3)),
        ("requestId", t("req-1")),
        ("sentAt", i(ms("2026-10-05T00:00:00Z"))),
        ("processingStatus", t("processing")),
        ("nextProcessingCheckAt", i(ms("2026-10-05T00:05:00Z"))),
    ];
    put_event(&db, 1, WS, sent).await;
    let misses = [
        (OTHER_WS, Some("req-1"), 3),
        (WS, Some("req-2"), 3),
        (WS, Some("req-1"), 2),
    ];
    for (ws, request, attempt) in misses {
        let mut input = processing_input("processed", "success", request, attempt);
        input.workspace_id = ws;
        assert!(
            write(&db, move |c| gace::apply_processing_result(c, &input))
                .await
                .is_none()
        );
    }
    let input = processing_input("processed", "success", Some("req-1"), 3);
    let row = write(&db, move |c| gace::apply_processing_result(c, &input))
        .await
        .expect("applied");
    assert_eq!(row.status, "processed");
    assert_eq!(row.processing_status.as_deref(), Some("success"));
    assert_eq!(row.next_processing_check_at, None);
    assert_eq!(row.processing_attempts, 2);
    assert!(row.processing_checked_at.is_some());
    // "status" = 'sent': the processed row is not polled again.
    let again = processing_input("processed", "success", Some("req-1"), 3);
    assert!(
        write(&db, move |c| gace::apply_processing_result(c, &again))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn google_ads_conversion_event_repository_leases_apply_processing_result_fences_a_request_less_event_with_is_null()
 {
    let db = two_ws_db().await;
    // A `sent` row always has a request (CHECK sent_requires_request), so the `IS NULL`
    // fence of `expectedRequestId: null` never matches it (and it is not dropped).
    put_event(
        &db,
        1,
        WS,
        vec![
            ("status", t("sent")),
            ("requestId", t("req-1")),
            ("sentAt", i(ms("2026-10-05T00:00:00Z"))),
        ],
    )
    .await;
    let input = processing_input("sent", "unknown", None, 0);
    assert!(
        write(&db, move |c| gace::apply_processing_result(c, &input))
            .await
            .is_none()
    );
    let row = event(&db, 1).await.expect("row");
    assert_eq!((row.status.as_str(), row.processing_status), ("sent", None));
}

#[tokio::test]
async fn google_ads_conversion_event_repository_leases_redrive_fences_on_the_request_id_only_when_one_is_given()
 {
    let db = two_ws_db().await;
    put_event(
        &db,
        1,
        WS,
        vec![
            ("status", t("sent")),
            ("attempt", i(1)),
            ("requestId", t("req-1")),
            ("sentAt", i(ms("2026-10-05T00:00:00Z"))),
        ],
    )
    .await;
    put_event(
        &db,
        2,
        WS,
        vec![
            ("status", t("failed")),
            ("attempt", i(1)),
            ("requestId", t("req-9")),
            ("sentAt", i(ms("2026-10-05T00:00:00Z"))),
        ],
    )
    .await;
    let with_request = |request: &str| RedriveInput {
        id: 1,
        workspace_id: WS,
        from_statuses: vec!["sent".to_owned()],
        expected_attempt: 1,
        expected_request_id: Some(request.to_owned()),
        expected_claim_token: None,
    };
    let other = with_request("req-2");
    assert!(
        write(&db, move |c| gace::redrive(c, &other))
            .await
            .is_none()
    );
    let same = with_request("req-1");
    assert!(write(&db, move |c| gace::redrive(c, &same)).await.is_some());
    // Without `expectedRequestId` the stored request does not matter.
    let without = RedriveInput {
        id: 2,
        workspace_id: WS,
        from_statuses: vec!["failed".to_owned()],
        expected_attempt: 1,
        ..RedriveInput::default()
    };
    let row = write(&db, move |c| gace::redrive(c, &without))
        .await
        .expect("redriven");
    assert_eq!((row.status.as_str(), row.attempt), ("pending", 2));
}

// ---------------------------------------------------------------------------
// reads
// ---------------------------------------------------------------------------

#[tokio::test]
async fn google_ads_conversion_event_repository_reads_list_by_workspace_filters_by_workspace_and_clamps_per_page()
 {
    let db = two_ws_db().await;
    let since = ms("2026-10-01T00:00:00Z");
    let until = ms("2026-10-02T00:00:00Z");
    let mid = ms("2026-10-01T12:00:00Z");
    let matching = |at: i64| {
        vec![
            ("status", t("failed")),
            ("channel", t("messenger")),
            ("occurredAt", i(at)),
        ]
    };
    // 203 matching rows: id 1 at `since`, id 2 at `until` (both bounds inclusive).
    put_event(&db, 1, WS, matching(since)).await;
    put_event(&db, 2, WS, matching(until)).await;
    put_events(&db, 3, 201, WS, matching(mid)).await;
    // One miss per filter.
    put_event(&db, 900, OTHER_WS, matching(mid)).await;
    put_event(
        &db,
        901,
        WS,
        vec![("channel", t("messenger")), ("occurredAt", i(mid))],
    )
    .await;
    put_event(
        &db,
        902,
        WS,
        vec![("status", t("failed")), ("occurredAt", i(mid))],
    )
    .await;
    put_event(&db, 903, WS, matching(since - 1)).await;
    put_event(&db, 904, WS, matching(until + 1)).await;
    let input = ListInput {
        workspace_id: WS,
        status: Some("failed".to_owned()),
        channel: Some("messenger".to_owned()),
        since: Some(since),
        until: Some(until),
        page: 3,
        per_page: 5000,
        ..ListInput::default()
    };
    let result = read(&db, move |c| gace::list_by_workspace(c, &input)).await;
    assert_eq!(result.total, 203);
    // perPage clamps to 100, so page 3 is offset 200 of (occurredAt DESC, id DESC).
    let ids: Vec<i64> = result.rows.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![4, 3, 1]);
}

#[tokio::test]
async fn google_ads_conversion_event_repository_reads_list_due_for_processing_check_selects_due_sent_events_oldest_first()
 {
    let db = two_ws_db().await;
    let now = ms("2026-10-05T00:00:00Z");
    let sent = |next: Option<i64>, method: &str| {
        vec![
            ("status", t("sent")),
            ("uploadMethod", t(method)),
            ("requestId", t("req")),
            ("sentAt", i(now - 60_000)),
            ("nextProcessingCheckAt", next.map_or_else(null, i)),
        ]
    };
    put_event(&db, 1, WS, sent(Some(now - 1_000), "dataManager")).await;
    put_event(&db, 2, WS, sent(Some(now), "dataManager")).await;
    put_event(&db, 3, WS, sent(Some(now + 1), "dataManager")).await;
    put_event(&db, 4, WS, sent(Some(now - 9_000), "legacy")).await;
    put_event(&db, 5, WS, sent(None, "dataManager")).await;
    let mut processed = sent(Some(now - 9_000), "dataManager");
    processed[0] = ("status", t("processed"));
    put_event(&db, 6, WS, processed).await;
    put_event(&db, 7, OTHER_WS, sent(Some(now - 5_000), "dataManager")).await;
    let due = read(&db, move |c| {
        gace::list_due_for_processing_check(c, now, 50)
    })
    .await;
    let ids: Vec<i64> = due.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![7, 1, 2]);
    let limited = read(&db, move |c| gace::list_due_for_processing_check(c, now, 2)).await;
    assert_eq!(limited.len(), 2);
}

#[tokio::test]
async fn google_ads_conversion_event_repository_reads_list_stranded_runs_separate_separately_limited_queries_for_sending_and_pending_rows()
 {
    let db = two_ws_db().await;
    let cutoff = ms("2026-10-05T00:00:00Z");
    let sending = |claimed: i64| {
        vec![
            ("status", t("sending")),
            ("claimToken", t("token")),
            ("claimedAt", i(claimed)),
            // The sending window ignores the click gate.
            ("googleClickReceivedAt", i(cutoff + 1_000)),
            ("updatedAt", i(cutoff + 1_000)),
        ]
    };
    let pending = |updated: i64, click: i64| {
        vec![
            ("updatedAt", i(updated)),
            ("googleClickReceivedAt", i(click)),
            // The pending window ignores claimedAt.
            ("claimedAt", i(cutoff + 1_000)),
        ]
    };
    put_event(&db, 1, WS, sending(cutoff - 500)).await;
    put_event(&db, 2, WS, sending(cutoff - 900)).await;
    put_event(&db, 3, WS, sending(cutoff)).await;
    put_event(&db, 4, WS, pending(cutoff - 100, cutoff - 100)).await;
    put_event(&db, 5, WS, pending(cutoff - 300, cutoff - 100)).await;
    put_event(&db, 6, WS, pending(cutoff, cutoff - 100)).await;
    put_event(&db, 7, WS, pending(cutoff - 100, cutoff)).await;
    let input = move |limit: i64| StrandedInput {
        pending_older_than: cutoff,
        six_hour_gate_before: cutoff,
        sending_older_than: cutoff,
        limit,
    };
    let all = read(&db, move |c| gace::list_stranded(c, &input(10))).await;
    let ids: Vec<i64> = all.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![2, 1, 5, 4]);
    // `limit` applies to each window.
    let one = read(&db, move |c| gace::list_stranded(c, &input(1))).await;
    let ids: Vec<i64> = one.iter().map(|r| r.id).collect();
    assert_eq!(ids, vec![2, 5]);
}

// ---------------------------------------------------------------------------
// insertIgnoreDuplicate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn google_ads_conversion_event_repository_insert_ignore_duplicate_targets_the_workspace_id_transaction_id_unique_index_and_maps_a_conflict_to_null()
 {
    let db = two_ws_db().await;
    let first = write(&db, |c| {
        gace::insert_ignore_duplicate(c, &event_values(WS, "t", &[]))
    })
    .await
    .expect("inserted");
    assert_eq!(
        (first.workspace_id, first.transaction_id.as_str()),
        (WS, "t")
    );
    let duplicate = write(&db, |c| {
        gace::insert_ignore_duplicate(c, &event_values(WS, "t", &[("channel", t("messenger"))]))
    })
    .await;
    assert!(duplicate.is_none());
    assert_eq!(event(&db, first.id).await.expect("row").channel, "whatsapp");
    // The target is (workspaceId, transactionId): the same id in another workspace inserts.
    let other = write(&db, |c| {
        gace::insert_ignore_duplicate(c, &event_values(OTHER_WS, "t", &[]))
    })
    .await
    .expect("inserted");
    assert_ne!(other.id, first.id);
}

#[tokio::test]
async fn google_ads_conversion_event_repository_insert_ignore_duplicate_writes_the_options_snapshot_with_the_row()
 {
    let db = two_ws_db().await;
    let mut options = event_options();
    options["marker"] = json!("snapshot");
    let text = options.to_string();
    let row = write(&db, move |c| {
        gace::insert_ignore_duplicate(c, &event_values(WS, "t", &[("options", t(&text))]))
    })
    .await
    .expect("inserted");
    assert_eq!(row.transaction_id, "t");
    assert_eq!(row.options, Some(Json(options.clone())));
    assert_eq!(
        event(&db, row.id).await.expect("row").options,
        Some(Json(options))
    );
}

// ---------------------------------------------------------------------------
// direct completion (C6)
// ---------------------------------------------------------------------------

fn completion() -> FinishSendingProcessedInput {
    FinishSendingProcessedInput {
        id: 1,
        workspace_id: WS,
        claim_token: "token".to_owned(),
        attempt: 4,
        request_id: "legacy:42".to_owned(),
        sent_at: ms("2026-10-07T00:00:00Z"),
        processing_detail: json!({
            "requestStatus": "LEGACY_UPLOAD_COMPLETED",
            "recordCount": 1,
            "errorCounts": [],
            "warningCounts": [],
        }),
    }
}

#[tokio::test]
async fn google_ads_conversion_event_repository_direct_completion_c6_finish_sending_processed_writes_the_whole_terminal_state_in_one_fenced_update()
 {
    let db = two_ws_db().await;
    put_event(
        &db,
        1,
        WS,
        vec![
            ("status", t("sending")),
            ("attempt", i(4)),
            ("claimToken", t("token")),
            ("claimedAt", i(ms("2026-10-06T23:59:00Z"))),
            ("error", t("old")),
            ("nextProcessingCheckAt", i(ms("2026-10-08T00:00:00Z"))),
        ],
    )
    .await;
    let misses = [(OTHER_WS, "token", 4), (WS, "other", 4), (WS, "token", 3)];
    for (ws, token, attempt) in misses {
        let mut input = completion();
        input.workspace_id = ws;
        input.claim_token = token.to_owned();
        input.attempt = attempt;
        assert!(
            write(&db, move |c| gace::finish_sending_processed(c, &input))
                .await
                .is_none()
        );
    }
    let input = completion();
    let done = write(&db, move |c| gace::finish_sending_processed(c, &input))
        .await
        .expect("done");
    assert_eq!(done.id, 1);
    assert_eq!(done.status, "processed");
    assert_eq!(done.processing_status.as_deref(), Some("success"));
    assert_eq!(done.request_id.as_deref(), Some("legacy:42"));
    assert_eq!(done.sent_at, Some(ms("2026-10-07T00:00:00Z")));
    assert_eq!(done.processing_attempts, 1);
    assert_eq!((done.claim_token.as_deref(), done.claimed_at), (None, None));
    assert_eq!(
        (done.error.as_deref(), done.failure_stage.as_deref()),
        (None, None)
    );
    assert_eq!(done.next_processing_check_at, None);
    assert!(done.processing_checked_at.is_some());
    assert_eq!(
        done.processing_detail,
        Some(Json(completion().processing_detail))
    );
}

#[tokio::test]
async fn google_ads_conversion_event_repository_direct_completion_c6_finish_sending_processed_returns_null_when_the_lease_is_gone()
 {
    let db = two_ws_db().await;
    put_event(&db, 1, WS, vec![("attempt", i(4))]).await;
    let input = completion();
    assert!(
        write(&db, move |c| gace::finish_sending_processed(c, &input))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn google_ads_conversion_event_repository_direct_completion_c6_mark_send_attempted_is_fenced_by_lease_and_generation()
 {
    let db = two_ws_db().await;
    put_sending(&db, 1, 4).await;
    put_event(
        &db,
        2,
        WS,
        vec![("attempt", i(4)), ("claimToken", t("token"))],
    )
    .await;
    let at = ms("2026-10-05T00:00:00Z");
    for (id, token, attempt) in [(1, "other", 4), (1, "token", 3), (2, "token", 4)] {
        assert!(
            write(&db, move |c| {
                gace::mark_send_attempted(c, &claim(id, WS, attempt, token), at)
            })
            .await
            .is_none()
        );
    }
    let row = write(&db, move |c| {
        gace::mark_send_attempted(c, &claim(1, WS, 4, "token"), at)
    })
    .await
    .expect("stamped");
    assert_eq!(
        row.processing_detail,
        Some(Json(json!({ "sendAttemptedAt": iso(at) })))
    );
    // The first stamp wins.
    let row = write(&db, move |c| {
        gace::mark_send_attempted(c, &claim(1, WS, 4, "token"), at + 60_000)
    })
    .await
    .expect("stamped");
    assert_eq!(
        row.processing_detail,
        Some(Json(
            json!({ "sendAttemptedAt": "2026-10-05T00:00:00.000Z" })
        ))
    );
}

// ---------------------------------------------------------------------------
// stats
// ---------------------------------------------------------------------------

fn window() -> StatsFilters {
    StatsFilters {
        workspace_id: WS,
        since: ms("2026-10-01T00:00:00Z"),
        until: ms("2026-10-07T23:59:59.999Z"),
        ..StatsFilters::default()
    }
}

fn sent_like(status: &str, at: &str) -> Vec<common::ColumnOverride> {
    vec![
        ("status", t(status)),
        ("requestId", t("req")),
        ("sentAt", i(ms("2026-10-02T00:00:00Z"))),
        ("occurredAt", i(ms(at))),
    ]
}

#[tokio::test]
async fn google_ads_conversion_event_repository_stats_stats_by_day_and_channel_puts_the_date_first_groups_by_ordinal_and_channel_counts_every_status_and_stage()
 {
    let db = two_ws_db().await;
    // 17:30Z on the 2nd is the 3rd in Ho Chi Minh (UTC+7).
    let day = |status: &str, stage: Option<&str>| {
        vec![
            ("status", t(status)),
            ("failureStage", stage.map_or_else(null, t)),
            ("occurredAt", i(ms("2026-10-02T17:30:00Z"))),
        ]
    };
    put_event(&db, 1, WS, day("failed", Some("delivery"))).await;
    put_event(&db, 2, WS, day("failed", Some("processing"))).await;
    put_event(&db, 3, WS, day("failed", Some("timeout"))).await;
    put_event(&db, 4, WS, day("failed", None)).await;
    put_event(&db, 5, WS, day("pending", None)).await;
    put_event(&db, 6, WS, day("skipped_no_account", None)).await;
    put_event(&db, 7, WS, day("skipped_expired", None)).await;
    put_event(&db, 8, WS, sent_like("sent", "2026-10-02T17:30:00Z")).await;
    put_event(&db, 9, WS, sent_like("processed", "2026-10-02T10:00:00Z")).await;
    let mut messenger = sent_like("processed", "2026-10-02T17:30:00Z");
    messenger.push(("channel", t("messenger")));
    put_event(&db, 10, WS, messenger).await;
    let mut rows = read(&db, |c| {
        gace::stats_by_day_and_channel(c, &window(), "Asia/Ho_Chi_Minh")
    })
    .await;
    rows.sort_by(|a, b| (&a.date, &a.channel).cmp(&(&b.date, &b.channel)));
    let keys: Vec<(String, String, i64)> = rows
        .iter()
        .map(|r| (r.date.clone(), r.channel.clone(), r.counts.processed))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("2026-10-02".to_owned(), "whatsapp".to_owned(), 1),
            ("2026-10-03".to_owned(), "messenger".to_owned(), 1),
            ("2026-10-03".to_owned(), "whatsapp".to_owned(), 0),
        ]
    );
    let row = &rows[2];
    assert_eq!(
        (
            row.counts.pending,
            row.counts.sending,
            row.counts.sent,
            row.counts.failed,
            row.counts.skipped_no_account,
            row.counts.skipped_expired,
        ),
        (1, 0, 1, 4, 1, 1)
    );
    // FILTER (WHERE "failureStage" is null) counts the stage-less failure directly.
    assert_eq!(
        (
            row.failed_delivery,
            row.failed_processing,
            row.failed_timeout,
            row.failed_unknown,
        ),
        (1, 1, 1, 1)
    );
}

#[tokio::test]
async fn google_ads_conversion_event_repository_stats_every_stats_read_is_workspace_scoped_and_bounded_by_the_window()
 {
    let db = two_ws_db().await;
    let valued = |at: &str, channel: &str, action: &str| {
        let mut row = sent_like("processed", at);
        row.extend([
            ("value", t("5")),
            ("currency", t("USD")),
            ("channel", t(channel)),
            ("conversionActionId", t(action)),
        ]);
        row
    };
    // Inside: two channels and two actions (no channel / action filter narrows them).
    put_event(
        &db,
        1,
        WS,
        valued("2026-10-01T00:00:00Z", "whatsapp", "111"),
    )
    .await;
    put_event(
        &db,
        2,
        WS,
        valued("2026-10-07T23:59:59.999Z", "messenger", "222"),
    )
    .await;
    // Outside: other workspace, before and after the window.
    put_event(
        &db,
        3,
        OTHER_WS,
        valued("2026-10-03T00:00:00Z", "whatsapp", "111"),
    )
    .await;
    put_event(
        &db,
        4,
        WS,
        valued("2026-09-30T23:59:59.999Z", "whatsapp", "111"),
    )
    .await;
    put_event(
        &db,
        5,
        WS,
        valued("2026-10-08T00:00:00Z", "whatsapp", "111"),
    )
    .await;
    let days = read(&db, |c| gace::stats_by_day_and_channel(c, &window(), "UTC")).await;
    assert_eq!(days.iter().map(|r| r.counts.processed).sum::<i64>(), 2);
    let actions = read(&db, |c| gace::stats_by_action(c, &window(), 50)).await;
    assert_eq!(actions.iter().map(|r| r.counts.processed).sum::<i64>(), 2);
    let values = read(&db, |c| gace::confirmed_value_by_currency(c, &window())).await;
    assert_eq!(values.len(), 1);
    assert_eq!((values[0].value.as_str(), values[0].count), ("10", 2));
}

#[tokio::test]
async fn google_ads_conversion_event_repository_stats_optional_channel_and_conversion_action_id_narrow_the_window()
 {
    let db = two_ws_db().await;
    let at = "2026-10-02T12:00:00Z";
    put_event(&db, 1, WS, sent_like("processed", at)).await;
    let mut messenger = sent_like("processed", at);
    messenger.push(("channel", t("messenger")));
    put_event(&db, 2, WS, messenger).await;
    let mut action = sent_like("processed", at);
    action.push(("conversionActionId", t("123")));
    put_event(&db, 3, WS, action).await;
    let mut both = sent_like("processed", at);
    both.push(("conversionActionId", t("123")));
    put_event(&db, 4, WS, both).await;
    let filters = StatsFilters {
        channel: Some("whatsapp".to_owned()),
        conversion_action_id: Some("123".to_owned()),
        ..window()
    };
    let rows = read(&db, move |c| {
        gace::stats_by_day_and_channel(c, &filters, "UTC")
    })
    .await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].channel.as_str(), rows[0].counts.processed),
        ("whatsapp", 2)
    );
}

#[tokio::test]
async fn google_ads_conversion_event_repository_stats_stats_by_action_groups_by_action_orders_by_volume_then_id_and_asks_for_one_extra_row()
 {
    let db = two_ws_db().await;
    let at = |id: i64, action: &str, name: &str, when: &str| {
        let mut row = sent_like("processed", when);
        row.extend([
            ("conversionActionId", t(action)),
            ("conversionActionName", t(name)),
        ]);
        (id, row)
    };
    for (id, row) in [
        at(1, "1", "Old", "2026-10-02T10:00:00Z"),
        at(2, "1", "New", "2026-10-04T10:00:00Z"),
        at(3, "3", "Three", "2026-10-02T10:00:00Z"),
        at(4, "2", "Two", "2026-10-02T10:00:00Z"),
        at(5, "4", "Four", "2026-10-02T10:00:00Z"),
    ] {
        put_event(&db, id, WS, row).await;
    }
    let rows = read(&db, |c| gace::stats_by_action(c, &window(), 2)).await;
    let ids: Vec<&str> = rows
        .iter()
        .map(|r| r.conversion_action_id.as_str())
        .collect();
    // count(*) DESC, then conversionActionId ASC; limit + 1 = 3 rows.
    assert_eq!(ids, vec!["1", "2", "3"]);
    // The latest snapshot (array_agg ORDER BY occurredAt DESC)[1].
    assert_eq!(rows[0].name.as_deref(), Some("New"));
    assert_eq!(rows[0].category.as_deref(), Some("QUALIFIED_LEAD"));
    assert_eq!(rows[0].counts.processed, 2);
}

#[tokio::test]
async fn google_ads_conversion_event_repository_stats_confirmed_value_by_currency_only_sums_processed_events_that_carry_a_value()
 {
    let db = two_ws_db().await;
    let at = "2026-10-02T12:00:00Z";
    let valued = |status: &str, value: &str, currency: &str| {
        let mut row = sent_like(status, at);
        row.extend([("value", t(value)), ("currency", t(currency))]);
        row
    };
    put_event(&db, 1, WS, valued("processed", "1.5", "USD")).await;
    put_event(&db, 2, WS, valued("processed", "2", "USD")).await;
    put_event(&db, 3, WS, valued("sent", "100", "USD")).await;
    let mut failed = valued("failed", "200", "USD");
    failed.retain(|(name, _)| *name != "requestId" && *name != "sentAt");
    put_event(&db, 4, WS, failed).await;
    put_event(&db, 5, WS, sent_like("processed", at)).await;
    let rows = read(&db, |c| gace::confirmed_value_by_currency(c, &window())).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].currency.as_deref(), Some("USD"));
    // sum(value)::text: exact decimal text.
    assert_eq!((rows[0].value.as_str(), rows[0].count), ("3.5", 2));
}

#[tokio::test]
async fn google_ads_conversion_event_repository_stats_exists_for_workspace_is_a_workspace_scoped_limit_1_probe()
 {
    let db = two_ws_db().await;
    put_events(&db, 1, 3, WS, vec![]).await;
    assert!(read(&db, |c| gace::exists_for_workspace(c, WS)).await);
    assert!(!read(&db, |c| gace::exists_for_workspace(c, OTHER_WS)).await);
}

#[tokio::test]
async fn google_ads_conversion_event_repository_stats_list_by_workspace_accepts_a_conversion_action_id_filter()
 {
    let db = two_ws_db().await;
    put_event(&db, 1, WS, vec![]).await;
    put_event(&db, 2, WS, vec![("conversionActionId", t("123"))]).await;
    let input = ListInput {
        workspace_id: WS,
        conversion_action_id: Some("123".to_owned()),
        page: 1,
        per_page: 10,
        ..ListInput::default()
    };
    let result = read(&db, move |c| gace::list_by_workspace(c, &input)).await;
    assert_eq!(result.total, 1);
    assert_eq!(
        result.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![2]
    );
}
