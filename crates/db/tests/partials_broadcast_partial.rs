//! Port of `packages/database/__tests__/broadcast-partial.test.ts` (P3.2b, ported: the
//! broadcast-shaped inputs are JSON objects; `null` / `undefined` are `None`, and where the
//! TS tells them apart (`submitted`) `undefined` is `None` and `null` is `Some(None)`;
//! `toMatchObject` checks the named fields; times are unix ms).

use chatbotx_db::repo::partials::broadcast::{
    BROADCAST_AUDIENCE_POSITION_MIN, BROADCAST_CHANNEL_CAPABILITIES,
    BROADCAST_DEFAULT_SEND_RATE_PER_MINUTE, BROADCAST_DISPATCH_WINDOW_MS,
    BROADCAST_MAX_SEND_RATE_PER_MINUTE, BROADCAST_OCCURRENCE_OUTCOMES,
    BROADCAST_OCCURRENCE_REASONS, BROADCAST_OUTCOME_GRACE_MS, BROADCAST_SCHEDULE_TYPES,
    BROADCAST_SEND_LIMIT_ISSUE_RANGE_END_BEFORE_START, BROADCAST_STATUSES,
    BROADCAST_SUBACTION_AUDIENCE_RULES, BROADCAST_SUBACTIONS, BroadcastAudienceRange,
    BroadcastSendLimit, CappedCount, PageWindow, clamp_audience_count_to_range,
    clamp_capped_audience_count, find_broadcast_channel_capability, is_audience_range_ordered,
    is_broadcast_outcome_grace_elapsed, is_recurrence_run, is_recurring_template,
    is_targets_flow_send_without_flow, is_targets_template_send_without_template,
    normalize_broadcast_send_limit, parse_broadcast_send_limit, requires_recent_interaction_window,
    resolve_activation_send_rate, resolve_audience_page_window, resolve_broadcast_audience_range,
    resolve_broadcast_send_rate_per_minute, resolve_broadcast_terminal_status,
    resolve_submitted_send_rate_patch,
};
use serde_json::json;

fn range(offset: i64, size: Option<i64>) -> Option<BroadcastAudienceRange> {
    Some(BroadcastAudienceRange { offset, size })
}

#[test]
fn requires_recent_interaction_window_requires_the_24h_messaging_window_for_non_template_messenger_and_whatsapp_broadcast_subactions()
 {
    assert!(requires_recent_interaction_window(Some(
        "messengerActiveContacts"
    )));
    assert!(requires_recent_interaction_window(Some(
        "whatsappWithin24Hours"
    )));
    assert!(requires_recent_interaction_window(Some(
        "instagramActiveContacts"
    )));
    assert!(requires_recent_interaction_window(Some(
        "tiktokActiveContacts"
    )));
}

#[test]
fn requires_recent_interaction_window_does_not_require_the_24h_messaging_window_for_templates_all_contacts_telegram_or_unset_subactions()
 {
    assert!(!requires_recent_interaction_window(Some("allContacts")));
    assert!(!requires_recent_interaction_window(Some(
        "messengerTemplateMessage"
    )));
    assert!(!requires_recent_interaction_window(Some(
        "whatsappTemplateMessage"
    )));
    assert!(!requires_recent_interaction_window(Some(
        "telegramAllContacts"
    )));
    // TS `null` and `undefined` are both `None`.
    assert!(!requires_recent_interaction_window(None));
}

#[test]
fn requires_recent_interaction_window_declares_an_audience_rule_for_every_broadcast_subaction() {
    let mut keys: Vec<&str> = BROADCAST_SUBACTION_AUDIENCE_RULES
        .iter()
        .map(|(s, _)| *s)
        .collect();
    keys.sort_unstable();
    let mut subactions = BROADCAST_SUBACTIONS.to_vec();
    subactions.sort_unstable();
    assert_eq!(keys, subactions);
}

#[test]
fn broadcast_channel_capabilities_declares_default_subactions_that_belong_to_each_channel_capability()
 {
    for capability in BROADCAST_CHANNEL_CAPABILITIES {
        assert!(
            capability
                .subactions
                .contains(&capability.default_subaction),
            "{}",
            capability.channel
        );
    }
}

