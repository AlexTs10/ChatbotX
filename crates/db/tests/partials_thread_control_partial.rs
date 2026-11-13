//! Port of `packages/database/__tests__/thread-control-partial.test.ts` (P3.2b, ported:
//! times are unix ms, states / events / roles their stored texts; `Date` equality is ms
//! equality; the TS `undefined` and `null` expiry are both `None`; each `test.each` row is
//! one test. Adapted: "covers every event exactly once" checks the
//! `THREAD_CONTROL_TRANSITIONS` table and the `transition` lookup).

use chatbotx_db::repo::partials::thread_control::{
    THREAD_CONTROL_EVENT_PRECEDENCE, THREAD_CONTROL_EVENTS, THREAD_CONTROL_INBOX_ACTIVE_MS,
    THREAD_CONTROL_TRANSITIONS, THREAD_IDLE_AFTER_MS, ThreadControlColumns, ThreadControlInput,
    events_outranked_by, is_inbox_thread_control_active, is_service_send_blocked,
    parse_thread_control_role, read_thread_control_columns, resolve_thread_control_state,
    to_thread_control_timestamp, transition,
};
use serde_json::{Value, json};

/// `2026-09-29T12:00:00.000Z`.
const NOW: i64 = 1_790_683_200_000;
const NOW_ISO: &str = "2026-09-29T12:00:00.000Z";

fn ago(ms: i64) -> i64 {
    NOW - ms
}

fn input(
    state: Option<&str>,
    last_incoming: Option<i64>,
    updated: Option<i64>,
) -> ThreadControlInput<'_> {
    ThreadControlInput {
        state,
        last_incoming_message_at: last_incoming,
        thread_control_updated_at: updated,
        thread_owner_expires_at: None,
        now: NOW,
    }
}

fn resolve(input: ThreadControlInput<'_>) -> Option<&str> {
    resolve_thread_control_state(&input)
}

fn sorted(mut list: Vec<&str>) -> Vec<&str> {
    list.sort_unstable();
    list
}

fn rank(event: &str) -> usize {
    THREAD_CONTROL_EVENT_PRECEDENCE
        .iter()
        .position(|e| *e == event)
        .expect("ranked event")
}

fn assert_transition(event: &str, state: &str) {
    assert_eq!(transition(event), Some(state));
}

#[test]
fn thread_control_transitions_inbound_received_resolves_to_owned() {
    assert_transition("inboundReceived", "owned");
}

#[test]
fn thread_control_transitions_control_passed_resolves_to_owned() {
    assert_transition("controlPassed", "owned");
}

#[test]
fn thread_control_transitions_taken_resolves_to_owned() {
    assert_transition("taken", "owned");
}

#[test]
fn thread_control_transitions_service_sent_resolves_to_owned() {
    assert_transition("serviceSent", "owned");
}

#[test]
fn thread_control_transitions_standby_received_resolves_to_standby() {
    assert_transition("standbyReceived", "standby");
}

#[test]
fn thread_control_transitions_control_taken_resolves_to_standby() {
    assert_transition("controlTaken", "standby");
}

#[test]
fn thread_control_transitions_passed_resolves_to_standby() {
    assert_transition("passed", "standby");
}

#[test]
fn thread_control_transitions_service_rejected_resolves_to_standby() {
    assert_transition("serviceRejected", "standby");
}

#[test]
fn thread_control_transitions_released_resolves_to_idle() {
    assert_transition("released", "idle");
}

#[test]
fn thread_control_transitions_covers_every_event_exactly_once() {
    let keys: Vec<&str> = THREAD_CONTROL_TRANSITIONS.iter().map(|(e, _)| *e).collect();
    assert_eq!(sorted(keys), sorted(THREAD_CONTROL_EVENTS.to_vec()));
    for (event, state) in THREAD_CONTROL_TRANSITIONS {
        assert_eq!(transition(event), Some(*state), "{event}");
    }
}

#[test]
fn thread_control_event_precedence_is_a_total_order_over_every_event_with_no_duplicates() {
    assert_eq!(
        sorted(THREAD_CONTROL_EVENT_PRECEDENCE.to_vec()),
        sorted(THREAD_CONTROL_EVENTS.to_vec())
    );
    let unique: std::collections::HashSet<&&str> = THREAD_CONTROL_EVENT_PRECEDENCE.iter().collect();
    assert_eq!(unique.len(), THREAD_CONTROL_EVENT_PRECEDENCE.len());
}

