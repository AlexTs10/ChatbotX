//! Port of `apps/realtime/__tests__/replay.test.ts` (P2.8 spec 4.6,
//! R184-R193). The fake `xrange`/`xrevrange` responses become a real
//! per-workspace hub buffer; a `TestClock` sets the entry ids (`<ms>-0`).
//! R189-R191 (malformed entries) and R193 (noisy shared shard) are not
//! applicable (user decision 2026-11-12, DECISIONS P2.8).

use std::sync::Arc;

use chatbotx_rt::TestClock;
use chatbotx_rt::delivery::{Counters, Delivery};
use chatbotx_rt::hub::{Hub, HubConfig};
use chatbotx_rt::protocol::Envelope;
use serde_json::json;

fn hub(clock: &Arc<TestClock>) -> Arc<Hub> {
    Hub::new(
        clock.clone(),
        HubConfig::default(),
        Delivery::new(Arc::new(Counters::default())),
    )
}

fn typing(n: u64) -> Vec<Envelope> {
    vec![
        serde_json::from_value(json!({ "data": { "id": n.to_string() }, "eventType": "typing" }))
            .unwrap(),
    ]
}

/// One `workspace-1` entry with id `<ms>-0`.
fn append_at(hub: &Hub, clock: &TestClock, ms: i64) {
    clock.set(ms);
    let seq = hub.append_workspace_events("workspace-1", typing(ms as u64));
    assert_eq!(seq.to_string(), format!("{ms}-0"));
}

fn requires_resync(entries_at: &[i64], last_seq: &str, expected_close_reason: &str) {
    let clock = Arc::new(TestClock::new(1));
    let hub = hub(&clock);
    for ms in entries_at {
        append_at(&hub, &clock, *ms);
    }

    let replay = hub.load_replay("workspace-1", Some(last_seq));

    assert_eq!(replay.close_reason, Some(expected_close_reason));
    assert!(replay.entries.is_empty());
}

/// Port of: apps/realtime/__tests__/replay.test.ts:5 "loadReplay > requires resync for a cursor ahead of the retained stream tail"
#[test]
fn loadreplay_requires_resync_for_a_cursor_ahead_of_the_retained_stream_tail() {
    requires_resync(&[10], "11-0", "replay-cursor-ahead");
}

/// Port of: apps/realtime/__tests__/replay.test.ts:5 "loadReplay > requires resync for a nonzero cursor after the stream was reset"
#[test]
fn loadreplay_requires_resync_for_a_nonzero_cursor_after_the_stream_was_reset() {
    // The restart case: the buffer is empty.
    requires_resync(&[], "10-0", "replay-cursor-ahead");
}

/// Port of: apps/realtime/__tests__/replay.test.ts:5 "loadReplay > requires resync for trimming that advances the replay head"
#[test]
fn loadreplay_requires_resync_for_trimming_that_advances_the_replay_head() {
    // Adapted: the snapshot is taken under one lock, so the head cannot move
    // during the scan; the retained head is already after the cursor.
    requires_resync(&[10], "5-0", "replay-window-expired");
}

/// Port of: apps/realtime/__tests__/replay.test.ts:5 "loadReplay > requires resync for the cursor-less sentinel against retained history"
#[test]
fn loadreplay_requires_resync_for_the_cursor_less_sentinel_against_retained_history() {
    requires_resync(&[5], "0-0", "replay-window-expired");
}

/// Port of: apps/realtime/__tests__/replay.test.ts:63 "loadReplay > does not resync the cursor-less sentinel against a genuinely empty stream"
#[test]
fn loadreplay_does_not_resync_the_cursor_less_sentinel_against_a_genuinely_empty_stream() {
    let clock = Arc::new(TestClock::new(1));
    let hub = hub(&clock);

    let replay = hub.load_replay("workspace-1", Some("0-0"));

    assert_eq!(replay.close_reason, None);
    assert!(replay.entries.is_empty());
}

/// Port of: apps/realtime/__tests__/replay.test.ts:182 "loadReplay > closes the replay window once this workspace's OWN entries exceed MAX_REPLAY_ENTRIES (500), even within a single page"
#[test]
fn loadreplay_closes_the_replay_window_once_this_workspace_s_own_entries_exceed_max_replay_entries_500_even_within_a_single_page()
 {
    // Entry 1-0 (the oldest-entry check) plus 501 own entries 2-0..502-0.
    let clock = Arc::new(TestClock::new(1));
    let hub = hub(&clock);
    for ms in 1..=502 {
        append_at(&hub, &clock, ms);
    }

    let replay = hub.load_replay("workspace-1", Some("1-0"));

    assert_eq!(replay.close_reason, Some("replay-window-too-large"));
    assert!(replay.entries.is_empty());
}