#[test]
fn broadcast_channel_capabilities_finds_instagram_telegram_and_tiktok_capabilities_and_excludes_non_broadcast_channels()
 {
    for (channel, default_subaction) in [
        ("instagram", "instagramActiveContacts"),
        ("telegram", "telegramAllContacts"),
        ("tiktok", "tiktokActiveContacts"),
    ] {
        let capability = find_broadcast_channel_capability(channel).expect(channel);
        assert_eq!(capability.channel, channel);
        assert_eq!(capability.default_subaction, default_subaction);
    }
    assert!(find_broadcast_channel_capability("webchat").is_none());
}

#[test]
fn broadcast_channel_capabilities_marks_only_messenger_and_whatsapp_as_supporting_template_broadcasts()
 {
    let mut template_channels: Vec<&str> = BROADCAST_CHANNEL_CAPABILITIES
        .iter()
        .filter(|c| c.supports_template_broadcast)
        .map(|c| c.channel)
        .collect();
    template_channels.sort_unstable();
    assert_eq!(template_channels, ["messenger", "whatsapp"]);
    for channel in ["instagram", "telegram", "tiktok"] {
        assert_eq!(
            find_broadcast_channel_capability(channel).map(|c| c.supports_template_broadcast),
            Some(false),
            "{channel}"
        );
    }
}

#[test]
fn broadcast_statuses_includes_draft_and_failed() {
    assert!(BROADCAST_STATUSES.contains(&"draft"));
    assert!(BROADCAST_STATUSES.contains(&"failed"));
}

#[test]
fn resolve_broadcast_terminal_status_is_sent_when_no_contact_failed() {
    assert_eq!(resolve_broadcast_terminal_status(Some(10), 0), "sent");
}

#[test]
fn resolve_broadcast_terminal_status_is_sent_when_only_some_contacts_failed() {
    assert_eq!(resolve_broadcast_terminal_status(Some(10), 9), "sent");
}

#[test]
fn resolve_broadcast_terminal_status_is_failed_when_every_contact_failed() {
    assert_eq!(resolve_broadcast_terminal_status(Some(10), 10), "failed");
}

#[test]
fn resolve_broadcast_terminal_status_is_sent_when_contact_count_is_null_or_zero() {
    assert_eq!(resolve_broadcast_terminal_status(None, 3), "sent");
    assert_eq!(resolve_broadcast_terminal_status(Some(0), 0), "sent");
}

/// `2026-08-31T10:00:00Z`.
const GRACE_NOW: i64 = 1_788_170_400_000;

#[test]
fn is_broadcast_outcome_grace_elapsed_is_false_inside_the_grace_window() {
    assert!(!is_broadcast_outcome_grace_elapsed(
        GRACE_NOW - BROADCAST_OUTCOME_GRACE_MS + 1,
        GRACE_NOW
    ));
}

#[test]
fn is_broadcast_outcome_grace_elapsed_is_true_at_or_past_the_grace_window() {
    assert!(is_broadcast_outcome_grace_elapsed(
        GRACE_NOW - BROADCAST_OUTCOME_GRACE_MS,
        GRACE_NOW
    ));
}

#[test]
fn is_targets_template_send_without_template_is_true_for_a_legacy_top_level_template_id_with_only_empty_targets_the_edge_this_predicate_closes()
 {
    assert!(is_targets_template_send_without_template(&json!({
        "templateId": "legacy-template",
        "targets": [{ "inboxId": "inbox-a" }, { "inboxId": "inbox-b" }],
    })));
}

#[test]
fn is_targets_template_send_without_template_is_true_when_no_target_carries_a_template_and_there_is_no_legacy_template_id_either()
 {
    assert!(is_targets_template_send_without_template(&json!({
        "targets": [{ "inboxId": "inbox-a" }, { "inboxId": "inbox-b" }],
    })));
}

#[test]
fn is_targets_template_send_without_template_is_false_when_at_least_one_target_carries_a_template()
{
    assert!(!is_targets_template_send_without_template(&json!({
        "targets": [{ "inboxId": "inbox-a" }, { "inboxId": "inbox-b", "templateId": "template-b" }],
    })));
}

#[test]
fn is_targets_template_send_without_template_is_false_for_a_flow_send_even_when_no_target_has_a_template()
 {
    assert!(!is_targets_template_send_without_template(&json!({
        "targets": [{ "inboxId": "inbox-a", "flowId": "flow-a" }, { "inboxId": "inbox-b" }],
    })));
}