#[test]
fn thread_control_event_precedence_meta_explicit_handovers_outrank_our_calls_which_outrank_inferred_states()
 {
    assert!(rank("controlTaken") > rank("taken"));
    assert!(rank("controlPassed") > rank("taken"));
    assert!(rank("taken") > rank("serviceSent"));
    assert!(rank("passed") > rank("inboundReceived"));
}

#[test]
fn events_outranked_by_the_lowest_precedence_event_outranks_nothing() {
    assert!(events_outranked_by("inboundReceived").is_empty());
}

#[test]
fn events_outranked_by_the_highest_precedence_event_outranks_every_other_event() {
    let others: Vec<&str> = THREAD_CONTROL_EVENTS
        .iter()
        .copied()
        .filter(|e| *e != "controlTaken")
        .collect();
    assert_eq!(
        sorted(events_outranked_by("controlTaken").to_vec()),
        sorted(others)
    );
}

#[test]
fn events_outranked_by_never_includes_the_event_itself_and_only_strictly_lower_events() {
    for event in THREAD_CONTROL_EVENTS {
        let outranked = events_outranked_by(event);
        assert!(!outranked.contains(event), "{event}");
        for loser in outranked {
            assert!(rank(loser) < rank(event), "{event} {loser}");
        }
    }
}

#[test]
fn events_outranked_by_two_distinct_events_can_never_both_outrank_each_other() {
    for a in THREAD_CONTROL_EVENTS {
        for b in THREAD_CONTROL_EVENTS {
            if a == b {
                continue;
            }
            let a_beats_b = events_outranked_by(a).contains(b);
            let b_beats_a = events_outranked_by(b).contains(a);
            assert!(a_beats_b != b_beats_a, "{a} {b}");
        }
    }
}

#[test]
fn resolve_thread_control_state_null_state_means_routing_was_never_observed() {
    assert_eq!(resolve(input(None, Some(ago(1000)), None)), None);
}

#[test]
fn resolve_thread_control_state_owned_within_24h_stays_owned() {
    assert_eq!(
        resolve(input(
            Some("owned"),
            Some(ago(THREAD_IDLE_AFTER_MS - 1)),
            None
        )),
        Some("owned")
    );
}

#[test]
fn resolve_thread_control_state_standby_within_24h_stays_standby() {
    assert_eq!(
        resolve(input(
            Some("standby"),
            Some(ago(THREAD_IDLE_AFTER_MS - 1)),
            None
        )),
        Some("standby")
    );
}

#[test]
fn resolve_thread_control_state_owned_at_exactly_24h_is_idle() {
    assert_eq!(
        resolve(input(Some("owned"), Some(ago(THREAD_IDLE_AFTER_MS)), None)),
        Some("idle")
    );
}

#[test]
fn resolve_thread_control_state_standby_at_exactly_24h_is_idle() {
    assert_eq!(
        resolve(input(
            Some("standby"),
            Some(ago(THREAD_IDLE_AFTER_MS)),
            None
        )),
        Some("idle")
    );
}

#[test]
fn resolve_thread_control_state_a_stored_state_older_than_24h_of_user_silence_is_idle() {
    assert_eq!(
        resolve(input(
            Some("standby"),
            Some(ago(THREAD_IDLE_AFTER_MS + 60_000)),
            None
        )),
        Some("idle")
    );
}

#[test]
fn resolve_thread_control_state_idle_stays_idle() {
    assert_eq!(
        resolve(input(Some("idle"), Some(ago(1000)), None)),
        Some("idle")
    );
}

#[test]
fn resolve_thread_control_state_without_any_reference_timestamp_the_stored_state_is_kept() {
    assert_eq!(resolve(input(Some("owned"), None, None)), Some("owned"));
}

fn old_incoming() -> i64 {
    ago(3 * THREAD_IDLE_AFTER_MS)
}

fn assert_stays_until_updated_at_plus_24h(state: &str) {
    let at = |updated_ago: i64| {
        resolve(input(
            Some(state),
            Some(old_incoming()),
            Some(ago(updated_ago)),
        ))
    };
    assert_eq!(at(0), Some(state));
    assert_eq!(at(THREAD_IDLE_AFTER_MS - 1), Some(state));
    assert_eq!(at(THREAD_IDLE_AFTER_MS), Some("idle"));
}

#[test]
fn resolve_thread_control_state_measured_from_the_last_transition_standby_after_control_taken_stays_until_updated_at_24h_despite_an_old_user_message()
 {
    assert_stays_until_updated_at_plus_24h("standby");
}

