//! Port of `partials/broadcast.ts`. The broadcast-shaped inputs of the predicates
//! (`targets`, `targetMode`, `flowId`, `templateId`, `occurrenceAt`) are the JSON object, with
//! TS truthiness; times are unix ms.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::shared::ZodError;

/// `broadcastStatuses`.
pub const BROADCAST_STATUSES: &[&str] = &[
    "scheduled",
    "sent",
    "sending",
    "cancelled",
    "draft",
    "failed",
    "recurring",
];

/// `BROADCAST_TRIAL_SEND_RATE_PER_MINUTE`.
pub const BROADCAST_TRIAL_SEND_RATE_PER_MINUTE: i64 = 60;
/// `BROADCAST_TRIAL_MAX_ACTIVE`.
pub const BROADCAST_TRIAL_MAX_ACTIVE: i64 = 1;
/// `BROADCAST_TRIAL_DISPLAYED_SEND_RATE_PER_MINUTE`.
pub const BROADCAST_TRIAL_DISPLAYED_SEND_RATE_PER_MINUTE: i64 = 100;
/// `BROADCAST_TRIAL_UPGRADE_SPEED_MULTIPLIER`.
pub const BROADCAST_TRIAL_UPGRADE_SPEED_MULTIPLIER: i64 = 20;

/// `broadcastScheduleTypes.options`.
pub const BROADCAST_SCHEDULE_TYPES: &[&str] = &["now", "future", "recurring"];

/// `broadcastOccurrenceOutcomes.options`.
pub const BROADCAST_OCCURRENCE_OUTCOMES: &[&str] = &["created", "skipped", "missed"];

/// `broadcastOccurrenceReasons.options`.
pub const BROADCAST_OCCURRENCE_REASONS: &[&str] = &[
    "catchUpSuperseded",
    "paused",
    "draft",
    "blockedOwner",
    "planLimit",
    "noTargets",
    "undeliverable",
    "excludedDate",
];

/// `broadcastSubactions.options` (`@chatbotx.io/utils/broadcast`).
pub const BROADCAST_SUBACTIONS: &[&str] = &[
    "allContacts",
    "messengerActiveContacts",
    "messengerTemplateMessage",
    "whatsappTemplateMessage",
    "whatsappWithin24Hours",
    "instagramActiveContacts",
    "telegramAllContacts",
    "tiktokActiveContacts",
];

/// `broadcastSubactionAudienceRules` as (subaction, requiresRecentInteractionWindow).
pub const BROADCAST_SUBACTION_AUDIENCE_RULES: &[(&str, bool)] = &[
    ("allContacts", false),
    ("messengerActiveContacts", true),
    ("messengerTemplateMessage", false),
    ("whatsappTemplateMessage", false),
    ("whatsappWithin24Hours", true),
    ("instagramActiveContacts", true),
    ("telegramAllContacts", false),
    ("tiktokActiveContacts", true),
];

/// `isRecurringTemplate`: a `recurring` broadcast without `occurrenceAt`.
pub fn is_recurring_template(row: &Value) -> bool {
    let _ = row;
    todo!("P3.2b")
}

/// `isRecurrenceRun`: a broadcast with an `occurrenceAt`.
pub fn is_recurrence_run(row: &Value) -> bool {
    let _ = row;
    todo!("P3.2b")
}

/// `BROADCAST_FAILED_RATE_THRESHOLD`.
pub const BROADCAST_FAILED_RATE_THRESHOLD: f64 = 1.0;
/// `BROADCAST_OUTCOME_GRACE_MS`.
pub const BROADCAST_OUTCOME_GRACE_MS: i64 = 10 * 60 * 1000;

/// `resolveBroadcastTerminalStatus`: `"failed"` when every contact failed, else `"sent"`.
pub fn resolve_broadcast_terminal_status(
    contact_count: Option<i64>,
    failed_count: i64,
) -> &'static str {
    let _ = (contact_count, failed_count);
    todo!("P3.2b")
}