#[test]
fn is_targets_template_send_without_template_is_false_for_a_legacy_channel_mode_payload_without_any_targets()
 {
    assert!(!is_targets_template_send_without_template(&json!({
        "templateId": "legacy-template",
        "targets": [],
    })));
    assert!(!is_targets_template_send_without_template(&json!({})));
}

#[test]
fn is_targets_template_send_without_template_is_false_once_a_persisted_row_pins_target_mode_to_channel_even_with_empty_target_rows()
 {
    assert!(!is_targets_template_send_without_template(&json!({
        "targetMode": "channel",
        "targets": [{ "inboxId": "inbox-a" }],
    })));
}

#[test]
fn is_targets_flow_send_without_flow_is_true_for_a_legacy_top_level_flow_id_with_only_empty_targets_the_edge_this_predicate_closes()
 {
    assert!(is_targets_flow_send_without_flow(&json!({
        "flowId": "legacy-flow",
        "targets": [{ "inboxId": "inbox-a" }, { "inboxId": "inbox-b" }],
    })));
}

#[test]
fn is_targets_flow_send_without_flow_is_true_when_no_target_carries_a_flow_and_there_is_no_legacy_flow_id_either()
 {
    assert!(is_targets_flow_send_without_flow(&json!({
        "targets": [{ "inboxId": "inbox-a" }, { "inboxId": "inbox-b" }],
    })));
}

#[test]
fn is_targets_flow_send_without_flow_is_false_when_at_least_one_target_carries_a_flow() {
    assert!(!is_targets_flow_send_without_flow(&json!({
        "targets": [{ "inboxId": "inbox-a" }, { "inboxId": "inbox-b", "flowId": "flow-b" }],
    })));
}

#[test]
fn is_targets_flow_send_without_flow_is_false_for_a_template_send_even_when_no_target_has_a_flow() {
    assert!(!is_targets_flow_send_without_flow(&json!({
        "targets": [{ "inboxId": "inbox-a", "templateId": "template-a" }, { "inboxId": "inbox-b" }],
    })));
}

#[test]
fn is_targets_flow_send_without_flow_is_false_for_a_legacy_channel_mode_payload_without_any_targets()
 {
    assert!(!is_targets_flow_send_without_flow(&json!({
        "flowId": "legacy-flow",
        "targets": [],
    })));
    assert!(!is_targets_flow_send_without_flow(&json!({})));
}

#[test]
fn is_targets_flow_send_without_flow_is_false_once_a_persisted_row_pins_target_mode_to_channel_even_with_empty_target_rows()
 {
    assert!(!is_targets_flow_send_without_flow(&json!({
        "targetMode": "channel",
        "targets": [{ "inboxId": "inbox-a" }],
    })));
}

#[test]
fn is_audience_range_ordered_is_true_when_both_bounds_are_null_only_one_bound_is_set_or_start_le_end()
 {
    assert!(is_audience_range_ordered(None, None));
    assert!(is_audience_range_ordered(Some(5), None));
    assert!(is_audience_range_ordered(None, Some(5)));
    assert!(is_audience_range_ordered(Some(3), Some(3)));
    assert!(is_audience_range_ordered(Some(3), Some(10)));
}

#[test]
fn is_audience_range_ordered_is_false_when_start_is_after_end() {
    assert!(!is_audience_range_ordered(Some(10), Some(3)));
}

#[test]
fn resolve_broadcast_audience_range_is_null_when_neither_bound_is_set_callers_keep_today_s_query_byte_identical()
 {
    assert_eq!(resolve_broadcast_audience_range(None, None), None);
}

#[test]
fn resolve_broadcast_audience_range_is_offset_0_with_no_size_cap_when_start_is_1_and_end_is_unset()
{
    assert_eq!(
        resolve_broadcast_audience_range(Some(1), None),
        range(0, None)
    );
}

#[test]
fn resolve_broadcast_audience_range_is_offset_start_1_with_no_size_cap_when_only_start_is_set() {
    assert_eq!(
        resolve_broadcast_audience_range(Some(8), None),
        range(7, None)
    );
}