#[test]
fn resolve_thread_control_state_measured_from_the_last_transition_owned_after_control_passed_stays_until_updated_at_24h_despite_an_old_user_message()
 {
    assert_stays_until_updated_at_plus_24h("owned");
}

#[test]
fn resolve_thread_control_state_measured_from_the_last_transition_a_newer_user_message_extends_the_window_past_the_transition()
 {
    assert_eq!(
        resolve(input(
            Some("standby"),
            Some(ago(1000)),
            Some(ago(2 * THREAD_IDLE_AFTER_MS))
        )),
        Some("standby")
    );
}

#[test]
fn resolve_thread_control_state_measured_from_the_last_transition_an_old_user_message_without_a_transition_time_is_idle_both_null_keeps_state()
 {
    assert_eq!(
        resolve(input(Some("owned"), Some(old_incoming()), None)),
        Some("idle")
    );
    assert_eq!(resolve(input(Some("owned"), None, None)), Some("owned"));
}

#[test]
fn resolve_thread_control_state_measured_from_the_last_transition_the_send_gate_stays_closed_for_a_fresh_control_taken_on_an_old_thread()
 {
    assert!(is_service_send_blocked(
        &input(Some("standby"), Some(old_incoming()), Some(ago(1000))),
        false
    ));
}

fn send_base(state: Option<&str>) -> ThreadControlInput<'_> {
    input(state, Some(ago(1000)), None)
}

#[test]
fn is_service_send_blocked_blocks_a_service_send_while_another_responder_holds_the_thread() {
    assert!(is_service_send_blocked(&send_base(Some("standby")), false));
}

#[test]
fn is_service_send_blocked_never_blocks_a_template_even_on_a_standby_thread() {
    assert!(!is_service_send_blocked(&send_base(Some("standby")), true));
}

#[test]
fn is_service_send_blocked_does_not_block_on_owned() {
    assert!(!is_service_send_blocked(&send_base(Some("owned")), false));
}

#[test]
fn is_service_send_blocked_does_not_block_on_idle() {
    assert!(!is_service_send_blocked(&send_base(Some("idle")), false));
}

#[test]
fn is_service_send_blocked_does_not_block_on_null() {
    assert!(!is_service_send_blocked(&send_base(None), false));
}

#[test]
fn is_service_send_blocked_does_not_block_once_the_standby_thread_aged_past_24h() {
    assert!(!is_service_send_blocked(
        &input(Some("standby"), Some(ago(THREAD_IDLE_AFTER_MS)), None),
        false
    ));
}

#[test]
fn is_inbox_thread_control_active_null_means_never_seen() {
    assert!(!is_inbox_thread_control_active(None, NOW));
}

#[test]
fn is_inbox_thread_control_active_active_within_30_days() {
    assert!(is_inbox_thread_control_active(
        Some(ago(THREAD_CONTROL_INBOX_ACTIVE_MS - 1)),
        NOW
    ));
}

#[test]
fn is_inbox_thread_control_active_decays_at_exactly_30_days() {
    assert!(!is_inbox_thread_control_active(
        Some(ago(THREAD_CONTROL_INBOX_ACTIVE_MS)),
        NOW
    ));
}

#[test]
fn parse_thread_control_role_accepts_a_known_role() {
    assert_eq!(
        parse_thread_control_role(Some(&json!("ai_agent"))),
        Some("ai_agent")
    );
}

#[test]
fn parse_thread_control_role_stores_future_role_as_null_forward_compatible() {
    assert_eq!(parse_thread_control_role(Some(&json!("future_role"))), None);
}

#[test]
fn parse_thread_control_role_stores_empty_as_null_forward_compatible() {
    assert_eq!(parse_thread_control_role(Some(&json!(""))), None);
}

#[test]
fn parse_thread_control_role_stores_null_as_null_forward_compatible() {
    assert_eq!(parse_thread_control_role(Some(&Value::Null)), None);
}

#[test]
fn parse_thread_control_role_stores_undefined_as_null_forward_compatible() {
    assert_eq!(parse_thread_control_role(None), None);
}

#[test]
fn parse_thread_control_role_stores_42_as_null_forward_compatible() {
    assert_eq!(parse_thread_control_role(Some(&json!(42))), None);
}

#[test]
fn to_thread_control_timestamp_truncates_our_own_event_time_to_meta_s_whole_second_resolution() {
    assert_eq!(to_thread_control_timestamp(NOW + 999), NOW);
    assert_eq!(to_thread_control_timestamp(NOW), NOW);
}

