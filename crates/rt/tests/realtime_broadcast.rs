//! Port of `packages/business/__tests__/realtime-broadcast.test.ts` (P2.8
//! spec 4.6, R209-R222). `vi.useFakeTimers` is paused tokio time; the
//! mocked `publishSerializedRealtimeStreamRecord` is the hub buffer (each
//! appended `workspace-events` entry is one record); the mocked marker is
//! `FakeRevokeStore`; `defaultQueue.add` is `RecordingRevokeRetry`.
//! Not applicable (user decision 2026-11-12, DECISIONS P2.8): R211, R216,
//! R221, R222 (the in-process append cannot reject or stay in flight).

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chatbotx_rt::TestClock;
use chatbotx_rt::hub::{Entry, EntryKind};
use chatbotx_rt::protocol::{Envelope, RevokeReason, encode_workspace_record};
use chatbotx_rt::revoke::{
    REVOKE_RETRY_JOB_ATTEMPTS, REVOKE_RETRY_JOB_DELAY_MS, REVOKE_RETRY_JOB_NAME, RevokeArgs,
    revoke_retry_job_payload,
};
use serde_json::{Value, json};
use support::{SendMode, member_data, member_socket, test_realtime};

const NOW_MS: i64 = 1_790_000_000_000;

fn envelope(event: Value) -> Envelope {
    serde_json::from_value(event).unwrap()
}

fn typing_event() -> Envelope {
    envelope(json!({
        "data": { "typing": true },
        "eventType": "typing",
        "route": { "assignedTeamIds": [], "assignedUserIds": ["user_1"] },
    }))
}

fn workspace_events(entry: &Entry) -> Vec<Envelope> {
    match &entry.kind {
        EntryKind::WorkspaceEvents { events, .. } => events.clone(),
        other => panic!("not a workspace-events entry: {other:?}"),
    }
}

/// The record JSON of every appended `workspace-events` entry.
fn records(rt: &chatbotx_rt::Realtime, workspace_id: &str) -> Vec<String> {
    rt.hub()
        .buffered(workspace_id)
        .iter()
        .filter(|entry| matches!(entry.kind, EntryKind::WorkspaceEvents { .. }))
        .map(|entry| encode_workspace_record(workspace_id, &workspace_events(entry)))
        .collect()
}

fn revoke_args(revoked_at_ms: Option<u64>) -> RevokeArgs {
    RevokeArgs {
        workspace_id: "workspace_1".to_owned(),
        user_id: "user_1".to_owned(),
        reason: RevokeReason::Deleted,
        revoked_at_ms,
    }
}

/// Port of: packages/business/__tests__/realtime-broadcast.test.ts:92 "realtime stream broadcast > coalesces workspace events into one durable stream record"
#[tokio::test(start_paused = true)]
async fn realtime_stream_broadcast_coalesces_workspace_events_into_one_durable_stream_record() {
    let (rt, _fakes) = test_realtime(Arc::new(TestClock::new(NOW_MS)));
    let first = rt.publish_workspace_event("workspace_1", typing_event());
    let second = rt.publish_workspace_event("workspace_1", typing_event());

    tokio::time::advance(Duration::from_millis(25)).await;
    assert_eq!(first.await, Ok(()));
    assert_eq!(second.await, Ok(()));

    assert_eq!(
        records(&rt, "workspace_1"),
        vec![
            r#"{"events":[{"data":{"typing":true},"eventType":"typing","route":{"assignedTeamIds":[],"assignedUserIds":["user_1"]}},{"data":{"typing":true},"eventType":"typing","route":{"assignedTeamIds":[],"assignedUserIds":["user_1"]}}],"kind":"workspace-events","workspaceId":"workspace_1"}"#
                .to_owned()
        ]
    );
}