#[test]
fn resolve_broadcast_audience_range_is_offset_0_with_size_end_when_only_end_is_set() {
    assert_eq!(
        resolve_broadcast_audience_range(None, Some(20)),
        range(0, Some(20))
    );
}

#[test]
fn resolve_broadcast_audience_range_is_offset_start_1_with_size_end_start_1_when_both_bounds_are_set()
 {
    assert_eq!(
        resolve_broadcast_audience_range(Some(5), Some(14)),
        range(4, Some(10))
    );
}

#[test]
fn resolve_broadcast_audience_range_clamps_size_to_0_never_throws_when_the_range_is_unordered() {
    assert_eq!(
        resolve_broadcast_audience_range(Some(10), Some(3)),
        range(9, Some(0))
    );
}

#[test]
fn clamp_audience_count_to_range_returns_the_total_unchanged_when_the_range_is_null() {
    assert_eq!(clamp_audience_count_to_range(42, None), 42);
}

#[test]
fn clamp_audience_count_to_range_returns_the_window_size_when_it_lies_fully_inside_the_total() {
    assert_eq!(clamp_audience_count_to_range(100, range(10, Some(20))), 20);
}

#[test]
fn clamp_audience_count_to_range_clamps_to_the_remaining_rows_when_the_window_extends_past_the_total()
 {
    assert_eq!(clamp_audience_count_to_range(15, range(10, Some(20))), 5);
}

#[test]
fn clamp_audience_count_to_range_is_0_when_the_start_position_lies_past_the_total() {
    assert_eq!(clamp_audience_count_to_range(5, range(10, Some(20))), 0);
}

#[test]
fn clamp_audience_count_to_range_is_0_for_an_unordered_range_size_0() {
    assert_eq!(clamp_audience_count_to_range(100, range(9, Some(0))), 0);
}

#[test]
fn clamp_audience_count_to_range_returns_every_row_from_the_offset_onward_when_size_is_null_open_ended_range()
 {
    assert_eq!(clamp_audience_count_to_range(100, range(10, None)), 90);
}

fn window(offset: i64, limit: i64) -> Option<PageWindow> {
    Some(PageWindow { offset, limit })
}

#[test]
fn resolve_audience_page_window_is_the_plain_page_window_when_there_is_no_range() {
    assert_eq!(resolve_audience_page_window(1, 10, None), window(0, 10));
    assert_eq!(resolve_audience_page_window(3, 10, None), window(20, 10));
}

#[test]
fn resolve_audience_page_window_is_the_page_window_inside_the_range_offset_by_the_range_start() {
    assert_eq!(
        resolve_audience_page_window(1, 10, range(100, Some(50))),
        window(100, 10)
    );
    assert_eq!(
        resolve_audience_page_window(2, 10, range(100, Some(50))),
        window(110, 10)
    );
}

#[test]
fn resolve_audience_page_window_clamps_the_limit_on_the_last_partial_page_of_the_range() {
    assert_eq!(
        resolve_audience_page_window(5, 10, range(100, Some(45))),
        window(140, 5)
    );
}

#[test]
fn resolve_audience_page_window_is_null_once_the_page_lies_entirely_past_the_range() {
    assert_eq!(
        resolve_audience_page_window(6, 10, range(100, Some(45))),
        None
    );
}

fn limit(start: Option<i64>, end: Option<i64>, rate: Option<i64>) -> BroadcastSendLimit {
    BroadcastSendLimit {
        audience_range_start: start,
        audience_range_end: end,
        send_rate_per_minute: rate,
    }
}

#[test]
fn normalize_broadcast_send_limit_normalizes_undefined_to_null_for_every_field() {
    assert_eq!(
        normalize_broadcast_send_limit(BroadcastSendLimit::default()),
        limit(None, None, None)
    );
}

#[test]
fn normalize_broadcast_send_limit_passes_explicit_null_values_through_unchanged() {
    assert_eq!(
        normalize_broadcast_send_limit(limit(None, None, None)),
        limit(None, None, None)
    );
}

#[test]
fn normalize_broadcast_send_limit_keeps_set_numeric_values() {
    assert_eq!(
        normalize_broadcast_send_limit(limit(Some(5), Some(50), Some(200))),
        limit(Some(5), Some(50), Some(200))
    );
}

