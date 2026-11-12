//! Port of `apps/realtime/__tests__/stream-reader.test.ts` (P2.8 spec 4.6,
//! R199-R208). There is no Redis reader: producers call the hub directly
//! and records are validated at publish. R203/R204 move to the publish side
//! (adapted); R206 checks the one per-workspace buffer. R199-R202, R205,
//! R207 and R208 are not applicable (user decision 2026-11-12, DECISIONS
//! P2.8).

mod support;

use std::sync::Arc;

use chatbotx_rt::TestClock;
use chatbotx_rt::delivery::{Counters, Delivery};
use chatbotx_rt::hub::{EntryKind, Hub, HubConfig};
use chatbotx_rt::protocol::{Envelope, parse_workspace_envelope};
use serde_json::{Value, json};
use support::test_realtime;

fn envelope(event: Value) -> Envelope {
    serde_json::from_value(event).unwrap()
}

fn workspace_events(entry: &chatbotx_rt::hub::Entry) -> Vec<Envelope> {
    match &entry.kind {
        EntryKind::WorkspaceEvents { events, .. } => events.clone(),
        other => panic!("not a workspace-events entry: {other:?}"),
    }
}

/// Port of: apps/realtime/__tests__/stream-reader.test.ts:253 "stream reader > drops routeless and targeted-only events while preserving valid workspace events"
#[tokio::test(start_paused = true)]
async fn stream_reader_drops_routeless_and_targeted_only_events_while_preserving_valid_workspace_events()
 {
    let (rt, _fakes) = test_realtime(Arc::new(TestClock::new(1_000)));

    let deliveries = [
        rt.publish_workspace_event(
            "workspace-1",
            envelope(json!({ "data": { "typing": true }, "eventType": "typing" })),
        ),
        rt.publish_workspace_event(
            "workspace-1",
            envelope(json!({ "data": { "id": "missing-route" }, "eventType": "messageCreated" })),
        ),
        rt.publish_workspace_event(
            "workspace-1",
            envelope(json!({
                "data": { "callId": "call-1" },
                "eventType": "whatsappCallTransportIncoming",
            })),
        ),
    ];
    let mut results = Vec::new();
    for delivery in deliveries {
        results.push(delivery.await);
    }
    rt.flush_all_pending().await;

    let dropped_count = results.iter().filter(|result| result.is_err()).count();
    assert_eq!(dropped_count, 2);
    assert!(results[0].is_ok());
    let entries = rt.hub().buffered("workspace-1");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        workspace_events(&entries[0]),
        vec![envelope(
            json!({ "data": { "typing": true }, "eventType": "typing" })
        )]
    );
}

/// Port of: apps/realtime/__tests__/stream-reader.test.ts:281 "stream reader > drops only the invalid envelope within a coalesced workspace-events record, keeping the valid ones"
#[tokio::test(start_paused = true)]
async fn stream_reader_drops_only_the_invalid_envelope_within_a_coalesced_workspace_events_record_keeping_the_valid_ones()
 {
    let (rt, _fakes) = test_realtime(Arc::new(TestClock::new(1_000)));
    let raw_events = [
        json!({ "data": { "id": "ok" }, "eventType": "typing" }),
        json!({ "data": {}, "eventType": 123 }),
    ];

    let mut invalid = 0;
    let mut deliveries = Vec::new();
    for raw in &raw_events {
        match parse_workspace_envelope(raw) {
            Ok(event) => deliveries.push(rt.publish_workspace_event("workspace-1", event)),
            Err(_) => invalid += 1,
        }
    }
    tokio::time::advance(std::time::Duration::from_millis(25)).await;
    for delivery in deliveries {
        delivery.await.unwrap();
    }

    assert_eq!(invalid, 1);
    let entries = rt.hub().buffered("workspace-1");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        workspace_events(&entries[0]),
        vec![envelope(
            json!({ "data": { "id": "ok" }, "eventType": "typing" })
        )]
    );
}

/// Port of: apps/realtime/__tests__/stream-reader.test.ts:451 "stream reader > does not report a recent-entry gap when every evicted entry was already seen"
#[test]
fn stream_reader_does_not_report_a_recent_entry_gap_when_every_evicted_entry_was_already_seen() {
    let clock = Arc::new(TestClock::new(1));
    let hub = Hub::new(
        clock.clone(),
        HubConfig::default(),
        Delivery::new(Arc::new(Counters::default())),
    );
    for ms in 1..=513 {
        clock.set(ms);
        hub.append_workspace_events(
            "workspace-1",
            vec![envelope(json!({ "data": {}, "eventType": "typing" }))],
        );
    }

    // 1-0 was evicted (512 kept) and the client already saw it.
    assert_eq!(hub.buffered("workspace-1").len(), 512);
    assert!(!hub.has_gap("workspace-1", "1-0"));
}
