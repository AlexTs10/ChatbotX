//! Port of `packages/database/__tests__/broadcast-plan-policy.test.ts` (P3.2b, ported:
//! `toEqual` of the trial policy compares its JSON; `toBe(policy)` is equality; the TS
//! `undefined` and `null` rates are both `None`; `channelTypes.options` is the list of
//! `@chatbotx.io/utils/channel`).

use chatbotx_db::repo::partials::broadcast_plan_policy::{
    ACTIVE_BROADCAST_STATUSES, BroadcastPlanPolicy, RESTRICTED_BROADCAST_PLAN_POLICIES,
    SendRateOverride, TRIAL_BROADCAST_PLAN_POLICY, UNRESTRICTED_BROADCAST_PLAN_POLICY,
    exceeds_plan_send_rate, is_channel_restricted_by_any_policy, parse_broadcast_plan_limit_data,
    resolve_applicable_policy, resolve_send_rate_override,
};
use serde_json::json;

/// `channelTypes.options`.
const CHANNEL_TYPES: &[&str] = &[
    "omnichannel",
    "webchat",
    "messenger",
    "whatsapp",
    "zalo",
    "smtp",
    "telegram",
    "instagram",
    "threads",
    "tiktok",
    "api",
];

fn trial() -> BroadcastPlanPolicy {
    BroadcastPlanPolicy::Restricted(TRIAL_BROADCAST_PLAN_POLICY)
}

#[test]
fn broadcast_plan_policy_defines_the_trial_messenger_limits_and_display_values() {
    assert_eq!(
        serde_json::to_value(trial()).expect("json"),
        json!({
            "kind": "restricted",
            "maxSendRatePerMinute": 60,
            "maxActiveBroadcasts": 1,
            "channels": ["messenger"],
            "display": { "sendRatePerMinute": 100, "upgradeSpeedMultiplier": 20 },
        })
    );
}

#[test]
fn broadcast_plan_policy_applies_the_trial_policy_only_to_messenger() {
    let policy = trial();
    assert_eq!(
        resolve_applicable_policy(&policy, "messenger"),
        Some(&TRIAL_BROADCAST_PLAN_POLICY)
    );
    for channel in CHANNEL_TYPES {
        if *channel != "messenger" {
            assert_eq!(
                resolve_applicable_policy(&policy, channel),
                None,
                "{channel}"
            );
        }
    }
}

#[test]
fn broadcast_plan_policy_registers_the_trial_policy_for_channel_first_restriction_checks() {
    assert!(RESTRICTED_BROADCAST_PLAN_POLICIES.contains(&TRIAL_BROADCAST_PLAN_POLICY));
    assert!(is_channel_restricted_by_any_policy("messenger"));
    assert!(!is_channel_restricted_by_any_policy("whatsapp"));
}

#[test]
fn broadcast_plan_policy_does_not_apply_an_unrestricted_policy_to_any_channel() {
    for channel in CHANNEL_TYPES {
        assert_eq!(
            resolve_applicable_policy(&UNRESTRICTED_BROADCAST_PLAN_POLICY, channel),
            None,
            "{channel}"
        );
    }
}

#[test]
fn broadcast_plan_policy_detects_only_send_rates_above_the_restricted_cap() {
    assert!(!exceeds_plan_send_rate(&TRIAL_BROADCAST_PLAN_POLICY, None));
    assert!(!exceeds_plan_send_rate(
        &TRIAL_BROADCAST_PLAN_POLICY,
        Some(60)
    ));
    assert!(exceeds_plan_send_rate(
        &TRIAL_BROADCAST_PLAN_POLICY,
        Some(61)
    ));
}

fn assert_override(rate: Option<i64>, expected: i64) {
    assert_eq!(
        resolve_send_rate_override(&TRIAL_BROADCAST_PLAN_POLICY, rate),
        SendRateOverride {
            send_rate_per_minute: expected
        }
    );
}

#[test]
fn broadcast_plan_policy_resolves_a_restricted_rate_of_undefined_to_60() {
    assert_override(None, 60);
}

#[test]
fn broadcast_plan_policy_resolves_a_restricted_rate_of_null_to_60() {
    assert_override(None, 60);
}

#[test]
fn broadcast_plan_policy_resolves_a_restricted_rate_of_30_to_30() {
    assert_override(Some(30), 30);
}

#[test]
fn broadcast_plan_policy_resolves_a_restricted_rate_of_60_to_60() {
    assert_override(Some(60), 60);
}

#[test]
fn broadcast_plan_policy_defines_scheduled_and_sending_as_the_active_slot_statuses() {
    assert_eq!(ACTIVE_BROADCAST_STATUSES, ["scheduled", "sending"]);
}

#[test]
fn broadcast_plan_policy_accepts_an_absent_plan_name_and_rejects_nullable_numeric_limits() {
    let valid = json!({
        "reason": "sendRate",
        "maxSendRatePerMinute": 60,
        "maxActiveBroadcasts": 1,
        "displayedSendRatePerMinute": 100,
        "upgradeSpeedMultiplier": 20,
    });
    assert!(parse_broadcast_plan_limit_data(&valid).is_ok());
    let mut invalid = valid.clone();
    invalid["maxSendRatePerMinute"] = json!(null);
    assert!(parse_broadcast_plan_limit_data(&invalid).is_err());
}