/// `isBroadcastOutcomeGraceElapsed`.
pub fn is_broadcast_outcome_grace_elapsed(handoff_completed_at: i64, now: i64) -> bool {
    let _ = (handoff_completed_at, now);
    todo!("P3.2b")
}

/// `requiresRecentInteractionWindow` (`None` is `null` / `undefined`).
pub fn requires_recent_interaction_window(subaction: Option<&str>) -> bool {
    let _ = subaction;
    todo!("P3.2b")
}

/// `BroadcastChannelCapability`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BroadcastChannelCapability {
    pub channel: &'static str,
    pub subactions: &'static [&'static str],
    pub default_subaction: &'static str,
    pub supports_template_broadcast: bool,
}

/// `broadcastChannelCapabilities`.
pub const BROADCAST_CHANNEL_CAPABILITIES: &[BroadcastChannelCapability] = &[
    BroadcastChannelCapability {
        channel: "omnichannel",
        subactions: &["allContacts"],
        default_subaction: "allContacts",
        supports_template_broadcast: false,
    },
    BroadcastChannelCapability {
        channel: "messenger",
        subactions: &["messengerTemplateMessage", "messengerActiveContacts"],
        default_subaction: "messengerTemplateMessage",
        supports_template_broadcast: true,
    },
    BroadcastChannelCapability {
        channel: "whatsapp",
        subactions: &["whatsappTemplateMessage", "whatsappWithin24Hours"],
        default_subaction: "whatsappTemplateMessage",
        supports_template_broadcast: true,
    },
    BroadcastChannelCapability {
        channel: "zalo",
        subactions: &["allContacts"],
        default_subaction: "allContacts",
        supports_template_broadcast: false,
    },
    BroadcastChannelCapability {
        channel: "instagram",
        subactions: &["instagramActiveContacts"],
        default_subaction: "instagramActiveContacts",
        supports_template_broadcast: false,
    },
    BroadcastChannelCapability {
        channel: "telegram",
        subactions: &["telegramAllContacts"],
        default_subaction: "telegramAllContacts",
        supports_template_broadcast: false,
    },
    BroadcastChannelCapability {
        channel: "tiktok",
        subactions: &["tiktokActiveContacts"],
        default_subaction: "tiktokActiveContacts",
        supports_template_broadcast: false,
    },
];

/// `findBroadcastChannelCapability`.
pub fn find_broadcast_channel_capability(
    channel: &str,
) -> Option<&'static BroadcastChannelCapability> {
    let _ = channel;
    todo!("P3.2b")
}

/// `usesBroadcastTargets`: `targetMode == "targets"` when set, else a non-empty `targets`.
pub fn uses_broadcast_targets(broadcast: &Value) -> bool {
    let _ = broadcast;
    todo!("P3.2b")
}

/// `broadcastSendsFlow`.
pub fn broadcast_sends_flow(broadcast: &Value) -> bool {
    let _ = broadcast;
    todo!("P3.2b")
}

/// `broadcastSendsTemplate`.
pub fn broadcast_sends_template(broadcast: &Value) -> bool {
    let _ = broadcast;
    todo!("P3.2b")
}

/// `isTargetsTemplateSendWithoutTemplate`.
pub fn is_targets_template_send_without_template(broadcast: &Value) -> bool {
    let _ = broadcast;
    todo!("P3.2b")
}

/// `isTargetsFlowSendWithoutFlow`.
pub fn is_targets_flow_send_without_flow(broadcast: &Value) -> bool {
    let _ = broadcast;
    todo!("P3.2b")
}