/// Port of: packages/business/__tests__/realtime-broadcast.test.ts:108 "realtime stream broadcast > rejects an invalid workspace event without delaying valid events"
#[tokio::test(start_paused = true)]
async fn realtime_stream_broadcast_rejects_an_invalid_workspace_event_without_delaying_valid_events()
 {
    let (rt, _fakes) = test_realtime(Arc::new(TestClock::new(NOW_MS)));
    let invalid_event = envelope(json!({
        "data": { "messageIds": ["message-1"] },
        "eventType": "messageDeleted",
    }));

    let err = rt
        .publish_workspace_event("workspace_1", invalid_event)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("Invalid workspace realtime event"),
        "{err}"
    );

    let valid_delivery = rt.publish_workspace_event("workspace_1", typing_event());
    tokio::time::advance(Duration::from_millis(25)).await;
    assert_eq!(valid_delivery.await, Ok(()));

    let records = records(&rt, "workspace_1");
    assert_eq!(records.len(), 1);
    assert!(records[0].contains(r#""events":[{"data":{"typing":true}"#));
}

/// Port of: packages/business/__tests__/realtime-broadcast.test.ts:152 "realtime stream broadcast > flushes pending workspace records before shutdown"
#[tokio::test(start_paused = true)]
async fn realtime_stream_broadcast_flushes_pending_workspace_records_before_shutdown() {
    let (rt, _fakes) = test_realtime(Arc::new(TestClock::new(NOW_MS)));
    let delivery = rt.publish_workspace_event("workspace_1", typing_event());

    rt.flush_all_pending().await;
    assert_eq!(delivery.await, Ok(()));

    assert_eq!(records(&rt, "workspace_1").len(), 1);
    assert_eq!(rt.pending_batches(), 0);
}

/// Port of: packages/business/__tests__/realtime-broadcast.test.ts:162 "realtime stream broadcast > splits workspace batches at 64 events"
#[tokio::test(start_paused = true)]
async fn realtime_stream_broadcast_splits_workspace_batches_at_64_events() {
    let (rt, _fakes) = test_realtime(Arc::new(TestClock::new(NOW_MS)));
    let deliveries: Vec<_> = (0..65)
        .map(|_| rt.publish_workspace_event("workspace_1", typing_event()))
        .collect();

    rt.flush_all_pending().await;
    let mut resolved = 0;
    for delivery in deliveries {
        assert_eq!(delivery.await, Ok(()));
        resolved += 1;
    }
    assert_eq!(resolved, 65);

    let batches: Vec<usize> = rt
        .hub()
        .buffered("workspace_1")
        .iter()
        .map(|entry| workspace_events(entry).len())
        .collect();
    assert_eq!(batches.len(), 2);
    assert_eq!(batches, vec![64, 1]);
}

/// Port of: packages/business/__tests__/realtime-broadcast.test.ts:180 "realtime stream broadcast > splits two valid sub-limit events when their exact record exceeds 256 KiB"
#[tokio::test(start_paused = true)]
async fn realtime_stream_broadcast_splits_two_valid_sub_limit_events_when_their_exact_record_exceeds_256_kib()
 {
    let (rt, _fakes) = test_realtime(Arc::new(TestClock::new(NOW_MS)));
    let workspace_id = "workspace_1";
    let message_event = envelope(json!({
        "data": { "text": "x".repeat(140 * 1024) },
        "eventType": "messageCreated",
        "route": { "assignedTeamIds": [], "assignedUserIds": [] },
    }));
    let first = rt.publish_workspace_event(workspace_id, message_event.clone());
    let second = rt.publish_workspace_event(workspace_id, message_event.clone());

    rt.flush_all_pending().await;
    assert_eq!(first.await, Ok(()));
    assert_eq!(second.await, Ok(()));

    let records = records(&rt, workspace_id);
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| record.len() <= 256 * 1024));
    let events: Vec<Vec<Envelope>> = rt
        .hub()
        .buffered(workspace_id)
        .iter()
        .map(|entry| workspace_events(entry))
        .collect();
    assert_eq!(
        events,
        vec![vec![message_event.clone()], vec![message_event]]
    );
}

/// Port of: packages/business/__tests__/realtime-broadcast.test.ts:209 "realtime stream broadcast > flushes immediate event types without waiting for the 25ms window"
#[tokio::test(start_paused = true)]
async fn realtime_stream_broadcast_flushes_immediate_event_types_without_waiting_for_the_25ms_window()
 {
    let (rt, _fakes) = test_realtime(Arc::new(TestClock::new(NOW_MS)));
    let started = tokio::time::Instant::now();

    let delivery = rt.publish_workspace_event(
        "workspace_1",
        envelope(json!({
            "data": {
                "assignedInboxTeamId": null,
                "assignedUserId": "user_1",
                "conversationIds": ["conversation_1"],
            },
            "eventType": "conversationAssigned",
            "route": { "assignedTeamIds": [], "assignedUserIds": ["user_1"] },
        })),
    );
    // Appended at call time, before any timer.
    assert_eq!(records(&rt, "workspace_1").len(), 1);

    assert_eq!(delivery.await, Ok(()));
    assert_eq!(tokio::time::Instant::now(), started);
    assert_eq!(records(&rt, "workspace_1").len(), 1);
}

