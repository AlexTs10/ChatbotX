//! Port of `partials/broadcast-plan-policy.ts`.

use serde::{Deserialize, Serialize};

use super::broadcast::{
    BROADCAST_TRIAL_DISPLAYED_SEND_RATE_PER_MINUTE, BROADCAST_TRIAL_MAX_ACTIVE,
    BROADCAST_TRIAL_SEND_RATE_PER_MINUTE, BROADCAST_TRIAL_UPGRADE_SPEED_MULTIPLIER,
};
use super::shared::{ZodError, deserialize};

/// The display values of a restricted policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastPlanPolicyDisplay {
    pub send_rate_per_minute: i64,
    pub upgrade_speed_multiplier: i64,
}

/// `RestrictedBroadcastPlanPolicy` (without the `kind` tag).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestrictedBroadcastPlanPolicy {
    pub max_send_rate_per_minute: i64,
    pub max_active_broadcasts: i64,
    pub channels: &'static [&'static str],
    pub display: BroadcastPlanPolicyDisplay,
}

/// `BroadcastPlanPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BroadcastPlanPolicy {
    Unrestricted,
    Restricted(RestrictedBroadcastPlanPolicy),
}

/// `UNRESTRICTED_BROADCAST_PLAN_POLICY`.
pub const UNRESTRICTED_BROADCAST_PLAN_POLICY: BroadcastPlanPolicy =
    BroadcastPlanPolicy::Unrestricted;

/// `TRIAL_BROADCAST_PLAN_POLICY`.
pub const TRIAL_BROADCAST_PLAN_POLICY: RestrictedBroadcastPlanPolicy =
    RestrictedBroadcastPlanPolicy {
        max_send_rate_per_minute: BROADCAST_TRIAL_SEND_RATE_PER_MINUTE,
        max_active_broadcasts: BROADCAST_TRIAL_MAX_ACTIVE,
        channels: &["messenger"],
        display: BroadcastPlanPolicyDisplay {
            send_rate_per_minute: BROADCAST_TRIAL_DISPLAYED_SEND_RATE_PER_MINUTE,
            upgrade_speed_multiplier: BROADCAST_TRIAL_UPGRADE_SPEED_MULTIPLIER,
        },
    };

/// `RESTRICTED_BROADCAST_PLAN_POLICIES`.
pub const RESTRICTED_BROADCAST_PLAN_POLICIES: &[RestrictedBroadcastPlanPolicy] =
    &[TRIAL_BROADCAST_PLAN_POLICY];

/// `ACTIVE_BROADCAST_STATUSES`.
pub const ACTIVE_BROADCAST_STATUSES: &[&str] = &["scheduled", "sending"];

/// `broadcastPlanLimitReasons`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BroadcastPlanLimitReason {
    SendRate,
    ActiveBroadcasts,
}

/// `broadcastPlanLimitDataSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastPlanLimitData {
    pub reason: BroadcastPlanLimitReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_name: Option<String>,
    pub max_send_rate_per_minute: i64,
    pub max_active_broadcasts: i64,
    pub displayed_send_rate_per_minute: i64,
    pub upgrade_speed_multiplier: i64,
}

/// `resolveApplicablePolicy`: the restricted policy when it lists `channel`.
pub fn resolve_applicable_policy<'a>(
    policy: &'a BroadcastPlanPolicy,
    channel: &str,
) -> Option<&'a RestrictedBroadcastPlanPolicy> {
    match policy {
        BroadcastPlanPolicy::Unrestricted => None,
        BroadcastPlanPolicy::Restricted(restricted) => {
            restricted.channels.contains(&channel).then_some(restricted)
        }
    }
}

/// `isChannelRestrictedByAnyPolicy`.
pub fn is_channel_restricted_by_any_policy(channel: &str) -> bool {
    RESTRICTED_BROADCAST_PLAN_POLICIES
        .iter()
        .any(|policy| policy.channels.contains(&channel))
}

/// `exceedsPlanSendRate`.
pub fn exceeds_plan_send_rate(
    policy: &RestrictedBroadcastPlanPolicy,
    send_rate_per_minute: Option<i64>,
) -> bool {
    send_rate_per_minute.is_some_and(|rate| rate > policy.max_send_rate_per_minute)
}

/// `resolveSendRateOverride` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendRateOverride {
    pub send_rate_per_minute: i64,
}

/// `resolveSendRateOverride`: the rate, else the policy cap.
pub fn resolve_send_rate_override(
    policy: &RestrictedBroadcastPlanPolicy,
    send_rate_per_minute: Option<i64>,
) -> SendRateOverride {
    SendRateOverride {
        send_rate_per_minute: send_rate_per_minute.unwrap_or(policy.max_send_rate_per_minute),
    }
}

/// `broadcastPlanLimitDataSchema.safeParse`.
pub fn parse_broadcast_plan_limit_data(
    value: &serde_json::Value,
) -> Result<BroadcastPlanLimitData, ZodError> {
    deserialize(value)
}