/// `BROADCAST_DEFAULT_SEND_RATE_PER_MINUTE`.
pub const BROADCAST_DEFAULT_SEND_RATE_PER_MINUTE: i64 = 500;
/// `BROADCAST_MAX_SEND_RATE_PER_MINUTE`.
pub const BROADCAST_MAX_SEND_RATE_PER_MINUTE: i64 = 1000;
/// `BROADCAST_AUDIENCE_POSITION_MIN`.
pub const BROADCAST_AUDIENCE_POSITION_MIN: i64 = 1;
/// `BROADCAST_DISPATCH_WINDOW_MS`.
pub const BROADCAST_DISPATCH_WINDOW_MS: i64 = 55_000;

/// `broadcastSendLimitSchema` (every field `.nullish()`: `None` is `null` and absent).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastSendLimit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience_range_start: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience_range_end: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_rate_per_minute: Option<i64>,
}

/// `broadcastSendLimitSchema.safeParse`.
pub fn parse_broadcast_send_limit(value: &Value) -> Result<BroadcastSendLimit, ZodError> {
    let _ = value;
    todo!("P3.2b")
}

/// `broadcastSendLimitIssues.rangeEndBeforeStart`.
pub const BROADCAST_SEND_LIMIT_ISSUE_RANGE_END_BEFORE_START: &str =
    "broadcastSendLimit.rangeEndBeforeStart";

/// `isAudienceRangeOrdered`.
pub fn is_audience_range_ordered(start: Option<i64>, end: Option<i64>) -> bool {
    let _ = (start, end);
    todo!("P3.2b")
}

/// `BroadcastAudienceRange` (`size` `None` is open-ended).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BroadcastAudienceRange {
    pub offset: i64,
    pub size: Option<i64>,
}

/// `resolveBroadcastAudienceRange`: `None` when neither bound is set.
pub fn resolve_broadcast_audience_range(
    start: Option<i64>,
    end: Option<i64>,
) -> Option<BroadcastAudienceRange> {
    let _ = (start, end);
    todo!("P3.2b")
}

/// `clampAudienceCountToRange`.
pub fn clamp_audience_count_to_range(total: i64, range: Option<BroadcastAudienceRange>) -> i64 {
    let _ = (total, range);
    todo!("P3.2b")
}

/// A count that may be a lower bound (`{ total, capped }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CappedCount {
    pub total: i64,
    pub capped: bool,
}

/// `clampCappedAudienceCount`.
pub fn clamp_capped_audience_count(
    counted: CappedCount,
    range: Option<BroadcastAudienceRange>,
) -> CappedCount {
    let _ = (counted, range);
    todo!("P3.2b")
}

/// The result of `resolveAudiencePageWindow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageWindow {
    pub offset: i64,
    pub limit: i64,
}

/// `resolveAudiencePageWindow`: `None` once the page lies past the range.
pub fn resolve_audience_page_window(
    page: i64,
    per_page: i64,
    range: Option<BroadcastAudienceRange>,
) -> Option<PageWindow> {
    let _ = (page, per_page, range);
    todo!("P3.2b")
}

/// `normalizeBroadcastSendLimit` (the Rust `None` is already `null`).
pub fn normalize_broadcast_send_limit(input: BroadcastSendLimit) -> BroadcastSendLimit {
    let _ = input;
    todo!("P3.2b")
}

/// `resolveBroadcastSendRatePerMinute`.
pub fn resolve_broadcast_send_rate_per_minute(send_rate_per_minute: Option<i64>) -> i64 {
    let _ = send_rate_per_minute;
    todo!("P3.2b")
}

/// `resolveActivationSendRate`: `submitted` `None` is `undefined` (keep `stored`),
/// `Some(None)` is `null` (clear).
pub fn resolve_activation_send_rate(
    submitted: Option<Option<i64>>,
    stored: Option<i64>,
) -> Option<i64> {
    let _ = (submitted, stored);
    todo!("P3.2b")
}

/// `resolveSubmittedSendRatePatch`: `None` for `undefined`, else the patch value
/// (`Some(None)` clears the rate).
pub fn resolve_submitted_send_rate_patch(submitted: Option<Option<i64>>) -> Option<Option<i64>> {
    let _ = submitted;
    todo!("P3.2b")
}