/// Port of: packages/business/__tests__/realtime-broadcast.test.ts:256 "realtime stream broadcast > publishes directed member, revocation, and guest records directly"
#[tokio::test(start_paused = true)]
async fn realtime_stream_broadcast_publishes_directed_member_revocation_and_guest_records_directly()
{
    let (rt, fakes) = test_realtime(Arc::new(TestClock::new(NOW_MS)));
    let (live_socket, live_sink) = member_socket(
        chatbotx_rt::delivery::MemberSocketData {
            workspace_id: "workspace_1".to_owned(),
            user_id: "user_1".to_owned(),
            ..member_data()
        },
        SendMode::Ok,
    );
    rt.hub().delivery().add_workspace_socket(&live_socket);
    let closed_at_mark = Arc::new(AtomicBool::new(true));
    {
        let closed_at_mark = Arc::clone(&closed_at_mark);
        let live_socket = Arc::clone(&live_socket);
        fakes.revoke_store.set_on_raise(Arc::new(move || {
            closed_at_mark.store(live_socket.is_closed(), Ordering::SeqCst);
        }));
    }

    assert_eq!(
        rt.publish_member_event("workspace_1", "user_1", typing_event()),
        Ok(())
    );
    rt.revoke_member(revoke_args(None)).await.unwrap();
    assert_eq!(
        rt.publish_guest_event("workspace_1", "guest_1", typing_event()),
        Ok(())
    );

    // The marker is written before the live sockets are closed.
    assert!(!closed_at_mark.load(Ordering::SeqCst));
    assert_eq!(live_sink.closed(), Some((4001, "revoked".to_owned())));
    let raises = fakes.revoke_store.raises();
    assert_eq!(raises.len(), 1);
    assert_eq!(
        (raises[0].0.as_str(), raises[0].1.as_str()),
        ("workspace_1", "user_1")
    );
    let entries = rt.hub().buffered("workspace_1");
    assert_eq!(entries.len(), 2);
    match &entries[0].kind {
        EntryKind::MemberSend { user_id, event, .. } => {
            assert_eq!(user_id, "user_1");
            assert_eq!(event, &typing_event());
        }
        other => panic!("expected a member-send entry, got {other:?}"),
    }
    match &entries[1].kind {
        EntryKind::GuestEvent {
            guest_conversation_id,
            event,
            ..
        } => {
            assert_eq!(guest_conversation_id, "guest_1");
            assert_eq!(event, &typing_event());
        }
        other => panic!("expected a guest-event entry, got {other:?}"),
    }
}

/// Port of: packages/business/__tests__/realtime-broadcast.test.ts:305 "realtime stream broadcast > re-marks the revoked member before every retry of a failed revoke append"
#[tokio::test(start_paused = true)]
async fn realtime_stream_broadcast_re_marks_the_revoked_member_before_every_retry_of_a_failed_revoke_append()
 {
    let (rt, fakes) = test_realtime(Arc::new(TestClock::new(NOW_MS)));
    fakes.revoke_store.fail_raises(1);

    let revoke = rt.revoke_member(revoke_args(Some(1000))).await;

    assert!(revoke.is_ok());
    assert_eq!(
        fakes.revoke_store.raises(),
        vec![
            ("workspace_1".to_owned(), "user_1".to_owned(), 1000),
            ("workspace_1".to_owned(), "user_1".to_owned(), 1000),
        ]
    );
}

/// Port of: packages/business/__tests__/realtime-broadcast.test.ts:335 "realtime stream broadcast > throws after exhausting every revoke-append retry, so a failed member revoke is never silently swallowed"
#[tokio::test(start_paused = true)]
async fn realtime_stream_broadcast_throws_after_exhausting_every_revoke_append_retry_so_a_failed_member_revoke_is_never_silently_swallowed()
 {
    let (rt, fakes) = test_realtime(Arc::new(TestClock::new(NOW_MS)));
    fakes.revoke_store.fail_every_raise();

    let revoke = rt.revoke_member(revoke_args(None)).await;

    let err = revoke.unwrap_err();
    assert!(err.to_string().contains("Redis unavailable"), "{err}");
    assert_eq!(fakes.revoke_store.raises().len(), 3);
}

/// Port of: packages/business/__tests__/realtime-broadcast.test.ts:355 "realtime stream broadcast > enqueues a durable retry after immediate member revocation fails"
#[tokio::test(start_paused = true)]
async fn realtime_stream_broadcast_enqueues_a_durable_retry_after_immediate_member_revocation_fails()
 {
    let (rt, fakes) = test_realtime(Arc::new(TestClock::new(NOW_MS)));
    fakes.revoke_store.fail_every_raise();

    assert!(
        !rt.try_revoke_member(
            revoke_args(None),
            "Failed to revoke member realtime connections"
        )
        .await
    );

    let calls = fakes.revoke_retry.calls();
    assert_eq!(calls.len(), 1);
    let revoked_at_ms = calls[0].revoked_at_ms.expect("revokedAtMs set");
    assert_eq!(calls[0], revoke_args(Some(revoked_at_ms)));
    assert_eq!(
        revoke_retry_job_payload(&calls[0]),
        json!({
            "type": "retryRealtimeMemberRevoke",
            "data": {
                "reason": "deleted",
                "revokedAtMs": revoked_at_ms,
                "userId": "user_1",
                "workspaceId": "workspace_1",
            },
        })
    );
    assert_eq!(REVOKE_RETRY_JOB_NAME, "retryRealtimeMemberRevoke");
    assert_eq!(REVOKE_RETRY_JOB_ATTEMPTS, 8);
    assert_eq!(REVOKE_RETRY_JOB_DELAY_MS, 1000);
}