#[test]
fn read_thread_control_columns_reads_a_db_row_as_is() {
    assert_eq!(
        read_thread_control_columns(&json!({
            "threadControlState": "owned",
            "lastIncomingMessageAt": NOW,
            "threadControlUpdatedAt": null,
        })),
        ThreadControlColumns {
            state: Some("owned".to_owned()),
            last_incoming_message_at: Some(NOW),
            thread_control_updated_at: None,
            thread_owner_expires_at: None,
        }
    );
}

#[test]
fn read_thread_control_columns_revives_iso_strings_from_a_job_payload() {
    assert_eq!(
        read_thread_control_columns(&json!({
            "threadControlState": "standby",
            "lastIncomingMessageAt": NOW_ISO,
            "threadControlUpdatedAt": NOW_ISO,
        })),
        ThreadControlColumns {
            state: Some("standby".to_owned()),
            last_incoming_message_at: Some(NOW),
            thread_control_updated_at: Some(NOW),
            thread_owner_expires_at: None,
        }
    );
}

#[test]
fn read_thread_control_columns_a_row_that_predates_the_routing_columns_reads_as_never_observed() {
    assert_eq!(
        read_thread_control_columns(&json!({})),
        ThreadControlColumns::default()
    );
}

#[test]
fn read_thread_control_columns_reads_and_revives_the_channel_expiry() {
    assert_eq!(
        read_thread_control_columns(&json!({ "threadOwnerExpiresAt": NOW }))
            .thread_owner_expires_at,
        Some(NOW)
    );
    assert_eq!(
        read_thread_control_columns(&json!({ "threadOwnerExpiresAt": NOW_ISO }))
            .thread_owner_expires_at,
        Some(NOW)
    );
}

fn expiry_base(state: Option<&str>, expiry: Option<i64>) -> ThreadControlInput<'_> {
    ThreadControlInput {
        thread_owner_expires_at: expiry,
        ..input(state, Some(ago(1000)), Some(ago(1000)))
    }
}

fn assert_no_expiry_regression(state: &str) {
    // The TS `undefined` and `null` expiry are both `None`: one pass.
    assert_eq!(resolve(expiry_base(Some(state), None)), Some(state));
    assert_eq!(
        resolve(input(Some(state), Some(ago(THREAD_IDLE_AFTER_MS)), None)),
        Some("idle")
    );
    assert_eq!(
        resolve(input(
            Some(state),
            Some(ago(THREAD_IDLE_AFTER_MS - 1)),
            None
        )),
        Some(state)
    );
}

#[test]
fn resolve_thread_control_state_channel_expiry_whatsapp_no_expiry_regression_owned_resolves_exactly_as_before_for_undefined_and_null_expiry()
 {
    assert_no_expiry_regression("owned");
}

#[test]
fn resolve_thread_control_state_channel_expiry_whatsapp_no_expiry_regression_standby_resolves_exactly_as_before_for_undefined_and_null_expiry()
 {
    assert_no_expiry_regression("standby");
}

#[test]
fn resolve_thread_control_state_channel_expiry_keeps_the_state_before_the_expiry_even_when_activity_is_older_than_24h()
 {
    assert_eq!(
        resolve(ThreadControlInput {
            thread_owner_expires_at: Some(NOW + 1),
            ..input(
                Some("standby"),
                Some(ago(THREAD_IDLE_AFTER_MS * 3)),
                Some(ago(THREAD_IDLE_AFTER_MS * 3))
            )
        }),
        Some("standby")
    );
}

#[test]
fn resolve_thread_control_state_channel_expiry_is_idle_at_and_after_the_expiry_even_when_activity_is_recent()
 {
    for offset in [0, 1, 60_000] {
        assert_eq!(
            resolve(expiry_base(Some("standby"), Some(NOW - offset))),
            Some("idle"),
            "{offset}"
        );
    }
}

#[test]
fn resolve_thread_control_state_channel_expiry_a_null_or_idle_state_is_returned_as_is_regardless_of_the_expiry()
 {
    assert_eq!(resolve(expiry_base(None, Some(ago(1)))), None);
    assert_eq!(
        resolve(expiry_base(Some("idle"), Some(NOW + 1000))),
        Some("idle")
    );
}

#[test]
fn resolve_thread_control_state_channel_expiry_is_service_send_blocked_honors_the_expiry() {
    assert!(is_service_send_blocked(
        &expiry_base(Some("standby"), Some(NOW + 1000)),
        false
    ));
    assert!(!is_service_send_blocked(
        &expiry_base(Some("standby"), Some(ago(0))),
        false
    ));
}