#[test]
fn resolve_broadcast_send_rate_per_minute_falls_back_to_the_default_rate_when_unset() {
    assert_eq!(
        resolve_broadcast_send_rate_per_minute(None),
        BROADCAST_DEFAULT_SEND_RATE_PER_MINUTE
    );
}

#[test]
fn resolve_broadcast_send_rate_per_minute_returns_the_stored_rate_when_set() {
    assert_eq!(resolve_broadcast_send_rate_per_minute(Some(750)), 750);
}

#[test]
fn resolve_activation_send_rate_keeps_the_stored_rate_when_no_value_is_submitted() {
    assert_eq!(resolve_activation_send_rate(None, Some(250)), Some(250));
}

#[test]
fn resolve_activation_send_rate_uses_a_submitted_numeric_rate() {
    assert_eq!(
        resolve_activation_send_rate(Some(Some(120)), Some(250)),
        Some(120)
    );
}

#[test]
fn resolve_activation_send_rate_uses_submitted_null_to_clear_the_stored_rate() {
    assert_eq!(resolve_activation_send_rate(Some(None), Some(250)), None);
}

#[test]
fn broadcast_send_limit_schema_accepts_null_undefined_and_the_max_rate() {
    assert!(
        parse_broadcast_send_limit(&json!({
            "audienceRangeStart": null,
            "audienceRangeEnd": null,
            "sendRatePerMinute": null,
        }))
        .is_ok()
    );
    assert!(parse_broadcast_send_limit(&json!({})).is_ok());
    assert!(
        parse_broadcast_send_limit(
            &json!({ "sendRatePerMinute": BROADCAST_MAX_SEND_RATE_PER_MINUTE })
        )
        .is_ok()
    );
}

#[test]
fn broadcast_send_limit_schema_rejects_a_position_below_broadcast_audience_position_min() {
    assert!(
        parse_broadcast_send_limit(
            &json!({ "audienceRangeStart": BROADCAST_AUDIENCE_POSITION_MIN - 1 })
        )
        .is_err()
    );
    assert!(parse_broadcast_send_limit(&json!({ "audienceRangeEnd": 0 })).is_err());
}

#[test]
fn broadcast_send_limit_schema_rejects_non_integer_values() {
    assert!(parse_broadcast_send_limit(&json!({ "audienceRangeStart": 1.5 })).is_err());
    assert!(parse_broadcast_send_limit(&json!({ "sendRatePerMinute": 500.5 })).is_err());
}

#[test]
fn broadcast_send_limit_schema_rejects_a_rate_above_broadcast_max_send_rate_per_minute() {
    assert!(
        parse_broadcast_send_limit(
            &json!({ "sendRatePerMinute": BROADCAST_MAX_SEND_RATE_PER_MINUTE + 1 })
        )
        .is_err()
    );
}

#[test]
fn broadcast_send_limit_schema_accepts_a_rate_of_exactly_broadcast_max_send_rate_per_minute_1000() {
    let parsed = parse_broadcast_send_limit(
        &json!({ "sendRatePerMinute": BROADCAST_MAX_SEND_RATE_PER_MINUTE }),
    )
    .expect("parse");
    assert_eq!(
        serde_json::to_value(parsed).expect("json"),
        json!({ "sendRatePerMinute": BROADCAST_MAX_SEND_RATE_PER_MINUTE })
    );
}

#[test]
fn broadcast_send_limit_constants_pins_the_exact_values_the_plan_specifies() {
    assert_eq!(BROADCAST_DEFAULT_SEND_RATE_PER_MINUTE, 500);
    assert_eq!(BROADCAST_MAX_SEND_RATE_PER_MINUTE, 1000);
    assert_eq!(BROADCAST_AUDIENCE_POSITION_MIN, 1);
    assert_eq!(BROADCAST_DISPATCH_WINDOW_MS, 55_000);
}

#[test]
fn broadcast_send_limit_issues_carries_a_stable_message_key_for_an_unordered_audience_range() {
    assert_eq!(
        BROADCAST_SEND_LIMIT_ISSUE_RANGE_END_BEFORE_START,
        "broadcastSendLimit.rangeEndBeforeStart"
    );
}

#[test]
fn resolve_submitted_send_rate_patch_returns_no_patch_when_the_rate_was_not_submitted() {
    assert_eq!(resolve_submitted_send_rate_patch(None), None);
}

