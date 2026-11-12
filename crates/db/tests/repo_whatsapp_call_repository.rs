//! Port of `packages/database/__tests__/whatsapp-call-repository.test.ts` (P3.3).
//! Mocked Drizzle chains -> the observable effect on real rows of a migrated SQLite file
//! ("no UPDATE" = the stored row, `updatedAt` included, is unchanged).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo6.rs"]
mod repo6;

use chatbotx_db::repo::models::WhatsappCall;
use chatbotx_db::repo::whatsapp_call::*;
use chatbotx_db::repo::{self, now_ms};
use repo_common::{ok, setup};
use repo6::*;
use serde_json::json;

const MIN: i64 = 60_000;
const DAY: i64 = 24 * 60 * MIN;

async fn db_with(set: &'static str) -> common::TestDb {
    let (db, _) = call_db().await;
    setup(&db, move |tx| put_call(tx, 1, T0, set)).await;
    db
}

async fn finalize(db: &chatbotx_db::Db, input: FinalizeInput) -> Option<WhatsappCall> {
    ok(repo::write(db, move |tx| finalize_by_id(tx, &input)).await)
}

fn fin(status: &str, outcome: &str) -> FinalizeInput {
    FinalizeInput {
        id: 1,
        status: status.to_owned(),
        outcome: outcome.to_owned(),
        ..FinalizeInput::default()
    }
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:97 "canAdvanceStatus > never allows anything to advance past completed"
#[test]
fn can_advance_status_never_allows_anything_to_advance_past_completed() {
    for next in ["ringing", "accepted", "rejected", "failed", "completed"] {
        assert!(!can_advance_status("completed", next), "{next}");
    }
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:105 "canAdvanceStatus > allows rejected to overwrite a stale failed"
#[test]
fn can_advance_status_allows_rejected_to_overwrite_a_stale_failed() {
    assert!(can_advance_status("failed", "rejected"));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:109 "canAdvanceStatus > blocks a same-or-lower rank transition"
#[test]
fn can_advance_status_blocks_a_same_or_lower_rank_transition() {
    assert!(!can_advance_status("accepted", "ringing"));
    assert!(!can_advance_status("accepted", "accepted"));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:114 "canAdvanceStatus > allows a strictly higher rank transition"
#[test]
fn can_advance_status_allows_a_strictly_higher_rank_transition() {
    assert!(can_advance_status("ringing", "accepted"));
    assert!(can_advance_status("accepted", "completed"));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:121 "whatsappCallRepository.findByIdForWorkspace > scopes the lookup by workspaceId in the WHERE clause (B7 defense-in-depth)"
#[tokio::test]
async fn whatsapp_call_repository_find_by_id_for_workspace_scopes_the_lookup_by_workspace_id_in_the_where_clause_b7_defense_in_depth()
 {
    let db = db_with("").await;
    let result = ok(repo::read(&db, |c| find_by_id_for_workspace(c, 1, 2001)).await);
    assert_eq!(result, call_row(&db, 1).await);
    assert!(result.is_some());
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:138 "whatsappCallRepository.findByIdForWorkspace > returns undefined when no row matches (wrong workspace or missing id)"
#[tokio::test]
async fn whatsapp_call_repository_find_by_id_for_workspace_returns_undefined_when_no_row_matches_wrong_workspace_or_missing_id()
 {
    let db = db_with("").await;
    assert_eq!(
        ok(repo::read(&db, |c| find_by_id_for_workspace(c, 1, 2002)).await),
        None
    );
    assert_eq!(
        ok(repo::read(&db, |c| find_by_id_for_workspace(c, 999, 2001)).await),
        None
    );
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:153 "whatsappCallRepository.finalizeById > never downgrades a completed row, even if the caller asks"
#[tokio::test]
async fn whatsapp_call_repository_finalize_by_id_never_downgrades_a_completed_row_even_if_the_caller_asks()
 {
    let db = db_with("\"status\" = 'completed'").await;
    let before = call_row(&db, 1).await;
    assert_eq!(finalize(&db, fin("failed", "failed")).await, None);
    assert_eq!(call_row(&db, 1).await, before);
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:166 "whatsappCallRepository.finalizeById > advances ringing to completed and writes the terminal fields plus outcome"
#[tokio::test]
async fn whatsapp_call_repository_finalize_by_id_advances_ringing_to_completed_and_writes_the_terminal_fields_plus_outcome()
 {
    let db = db_with("\"status\" = 'accepted'").await;
    let input = FinalizeInput {
        ended_at: Some(Some(T0 + 5 * MIN)),
        duration_seconds: Some(Some(300)),
        ..fin("completed", "completed")
    };
    let result = finalize(&db, input).await.expect("updated");
    assert_eq!(result.status, "completed");
    assert_eq!(result.outcome.as_deref(), Some("completed"));
    assert_eq!(result.duration_seconds, Some(300));
    assert_eq!(result.ended_at, Some(T0 + 5 * MIN));
    assert_eq!(call_row(&db, 1).await, Some(result));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:194 "whatsappCallRepository.finalizeById > a permitted rejected → failed advance rewrites outcome from failed's input"
#[tokio::test]
async fn whatsapp_call_repository_finalize_by_id_a_permitted_rejected_failed_advance_rewrites_outcome_from_failed_s_input()
 {
    let db = db_with("\"status\" = 'rejected', \"outcome\" = 'rejected'").await;
    let input = FinalizeInput {
        ended_at: Some(Some(T0 + 5 * MIN)),
        ..fin("failed", "failed")
    };
    let result = finalize(&db, input).await.expect("updated");
    assert_eq!(
        (result.status.as_str(), result.outcome.as_deref()),
        ("failed", Some("failed"))
    );
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:216 "whatsappCallRepository.finalizeById > a permitted rejected → completed advance rewrites outcome from completed's input"
#[tokio::test]
async fn whatsapp_call_repository_finalize_by_id_a_permitted_rejected_completed_advance_rewrites_outcome_from_completed_s_input()
 {
    let db = db_with("\"status\" = 'rejected', \"outcome\" = 'rejected'").await;
    let input = FinalizeInput {
        ended_at: Some(Some(T0 + 5 * MIN)),
        ..fin("completed", "completed")
    };
    let result = finalize(&db, input).await.expect("updated");
    assert_eq!(
        (result.status.as_str(), result.outcome.as_deref()),
        ("completed", Some("completed"))
    );
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:238 "whatsappCallRepository.finalizeById > a disallowed downgrade (completed → failed) leaves outcome untouched"
#[tokio::test]
async fn whatsapp_call_repository_finalize_by_id_a_disallowed_downgrade_completed_failed_leaves_outcome_untouched()
 {
    let db = db_with("\"status\" = 'completed', \"outcome\" = 'completed'").await;
    let before = call_row(&db, 1).await;
    assert_eq!(finalize(&db, fin("failed", "failed")).await, None);
    assert_eq!(call_row(&db, 1).await, before);
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:253 "whatsappCallRepository.finalizeById idempotent endedAt fill > fills missing endedAt on a same-status terminate redelivery, status unchanged"
#[tokio::test]
async fn whatsapp_call_repository_finalize_by_id_idempotent_ended_at_fill_fills_missing_ended_at_on_a_same_status_terminate_redelivery_status_unchanged()
 {
    let db = db_with("\"status\" = 'rejected', \"outcome\" = 'rejected'").await;
    let input = FinalizeInput {
        ended_at: Some(Some(T0 + 5 * MIN)),
        ..fin("rejected", "rejected")
    };
    let result = finalize(&db, input).await.expect("filled");
    assert_eq!(result.status, "rejected");
    assert_eq!(result.outcome.as_deref(), Some("rejected"));
    assert_eq!(result.ended_at, Some(T0 + 5 * MIN));
    assert_eq!(call_row(&db, 1).await, Some(result));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:284 "whatsappCallRepository.finalizeById idempotent endedAt fill > does not overwrite an existing endedAt on a same-status terminate redelivery"
#[tokio::test]
async fn whatsapp_call_repository_finalize_by_id_idempotent_ended_at_fill_does_not_overwrite_an_existing_ended_at_on_a_same_status_terminate_redelivery()
 {
    let db =
        db_with("\"status\" = 'rejected', \"outcome\" = 'rejected', \"endedAt\" = 1785542460000")
            .await;
    let before = call_row(&db, 1).await;
    let input = FinalizeInput {
        ended_at: Some(Some(T0 + 9 * MIN)),
        ..fin("rejected", "rejected")
    };
    assert_eq!(finalize(&db, input).await, before);
    assert_eq!(call_row(&db, 1).await, before);
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:309 "whatsappCallRepository.finalizeById idempotent endedAt fill > still blocks a genuine downgrade attempt (different, lower-rank status)"
#[tokio::test]
async fn whatsapp_call_repository_finalize_by_id_idempotent_ended_at_fill_still_blocks_a_genuine_downgrade_attempt_different_lower_rank_status()
 {
    let db = db_with("\"status\" = 'completed', \"outcome\" = 'completed'").await;
    let before = call_row(&db, 1).await;
    assert_eq!(finalize(&db, fin("failed", "failed")).await, None);
    assert_eq!(call_row(&db, 1).await, before);
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:326 "whatsappCallRepository.finalizeById idempotent endedAt fill > fills every still-missing terminal field (not just endedAt) on a same-status redelivery"
#[tokio::test]
async fn whatsapp_call_repository_finalize_by_id_idempotent_ended_at_fill_fills_every_still_missing_terminal_field_not_just_ended_at_on_a_same_status_redelivery()
 {
    let db = db_with("\"status\" = 'rejected', \"outcome\" = 'rejected'").await;
    let input = FinalizeInput {
        ended_at: Some(Some(T0 + 5 * MIN)),
        message_id: Some(Some(77)),
        last_error: Some(Some("meta-timeout".to_owned())),
        ..fin("rejected", "rejected")
    };
    let result = finalize(&db, input).await.expect("filled");
    assert_eq!(result.ended_at, Some(T0 + 5 * MIN));
    assert_eq!(result.message_id, Some(77));
    assert_eq!(result.last_error.as_deref(), Some("meta-timeout"));
    assert_eq!(result.status, "rejected");
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:364 "whatsappCallRepository.finalizeById idempotent endedAt fill > does not clobber an already-set messageId on redelivery, even while filling the still-missing endedAt"
#[tokio::test]
async fn whatsapp_call_repository_finalize_by_id_idempotent_ended_at_fill_does_not_clobber_an_already_set_message_id_on_redelivery_even_while_filling_the_still_missing_ended_at()
 {
    let db = db_with("\"status\" = 'rejected', \"outcome\" = 'rejected', \"messageId\" = 55").await;
    let input = FinalizeInput {
        ended_at: Some(Some(T0 + 5 * MIN)),
        message_id: Some(Some(66)),
        ..fin("rejected", "rejected")
    };
    let result = finalize(&db, input).await.expect("filled");
    assert_eq!(result.ended_at, Some(T0 + 5 * MIN));
    assert_eq!(result.message_id, Some(55));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:404 "whatsappCallRepository.finalizeById outcome fill-when-null > fills outcome on a same-status redelivery when the current row has none yet (legacy row)"
#[tokio::test]
async fn whatsapp_call_repository_finalize_by_id_outcome_fill_when_null_fills_outcome_on_a_same_status_redelivery_when_the_current_row_has_none_yet_legacy_row()
 {
    let db = db_with("\"status\" = 'failed', \"endedAt\" = 1785542700000").await;
    let input = FinalizeInput {
        ended_at: Some(Some(T0 + 5 * MIN)),
        ..fin("failed", "failed")
    };
    let result = finalize(&db, input).await.expect("filled");
    assert_eq!(result.outcome.as_deref(), Some("failed"));
    assert_eq!(result.ended_at, Some(T0 + 5 * MIN));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:432 "whatsappCallRepository.finalizeById outcome fill-when-null > a canceled outcome survives a same-status failed redelivery (never clobbered back to failed)"
#[tokio::test]
async fn whatsapp_call_repository_finalize_by_id_outcome_fill_when_null_a_canceled_outcome_survives_a_same_status_failed_redelivery_never_clobbered_back_to_failed()
 {
    let db = db_with(
        "\"status\" = 'failed', \"outcome\" = 'canceled', \"endedAt\" = 1785542700000, \"lastError\" = 'canceled_by_business'",
    )
    .await;
    let before = call_row(&db, 1).await;
    let input = FinalizeInput {
        ended_at: Some(Some(T0 + 5 * MIN)),
        last_error: Some(Some("canceled_by_business".to_owned())),
        ..fin("failed", "failed")
    };
    let result = finalize(&db, input).await;
    assert_eq!(result, before);
    assert_eq!(result.and_then(|r| r.outcome).as_deref(), Some("canceled"));
    assert_eq!(call_row(&db, 1).await, before);
}

async fn interim(db: &chatbotx_db::Db, status: &'static str) -> Option<String> {
    ok(repo::write(db, move |tx| {
        update_interim_status(tx, "wacid.1", status, None)
    })
    .await)
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:468 "whatsappCallRepository.updateInterimStatus outcome > a RINGING interim event against an already-ringing row is a same-rank no-op (never sets outcome)"
#[tokio::test]
async fn whatsapp_call_repository_update_interim_status_outcome_a_ringing_interim_event_against_an_already_ringing_row_is_a_same_rank_no_op_never_sets_outcome()
 {
    let db = db_with("\"wacid\" = 'wacid.1'").await;
    let before = call_row(&db, 1).await;
    assert_eq!(interim(&db, "ringing").await, None);
    assert_eq!(call_row(&db, 1).await, before);
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:481 "whatsappCallRepository.updateInterimStatus outcome > accepted leaves outcome untouched (no outcome key in the SET)"
#[tokio::test]
async fn whatsapp_call_repository_update_interim_status_outcome_accepted_leaves_outcome_untouched_no_outcome_key_in_the_set()
 {
    let db = db_with("\"wacid\" = 'wacid.1'").await;
    assert_eq!(interim(&db, "accepted").await.as_deref(), Some("ringing"));
    let row = call_row(&db, 1).await.expect("row");
    assert_eq!((row.status.as_str(), row.outcome), ("accepted", None));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:493 "whatsappCallRepository.updateInterimStatus outcome > rejected (terminal) sets outcome alongside status"
#[tokio::test]
async fn whatsapp_call_repository_update_interim_status_outcome_rejected_terminal_sets_outcome_alongside_status()
 {
    let db = db_with("\"wacid\" = 'wacid.1'").await;
    assert_eq!(interim(&db, "rejected").await.as_deref(), Some("ringing"));
    let row = call_row(&db, 1).await.expect("row");
    assert_eq!(
        (row.status.as_str(), row.outcome.as_deref()),
        ("rejected", Some("rejected"))
    );
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:509 "whatsappCallRepository.updateInterimStatus outcome > the failed → rejected repair (permitted by canAdvanceStatus) rewrites both columns"
#[tokio::test]
async fn whatsapp_call_repository_update_interim_status_outcome_the_failed_rejected_repair_permitted_by_can_advance_status_rewrites_both_columns()
 {
    let db = db_with("\"wacid\" = 'wacid.1', \"status\" = 'failed', \"outcome\" = 'failed'").await;
    assert_eq!(interim(&db, "rejected").await.as_deref(), Some("failed"));
    let row = call_row(&db, 1).await.expect("row");
    assert_eq!(
        (row.status.as_str(), row.outcome.as_deref()),
        ("rejected", Some("rejected"))
    );
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:531 "whatsappCallRepository.markAcceptedIfActive > applies from ringing"
#[tokio::test]
async fn whatsapp_call_repository_mark_accepted_if_active_applies_from_ringing() {
    let db = db_with("").await;
    let result =
        ok(repo::write(&db, |tx| mark_accepted_if_active(tx, 1, 1001)).await).expect("applied");
    assert_eq!(
        (result.status.as_str(), result.answered_by_user_id),
        ("accepted", Some(1001))
    );
    assert_eq!(call_row(&db, 1).await, Some(result));
}

async fn no_op_from(status: &'static str) {
    let (db, _) = call_db().await;
    setup(&db, move |tx| {
        put_call(tx, 1, T0, &format!("\"status\" = '{status}'"))
    })
    .await;
    let before = call_row(&db, 1).await;
    assert_eq!(
        ok(repo::write(&db, |tx| mark_accepted_if_active(tx, 1, 1001)).await),
        None
    );
    assert_eq!(call_row(&db, 1).await, before);
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:547 "whatsappCallRepository.markAcceptedIfActive > no-ops from terminal status rejected"
#[tokio::test]
async fn whatsapp_call_repository_mark_accepted_if_active_no_ops_from_terminal_status_rejected() {
    no_op_from("rejected").await;
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:547 "whatsappCallRepository.markAcceptedIfActive > no-ops from terminal status completed"
#[tokio::test]
async fn whatsapp_call_repository_mark_accepted_if_active_no_ops_from_terminal_status_completed() {
    no_op_from("completed").await;
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:547 "whatsappCallRepository.markAcceptedIfActive > no-ops from terminal status failed"
#[tokio::test]
async fn whatsapp_call_repository_mark_accepted_if_active_no_ops_from_terminal_status_failed() {
    no_op_from("failed").await;
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:562 "whatsappCallRepository.markAcceptedIfActive > sets answeredByUserId only on apply"
#[tokio::test]
async fn whatsapp_call_repository_mark_accepted_if_active_sets_answered_by_user_id_only_on_apply() {
    let (db, _) = call_db().await;
    setup(&db, |tx| {
        put_user(tx, 1002)?;
        put_call(
            tx,
            1,
            T0,
            "\"status\" = 'completed', \"answeredByUserId\" = 1001",
        )?;
        put_call(tx, 2, T0, "")
    })
    .await;
    assert_eq!(
        ok(repo::write(&db, |tx| mark_accepted_if_active(tx, 1, 1002)).await),
        None
    );
    assert_eq!(
        call_row(&db, 1).await.expect("row 1").answered_by_user_id,
        Some(1001)
    );
    ok(repo::write(&db, |tx| mark_accepted_if_active(tx, 2, 1002)).await);
    assert_eq!(
        call_row(&db, 2).await.expect("row 2").answered_by_user_id,
        Some(1002)
    );
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:594 "whatsappCallRepository.touchLivenessIfStale > wins the claim: bumps updatedAt behind a status + staleness guard"
#[tokio::test]
async fn whatsapp_call_repository_touch_liveness_if_stale_wins_the_claim_bumps_updated_at_behind_a_status_staleness_guard()
 {
    let (db, _) = call_db().await;
    setup(&db, |tx| {
        put_call(tx, 1, T0 - MIN, "\"status\" = 'accepted'")?;
        put_call(tx, 2, T0 - MIN, "")
    })
    .await;
    assert!(ok(repo::write(&db, |tx| touch_liveness_if_stale(
        tx, 1, T0
    ))
    .await));
    assert!(call_row(&db, 1).await.expect("row").updated_at > T0);
    assert!(!ok(repo::write(&db, |tx| touch_liveness_if_stale(
        tx, 2, T0
    ))
    .await));
    assert_eq!(call_row(&db, 2).await.expect("row").updated_at, T0 - MIN);
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:614 "whatsappCallRepository.touchLivenessIfStale > loses the claim when the row was updated since the cutoff"
#[tokio::test]
async fn whatsapp_call_repository_touch_liveness_if_stale_loses_the_claim_when_the_row_was_updated_since_the_cutoff()
 {
    let (db, _) = call_db().await;
    setup(&db, |tx| {
        put_call(tx, 1, T0 + MIN, "\"status\" = 'accepted'")
    })
    .await;
    assert!(!ok(repo::write(&db, |tx| touch_liveness_if_stale(
        tx, 1, T0
    ))
    .await));
    assert_eq!(call_row(&db, 1).await.expect("row").updated_at, T0 + MIN);
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:627 "whatsappCallRepository.attachWacid > is a no-op when the row already carries this exact wacid"
#[tokio::test]
async fn whatsapp_call_repository_attach_wacid_is_a_no_op_when_the_row_already_carries_this_exact_wacid()
 {
    let db = db_with("\"wacid\" = 'wamid.1'").await;
    let before = call_row(&db, 1).await;
    assert_eq!(
        ok(repo::write(&db, |tx| attach_wacid(tx, 1, "wamid.1")).await),
        before
    );
    assert_eq!(call_row(&db, 1).await, before);
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:640 "whatsappCallRepository.attachWacid > merges the newer row into the older one on a wacid collision"
#[tokio::test]
async fn whatsapp_call_repository_attach_wacid_merges_the_newer_row_into_the_older_one_on_a_wacid_collision()
 {
    let (db, _) = call_db().await;
    setup(&db, |tx| {
        put_call(
            tx,
            1,
            T0,
            "\"wacid\" = 'wamid.1', \"recordingPath\" = 'space/ws-1/calls/call-older.ogg'",
        )?;
        put_call(tx, 2, T0 + 5 * MIN, "\"attemptId\" = 'attempt-newer'")
    })
    .await;
    let merged = ok(repo::write(&db, |tx| attach_wacid(tx, 2, "wamid.1")).await).expect("merged");
    assert_eq!(merged.id, 1);
    assert_eq!(merged.attempt_id.as_deref(), Some("attempt-newer"));
    assert_eq!(
        merged.recording_path.as_deref(),
        Some("space/ws-1/calls/call-older.ogg")
    );
    assert_eq!(
        call_rows(&db)
            .await
            .iter()
            .map(|c| c.id)
            .collect::<Vec<_>>(),
        vec![1]
    );
}

fn outbound(attempt: &str) -> PendingOutboundInput {
    PendingOutboundInput {
        attempt_id: attempt.to_owned(),
        workspace_id: 2001,
        inbox_id: 3001,
        contact_inbox_id: CALL_CI,
        conversation_id: CALL_CONV,
        ..PendingOutboundInput::default()
    }
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:694 "whatsappCallRepository.createPendingOutbound > inserts a null-wacid businessInitiated ringing row"
#[tokio::test]
async fn whatsapp_call_repository_create_pending_outbound_inserts_a_null_wacid_business_initiated_ringing_row()
 {
    let (db, _) = call_db().await;
    let row = ok(repo::write(&db, |tx| create_pending_outbound(tx, &outbound("att-1"))).await);
    assert_eq!(row.attempt_id.as_deref(), Some("att-1"));
    assert_eq!(row.wacid, None);
    assert_eq!(
        (row.direction.as_str(), row.status.as_str()),
        ("businessInitiated", "ringing")
    );
    assert_eq!(row.answered_by_user_id, None);
    assert_eq!(call_row(&db, row.id).await, Some(row));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:724 "whatsappCallRepository.createPendingOutbound > stamps answeredByUserId with the initiator when provided (VoIP outbound)"
#[tokio::test]
async fn whatsapp_call_repository_create_pending_outbound_stamps_answered_by_user_id_with_the_initiator_when_provided_vo_ip_outbound()
 {
    let (db, _) = call_db().await;
    let input = PendingOutboundInput {
        answered_by_user_id: Some(1001),
        ..outbound("att-1")
    };
    let row = ok(repo::write(&db, move |tx| create_pending_outbound(tx, &input)).await);
    assert_eq!(row.attempt_id.as_deref(), Some("att-1"));
    assert_eq!(row.answered_by_user_id, Some(1001));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:756 "whatsappCallRepository.createPendingOutbound > maps the pendingOutbound unique-index violation to a typed error"
#[tokio::test]
async fn whatsapp_call_repository_create_pending_outbound_maps_the_pending_outbound_unique_index_violation_to_a_typed_error()
 {
    let (db, _) = call_db().await;
    ok(repo::write(&db, |tx| create_pending_outbound(tx, &outbound("att-1"))).await);
    let error = repo::write(&db, |tx| create_pending_outbound(tx, &outbound("att-2")))
        .await
        .expect_err("pending outbound exists");
    assert!(
        error.message().contains("pending-outbound-exists"),
        "{error}"
    );
    assert_eq!(error.code(), Some("pending-outbound-exists"));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:793 "whatsappCallRepository.markRecordingArrangement > stores that a recording is coming, clearing any failure reason"
#[tokio::test]
async fn whatsapp_call_repository_mark_recording_arrangement_stores_that_a_recording_is_coming_clearing_any_failure_reason()
 {
    let db = db_with("\"recordingFailureReason\" = 'old-reason'").await;
    let row = ok(repo::write(&db, |tx| mark_recording_arrangement(tx, 1, true, None)).await)
        .expect("row");
    assert_eq!(
        (
            row.recording_requested,
            row.recording_failure_reason.clone()
        ),
        (Some(true), None)
    );
    assert_eq!(call_row(&db, 1).await, Some(row));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:809 "whatsappCallRepository.markRecordingArrangement > stores why no recording is coming"
#[tokio::test]
async fn whatsapp_call_repository_mark_recording_arrangement_stores_why_no_recording_is_coming() {
    let db = db_with("").await;
    ok(repo::write(&db, |tx| {
        mark_recording_arrangement(tx, 1, false, Some("meta-rejected-recording-announcement"))
    })
    .await);
    let row = call_row(&db, 1).await.expect("row");
    assert_eq!(row.recording_requested, Some(false));
    assert_eq!(
        row.recording_failure_reason.as_deref(),
        Some("meta-rejected-recording-announcement")
    );
}

const STRANDED: &str = "stranded-accepted-recovered-on-dial";

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:842 "whatsappCallRepository.recoverStrandedAccepted > terminalizes the stale accepted row in one guarded statement"
#[tokio::test]
async fn whatsapp_call_repository_recover_stranded_accepted_terminalizes_the_stale_accepted_row_in_one_guarded_statement()
 {
    let db = db_with("\"status\" = 'accepted'").await;
    let row = ok(repo::write(&db, |tx| {
        recover_stranded_accepted(tx, 1, T0 + 10 * MIN, STRANDED)
    })
    .await)
    .expect("row");
    assert_eq!(
        (row.status.as_str(), row.outcome.as_deref()),
        ("completed", Some("completed"))
    );
    assert_eq!(row.last_error.as_deref(), Some(STRANDED));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:865 "whatsappCallRepository.recoverStrandedAccepted > returns undefined when a heartbeat or a real terminate got there first"
#[tokio::test]
async fn whatsapp_call_repository_recover_stranded_accepted_returns_undefined_when_a_heartbeat_or_a_real_terminate_got_there_first()
 {
    let (db, _) = call_db().await;
    setup(&db, |tx| {
        put_call(tx, 1, T0 + 20 * MIN, "\"status\" = 'accepted'")?;
        put_call(tx, 2, T0, "\"status\" = 'completed'")
    })
    .await;
    assert_eq!(
        ok(repo::write(&db, |tx| recover_stranded_accepted(
            tx,
            1,
            T0 + 10 * MIN,
            STRANDED
        ))
        .await),
        None
    );
    assert_eq!(
        ok(repo::write(&db, |tx| recover_stranded_accepted(
            tx,
            2,
            T0 + 10 * MIN,
            STRANDED
        ))
        .await),
        None
    );
    assert_eq!(call_row(&db, 1).await.expect("row").status, "accepted");
}

async fn active(db: &chatbotx_db::Db) -> Option<WhatsappCall> {
    ok(repo::read(db, |c| find_active_by_contact_inbox(c, 3001, CALL_CI)).await)
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:875 "whatsappCallRepository.findActiveByContactInbox > returns a ringing/accepted row regardless of direction (glare guard)"
#[tokio::test]
async fn whatsapp_call_repository_find_active_by_contact_inbox_returns_a_ringing_accepted_row_regardless_of_direction_glare_guard()
 {
    let (db, _) = call_db().await;
    setup(&db, |tx| {
        put_call(tx, 1, T0, "")?;
        put_call(tx, 2, T0 + MIN, "\"status\" = 'completed'")
    })
    .await;
    assert_eq!(active(&db).await.map(|c| c.id), Some(1));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:899 "whatsappCallRepository.findActiveByContactInbox > also matches a businessInitiated active row"
#[tokio::test]
async fn whatsapp_call_repository_find_active_by_contact_inbox_also_matches_a_business_initiated_active_row()
 {
    let db = db_with("\"direction\" = 'businessInitiated', \"status\" = 'accepted'").await;
    assert_eq!(active(&db).await, call_row(&db, 1).await);
    assert!(active(&db).await.is_some());
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:922 "whatsappCallRepository.findActiveByContactInbox > returns undefined when nothing is active"
#[tokio::test]
async fn whatsapp_call_repository_find_active_by_contact_inbox_returns_undefined_when_nothing_is_active()
 {
    let db = db_with("\"status\" = 'completed'").await;
    assert_eq!(active(&db).await, None);
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:946 "whatsappCallRepository.findRingingByWorkspace > scopes to workspace/ringing/wacid-present/unanswered rows, newest first, bounded by limit"
#[tokio::test]
async fn whatsapp_call_repository_find_ringing_by_workspace_scopes_to_workspace_ringing_wacid_present_unanswered_rows_newest_first_bounded_by_limit()
 {
    let (db, _) = call_db().await;
    setup(&db, |tx| {
        put_call(tx, 1, T0 + 100 * MIN, "")?;
        put_call(
            tx,
            2,
            T0 + 100 * MIN,
            "\"wacid\" = 'w-answered', \"answeredByUserId\" = 1001",
        )?;
        put_call(
            tx,
            3,
            T0 + 100 * MIN,
            "\"wacid\" = 'w-accepted', \"status\" = 'accepted'",
        )?;
        for i in 0..22_i64 {
            put_call(tx, 100 + i, T0 + i * MIN, &format!("\"wacid\" = 'w-{i}'"))?;
        }
        Ok(0)
    })
    .await;
    let rows = ok(repo::read(&db, |c| find_ringing_by_workspace(c, 2001)).await);
    assert_eq!(rows.len(), 20);
    assert_eq!(
        rows.iter().map(|c| c.id).collect::<Vec<_>>(),
        (102..122).rev().collect::<Vec<_>>()
    );
    assert!(ok(repo::read(&db, |c| find_ringing_by_workspace(c, 2002)).await).is_empty());
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:970 "whatsappCallRepository.findRingingByWorkspace > returns an empty array when nothing is ringing"
#[tokio::test]
async fn whatsapp_call_repository_find_ringing_by_workspace_returns_an_empty_array_when_nothing_is_ringing()
 {
    let db = db_with("\"wacid\" = 'w-1', \"status\" = 'accepted'").await;
    assert!(ok(repo::read(&db, |c| find_ringing_by_workspace(c, 2001)).await).is_empty());
}

fn put_retention(
    tx: &chatbotx_db::rusqlite::Connection,
    days: i64,
) -> Result<usize, chatbotx_db::DbError> {
    ins!(tx, "IntegrationWhatsapp", {
        "id" => 9001_i64,
        "auth" => "{}".to_owned(),
        "phoneNumberId" => "phone-1".to_owned(),
        "wabaId" => "waba-1".to_owned(),
        "businessId" => "business-1".to_owned(),
        "name" => "WhatsApp".to_owned(),
        "workspaceId" => 2001_i64,
        "inboxId" => 3001_i64,
        "callRecordingRetentionDays" => days,
    })
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:994 "whatsappCallRepository.listRecordingsPastRetention > joins IntegrationWhatsapp and bounds by limit, never offset"
#[tokio::test]
async fn whatsapp_call_repository_list_recordings_past_retention_joins_integration_whatsapp_and_bounds_by_limit_never_offset()
 {
    let (db, _) = call_db().await;
    setup(&db, |tx| {
        put_retention(tx, 30)?;
        let rec = |days: i64| {
            format!(
                "\"recordingPath\" = 'space/ws-1/calls/c.ogg', \"recordedAt\" = {}",
                T0 - days * DAY
            )
        };
        put_call(tx, 1, T0 - 100 * DAY, &rec(100))?;
        put_call(tx, 2, T0 - 10 * DAY, &rec(10))?;
        put_call(tx, 3, T0 - 45 * DAY, &rec(45))
    })
    .await;
    let mut all: Vec<i64> =
        ok(repo::read(&db, |c| list_recordings_past_retention(c, 500, T0)).await)
            .iter()
            .map(|c| c.id)
            .collect();
    all.sort_unstable();
    assert_eq!(all, vec![1, 3]);
    assert_eq!(
        ok(repo::read(&db, |c| list_recordings_past_retention(c, 1, T0)).await).len(),
        1
    );
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:1024 "whatsappCallRepository.clearRecording > nulls recordingPath/recordedAt and keeps the transcript untouched"
#[tokio::test]
async fn whatsapp_call_repository_clear_recording_nulls_recording_path_recorded_at_and_keeps_the_transcript_untouched()
 {
    let db = db_with(
        "\"recordingPath\" = 'rec/1.ogg', \"recordedAt\" = 1785542400000, \"transcript\" = 'hello'",
    )
    .await;
    ok(repo::write(&db, |tx| clear_recording(tx, 1)).await);
    let row = call_row(&db, 1).await.expect("row");
    assert_eq!((row.recording_path, row.recorded_at), (None, None));
    assert_eq!(row.transcript.as_deref(), Some("hello"));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:1043 "whatsappCallRepository.attachTranscript > stamps the flat transcript only when segments are omitted (SIP/Whisper)"
#[tokio::test]
async fn whatsapp_call_repository_attach_transcript_stamps_the_flat_transcript_only_when_segments_are_omitted_sip_whisper()
 {
    let db = db_with("").await;
    let input = AttachTranscriptInput {
        id: 1,
        transcript: "hello there".to_owned(),
        transcribed_at: T0,
        segments: None,
    };
    let row = ok(repo::write(&db, move |tx| attach_transcript(tx, &input)).await).expect("row");
    assert_eq!(row.transcript.as_deref(), Some("hello there"));
    assert_eq!(row.transcribed_at, Some(T0));
    assert_eq!(row.transcript_segments, None);
    assert_eq!(call_row(&db, 1).await, Some(row));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:1066 "whatsappCallRepository.attachTranscript > also persists diarized transcriptSegments when provided (Meta-native VoIP)"
#[tokio::test]
async fn whatsapp_call_repository_attach_transcript_also_persists_diarized_transcript_segments_when_provided_meta_native_vo_ip()
 {
    let db = db_with("").await;
    let segments = json!([
        { "speaker": "Business", "channel": 0, "start": 0, "end": 1.2, "text": "Hello" },
        { "speaker": "Customer", "channel": 1, "start": 1.5, "end": 3, "text": "Hi there" },
    ]);
    let input = AttachTranscriptInput {
        id: 1,
        transcript: "Hello Hi there".to_owned(),
        transcribed_at: T0,
        segments: Some(Some(segments.clone())),
    };
    let row = ok(repo::write(&db, move |tx| attach_transcript(tx, &input)).await).expect("row");
    assert_eq!(row.transcript.as_deref(), Some("Hello Hi there"));
    assert_eq!(row.transcript_segments.map(|j| j.0), Some(segments));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:1096 "whatsappCallRepository.attachTranscript > is a no-op (undefined) when the row already has a transcript"
#[tokio::test]
async fn whatsapp_call_repository_attach_transcript_is_a_no_op_undefined_when_the_row_already_has_a_transcript()
 {
    let db = db_with("\"transcript\" = 'first'").await;
    let input = AttachTranscriptInput {
        id: 1,
        transcript: "redelivered".to_owned(),
        transcribed_at: T0,
        segments: None,
    };
    assert_eq!(
        ok(repo::write(&db, move |tx| attach_transcript(tx, &input)).await),
        None
    );
    assert_eq!(
        call_row(&db, 1).await.expect("row").transcript.as_deref(),
        Some("first")
    );
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:1109 "whatsappCallRepository.attachAiSummary > persists the summary, provider, and aiSummarizedAt exactly once"
#[tokio::test]
async fn whatsapp_call_repository_attach_ai_summary_persists_the_summary_provider_and_ai_summarized_at_exactly_once()
 {
    let db = db_with("").await;
    let at = repo_common::ms("2026-09-01T00:00:00.000Z");
    let input = AiSummaryInput {
        id: 1,
        ai_summary: json!({ "summary": "Customer asked about pricing." }),
        ai_summary_provider: "openai".to_owned(),
        ai_summarized_at: Some(at),
    };
    let again = AiSummaryInput {
        ai_summary: json!({ "summary": "second" }),
        ..input.clone()
    };
    let row = ok(repo::write(&db, move |tx| attach_ai_summary(tx, &input)).await).expect("row");
    assert_eq!(
        row.ai_summary.clone().map(|j| j.0),
        Some(json!({ "summary": "Customer asked about pricing." }))
    );
    assert_eq!(
        (row.ai_summary_provider.as_deref(), row.ai_summarized_at),
        (Some("openai"), Some(at))
    );
    assert_eq!(
        ok(repo::write(&db, move |tx| attach_ai_summary(tx, &again)).await),
        None
    );
    assert_eq!(call_row(&db, 1).await, Some(row));
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:1137 "whatsappCallRepository.attachAiSummary > is a no-op (undefined) when the row already has aiSummarizedAt"
#[tokio::test]
async fn whatsapp_call_repository_attach_ai_summary_is_a_no_op_undefined_when_the_row_already_has_ai_summarized_at()
 {
    let db = db_with("\"aiSummarizedAt\" = 1785542400000").await;
    let input = AiSummaryInput {
        id: 1,
        ai_summary: json!({ "summary": "redelivered" }),
        ai_summary_provider: "openai".to_owned(),
        ai_summarized_at: Some(now_ms()),
    };
    assert_eq!(
        ok(repo::write(&db, move |tx| attach_ai_summary(tx, &input)).await),
        None
    );
    assert_eq!(call_row(&db, 1).await.expect("row").ai_summary, None);
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:1155 "whatsappCallRepository.createPendingOutbound initiatedByUserId > stamps initiatedByUserId independently of answeredByUserId"
#[tokio::test]
async fn whatsapp_call_repository_create_pending_outbound_initiated_by_user_id_stamps_initiated_by_user_id_independently_of_answered_by_user_id()
 {
    let (db, _) = call_db().await;
    let input = PendingOutboundInput {
        answered_by_user_id: Some(1001),
        initiated_by_user_id: Some(1001),
        ..outbound("att-1")
    };
    let row = ok(repo::write(&db, move |tx| create_pending_outbound(tx, &input)).await);
    assert_eq!(
        (row.answered_by_user_id, row.initiated_by_user_id),
        (Some(1001), Some(1001))
    );
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:1189 "whatsappCallRepository.createPendingOutbound initiatedByUserId > defaults initiatedByUserId to null when omitted"
#[tokio::test]
async fn whatsapp_call_repository_create_pending_outbound_initiated_by_user_id_defaults_initiated_by_user_id_to_null_when_omitted()
 {
    let (db, _) = call_db().await;
    let row = ok(repo::write(&db, |tx| create_pending_outbound(tx, &outbound("att-1"))).await);
    assert_eq!(row.initiated_by_user_id, None);
}

/// Port of: packages/database/__tests__/whatsapp-call-repository.test.ts:1252 "whatsappCallRepository.listRecordingsPastRetention > returns the call rows unwrapped and defaults `now` to the clock"
#[tokio::test]
async fn whatsapp_call_repository_list_recordings_past_retention_returns_the_call_rows_unwrapped_and_defaults_now_to_the_clock()
 {
    let (db, _) = call_db().await;
    setup(&db, |tx| {
        put_retention(tx, 90)?;
        put_call(
            tx,
            9,
            T0,
            "\"recordingPath\" = 'rec/9.ogg', \"recordedAt\" = 0",
        )
    })
    .await;
    let rows = ok(repo::read(&db, |c| list_recordings_past_retention(c, 500, now_ms())).await);
    assert_eq!(Some(rows), call_row(&db, 9).await.map(|r| vec![r]));
}
