//! Port of `broadcast/plan-policy.service.ts`.

use chatbotx_db::repo::broadcast as broadcast_repo;
use chatbotx_db::repo::partials::broadcast_plan_policy::{
    ACTIVE_BROADCAST_STATUSES, BroadcastPlanLimitReason, BroadcastPlanPolicy,
    RestrictedBroadcastPlanPolicy, TRIAL_BROADCAST_PLAN_POLICY, exceeds_plan_send_rate,
    is_channel_restricted_by_any_policy, resolve_applicable_policy,
};
use chatbotx_db::rusqlite::Connection;
use serde_json::json;

use crate::errors::{BusinessError, Result};

/// `BROADCAST_PLAN_LIMIT_CODE`.
pub const BROADCAST_PLAN_LIMIT_CODE: &str = "broadcastPlanLimit";

/// `BroadcastPlanContext`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanContext {
    pub policy: BroadcastPlanPolicy,
    pub plan_name: Option<String>,
}

/// `RestrictedBroadcastPlanContext`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestrictedContext {
    pub policy: RestrictedBroadcastPlanPolicy,
    pub plan_name: Option<String>,
}

impl PlanContext {
    /// The unrestricted context.
    pub fn unrestricted() -> PlanContext {
        PlanContext {
            policy: BroadcastPlanPolicy::Unrestricted,
            plan_name: None,
        }
    }

    /// The trial context (`TRIAL_BROADCAST_PLAN_POLICY`).
    pub fn trial(plan_name: Option<String>) -> PlanContext {
        PlanContext {
            policy: BroadcastPlanPolicy::Restricted(TRIAL_BROADCAST_PLAN_POLICY),
            plan_name,
        }
    }
}

/// `resolveForWorkspace`.
///
/// ponytail: the owner's plan identity (`userQuotaService.getPlanIdentity`)
/// is not ported, so every edition is unrestricted (the community edition is
/// unrestricted in the original too); wire the quota service here when ported.
pub fn resolve_for_workspace(_workspace_id: i64) -> PlanContext {
    PlanContext::unrestricted()
}

/// `appliesToChannel`.
pub fn applies_to_channel(channel: &str) -> bool {
    is_channel_restricted_by_any_policy(channel)
}

/// `hasRestrictions`.
pub fn has_restrictions(ctx: &PlanContext) -> bool {
    matches!(ctx.policy, BroadcastPlanPolicy::Restricted(_))
}

/// `restrictionFor`.
pub fn restriction_for(ctx: &PlanContext, channel: &str) -> Option<RestrictedContext> {
    resolve_applicable_policy(&ctx.policy, channel).map(|policy| RestrictedContext {
        policy: *policy,
        plan_name: ctx.plan_name.clone(),
    })
}

/// `broadcastPlanLimitException`.
pub fn plan_limit_exception(
    reason: BroadcastPlanLimitReason,
    ctx: &RestrictedContext,
) -> BusinessError {
    let policy = &ctx.policy;
    let message = match reason {
        BroadcastPlanLimitReason::SendRate => format!(
            "The current plan sends broadcasts at up to {} messages per minute",
            policy.max_send_rate_per_minute
        ),
        BroadcastPlanLimitReason::ActiveBroadcasts => format!(
            "The current plan runs up to {} broadcast(s) at a time",
            policy.max_active_broadcasts
        ),
    };
    let mut data = json!({
        "reason": reason,
        "maxSendRatePerMinute": policy.max_send_rate_per_minute,
        "maxActiveBroadcasts": policy.max_active_broadcasts,
        "displayedSendRatePerMinute": policy.display.send_rate_per_minute,
        "upgradeSpeedMultiplier": policy.display.upgrade_speed_multiplier,
    });
    if let Some(name) = ctx
        .plan_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        data["planName"] = json!(name);
    }
    BusinessError::Exception {
        message,
        code: BROADCAST_PLAN_LIMIT_CODE.to_owned(),
        status: 403,
        field: None,
        data: Some(data),
    }
}

/// `assertSendRateAllowed`.
pub fn assert_send_rate_allowed(
    ctx: &RestrictedContext,
    send_rate_per_minute: Option<i64>,
) -> Result<()> {
    if exceeds_plan_send_rate(&ctx.policy, send_rate_per_minute) {
        return Err(plan_limit_exception(
            BroadcastPlanLimitReason::SendRate,
            ctx,
        ));
    }
    Ok(())
}

/// `resolveSendRateOverride`: the submitted rate, else the plan's cap.
pub fn resolve_send_rate_override(
    ctx: &RestrictedContext,
    send_rate_per_minute: Option<i64>,
) -> i64 {
    send_rate_per_minute.unwrap_or(ctx.policy.max_send_rate_per_minute)
}

/// `assertActivationAllowed` with `checkSlot: true`: the rate, then one active slot.
/// The single SQLite writer is the activation lock (`pg_advisory_xact_lock`).
pub fn assert_activation_allowed(
    tx: &Connection,
    workspace_id: i64,
    channel: &str,
    ctx: &RestrictedContext,
    send_rate_per_minute: Option<i64>,
    exclude_broadcast_id: Option<i64>,
) -> Result<i64> {
    assert_send_rate_allowed(ctx, send_rate_per_minute)?;
    let active = broadcast_repo::count_active(
        tx,
        workspace_id,
        channel,
        ACTIVE_BROADCAST_STATUSES,
        exclude_broadcast_id,
    )?;
    if active >= ctx.policy.max_active_broadcasts {
        return Err(plan_limit_exception(
            BroadcastPlanLimitReason::ActiveBroadcasts,
            ctx,
        ));
    }
    Ok(resolve_send_rate_override(ctx, send_rate_per_minute))
}