#[test]
fn resolve_submitted_send_rate_patch_returns_a_patch_for_a_submitted_number() {
    assert_eq!(
        resolve_submitted_send_rate_patch(Some(Some(120))),
        Some(Some(120))
    );
}

#[test]
fn resolve_submitted_send_rate_patch_returns_a_clearing_patch_for_a_submitted_null() {
    assert_eq!(resolve_submitted_send_rate_patch(Some(None)), Some(None));
}

#[test]
fn recurring_broadcast_vocabulary_recurring_is_a_schedule_type_and_a_status_appended_so_the_enum_migration_only_adds_a_label()
 {
    assert_eq!(BROADCAST_SCHEDULE_TYPES, ["now", "future", "recurring"]);
    assert_eq!(BROADCAST_STATUSES.last(), Some(&"recurring"));
}

#[test]
fn recurring_broadcast_vocabulary_every_skip_and_miss_reason_the_ledger_can_store_is_a_known_value()
{
    assert_eq!(
        BROADCAST_OCCURRENCE_OUTCOMES,
        ["created", "skipped", "missed"]
    );
    assert_eq!(
        BROADCAST_OCCURRENCE_REASONS,
        [
            "catchUpSuperseded",
            "paused",
            "draft",
            "blockedOwner",
            "planLimit",
            "noTargets",
            "undeliverable",
            "excludedDate",
        ]
    );
}

#[test]
fn recurring_broadcast_vocabulary_a_template_is_a_recurring_broadcast_with_no_occurrence() {
    assert!(is_recurring_template(
        &json!({ "schedulesType": "recurring" })
    ));
    assert!(is_recurring_template(
        &json!({ "schedulesType": "recurring", "occurrenceAt": null })
    ));
    assert!(!is_recurring_template(&json!({
        "schedulesType": "recurring",
        "occurrenceAt": GRACE_NOW,
    })));
    assert!(!is_recurring_template(
        &json!({ "schedulesType": "future" })
    ));
}

#[test]
fn recurring_broadcast_vocabulary_a_run_is_identified_by_occurrence_at_alone_so_it_survives_its_template_being_purged()
 {
    assert!(is_recurrence_run(&json!({ "occurrenceAt": GRACE_NOW })));
    assert!(!is_recurrence_run(&json!({ "occurrenceAt": null })));
    assert!(!is_recurrence_run(&json!({})));
}

const CAP: i64 = 100_000;

fn capped() -> CappedCount {
    CappedCount {
        total: CAP,
        capped: true,
    }
}

fn count(total: i64, capped: bool) -> CappedCount {
    CappedCount { total, capped }
}

#[test]
fn clamp_capped_audience_count_behaves_like_the_exact_clamp_when_the_count_was_not_capped() {
    assert_eq!(
        clamp_capped_audience_count(count(100, false), range(10, Some(20))),
        count(20, false)
    );
}

#[test]
fn clamp_capped_audience_count_stays_a_lower_bound_with_no_window() {
    assert_eq!(
        clamp_capped_audience_count(capped(), None),
        count(CAP, true)
    );
}

#[test]
fn clamp_capped_audience_count_stays_a_lower_bound_for_an_open_ended_window() {
    assert_eq!(
        clamp_capped_audience_count(capped(), range(10, None)),
        count(CAP - 10, true)
    );
}

#[test]
fn clamp_capped_audience_count_is_exact_when_the_window_ends_inside_the_counted_rows() {
    assert_eq!(
        clamp_capped_audience_count(capped(), range(0, Some(500))),
        count(500, false)
    );
}

#[test]
fn clamp_capped_audience_count_is_exact_when_the_window_ends_exactly_at_the_cap() {
    assert_eq!(
        clamp_capped_audience_count(capped(), range(0, Some(CAP))),
        count(CAP, false)
    );
}

#[test]
fn clamp_capped_audience_count_stays_capped_when_the_window_extends_past_the_counted_rows() {
    assert_eq!(
        clamp_capped_audience_count(capped(), range(CAP - 1000, Some(5000))),
        count(1000, true)
    );
}

#[test]
fn clamp_capped_audience_count_reports_0_but_capped_for_a_window_that_starts_past_the_counted_rows()
{
    assert_eq!(
        clamp_capped_audience_count(capped(), range(CAP + 5, Some(10))),
        count(0, true)
    );
}
