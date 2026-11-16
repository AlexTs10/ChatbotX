//! `quota-enforcement/service.ts` (Q.D11 slice 6): two-level quota gates on
//! `UserQuota`. A sub-account of an active reseller tenant is gated by its
//! own row AND the pool (the tenant owner's row); the reseller acting
//! directly uses only the pool; a root-tenant user uses its own row.
//!
//! The original keeps Redis live counters, a per-scope distributed lock and
//! an atomic / lock MAC admission switch (`QUOTA_MAC_ADMISSION`). Here every
//! function runs on the caller's SQLite connection: the single write
//! transaction serializes check + create + consume, and the `*Used` column
//! is the live count (DECISIONS).

use chatbotx_db::repo::{self, now_ms, platform_admin::ROOT_TENANT_ID};
use chatbotx_db::rusqlite::{Connection, OptionalExtension};

use crate::user_quota::{self, QuotaMetric};
use crate::workspace_usage::{self, WorkspaceUsageMetric};

/// `ConsumeLevel`: the level that refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsumeLevel {
    User,
    Pool,
}

impl ConsumeLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            ConsumeLevel::User => "user",
            ConsumeLevel::Pool => "pool",
        }
    }
}

/// `QuotaContext`: `owner_id` = the reseller of an active non-root tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaContext {
    pub tenant_id: i64,
    pub owner_id: Option<i64>,
}

impl QuotaContext {
    /// `isPooled`: the pool owner.
    fn pool(self) -> Option<i64> {
        self.owner_id.filter(|_| self.tenant_id != ROOT_TENANT_ID)
    }
}

/// `resolveContext`: the user's tenant (else the tenant it owns); a tenant
/// that is not active has no pool.
pub fn resolve_context(conn: &Connection, user_id: i64) -> repo::Result<QuotaContext> {
    let root = QuotaContext {
        tenant_id: ROOT_TENANT_ID,
        owner_id: None,
    };
    let creator: Option<i64> = conn
        .query_row(
            r#"SELECT "tenantId" FROM "User" WHERE "id" = ?1"#,
            [user_id],
            |row| row.get(0),
        )
        .optional()?;
    let tenant_id = match creator {
        Some(id) if id != ROOT_TENANT_ID => id,
        _ => match repo::tenant::find_by_owner(conn, user_id)? {
            Some(owned) => owned.id,
            None => return Ok(root),
        },
    };
    if tenant_id == ROOT_TENANT_ID {
        return Ok(root);
    }
    Ok(match repo::tenant::find_by_id(conn, tenant_id)? {
        Some(tenant) if tenant.status == "active" => QuotaContext {
            tenant_id,
            owner_id: tenant.owner_id,
        },
        _ => root,
    })
}

/// `levelsForCtx`: the rows a consumption touches, pool first.
fn levels(ctx: QuotaContext, user_id: i64) -> Vec<(i64, ConsumeLevel)> {
    match ctx.pool() {
        None => vec![(user_id, ConsumeLevel::User)],
        Some(owner) if owner == user_id => vec![(owner, ConsumeLevel::Pool)],
        Some(owner) => vec![(owner, ConsumeLevel::Pool), (user_id, ConsumeLevel::User)],
    }
}

/// `tryConsume`: checks every level (DB `used < limit`), then consumes one
/// unit at each. `Err(level)` = the level that is full (nothing consumed).
pub fn try_consume(
    conn: &Connection,
    user_id: i64,
    metric: QuotaMetric,
) -> repo::Result<Result<(), ConsumeLevel>> {
    let levels = levels(resolve_context(conn, user_id)?, user_id);
    for &(id, level) in &levels {
        if !user_quota::has_capacity(conn, id, metric)? {
            return Ok(Err(level));
        }
    }
    for &(id, _) in &levels {
        user_quota::adjust(conn, id, metric, 1)?;
    }
    Ok(Ok(()))
}

/// `userQuotaService.isLimitReached`: no row = never; else `used >= limit`.
fn limit_reached(conn: &Connection, user_id: i64, metric: QuotaMetric) -> repo::Result<bool> {
    Ok(user_quota::get_for_user(conn, user_id)?.is_some_and(|q| {
        let (limit, used) = metric.values(&q);
        limit.is_some_and(|limit| used >= limit)
    }))
}

/// `isAtLimit`: any level at its limit.
pub fn is_at_limit(conn: &Connection, user_id: i64, metric: QuotaMetric) -> repo::Result<bool> {
    for (id, _) in levels(resolve_context(conn, user_id)?, user_id) {
        if limit_reached(conn, id, metric)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn adjust_levels(
    conn: &Connection,
    ctx: QuotaContext,
    user_id: i64,
    metric: QuotaMetric,
    delta: i64,
) -> repo::Result<()> {
    for (id, _) in levels(ctx, user_id) {
        user_quota::adjust(conn, id, metric, delta)?;
    }
    Ok(())
}

/// `incrementBy`: `+count` at every level (`count <= 0` = no-op).
pub fn increment_by(
    conn: &Connection,
    user_id: i64,
    metric: QuotaMetric,
    count: i64,
) -> repo::Result<()> {
    if count <= 0 {
        return Ok(());
    }
    adjust_levels(
        conn,
        resolve_context(conn, user_id)?,
        user_id,
        metric,
        count,
    )
}

/// `releaseBy`: `-count` at every level (never below 0).
pub fn release_by(
    conn: &Connection,
    user_id: i64,
    metric: QuotaMetric,
    count: i64,
) -> repo::Result<()> {
    if count <= 0 {
        return Ok(());
    }
    adjust_levels(
        conn,
        resolve_context(conn, user_id)?,
        user_id,
        metric,
        -count,
    )
}

/// `getRemainingSlots`: `None` = unlimited (no row or no limit).
fn remaining_slots(
    conn: &Connection,
    user_id: i64,
    metric: QuotaMetric,
) -> repo::Result<Option<i64>> {
    Ok(user_quota::get_for_user(conn, user_id)?.and_then(|q| {
        let (limit, used) = metric.values(&q);
        limit.map(|limit| (limit - used).max(0))
    }))
}

fn dual_remaining_for_ctx(
    conn: &Connection,
    ctx: QuotaContext,
    user_id: i64,
    metric: QuotaMetric,
) -> repo::Result<Option<i64>> {
    let mut tightest = None;
    for (id, _) in levels(ctx, user_id) {
        if let Some(left) = remaining_slots(conn, id, metric)? {
            tightest = Some(tightest.map_or(left, |t: i64| t.min(left)));
        }
    }
    Ok(tightest)
}

/// `getDualRemainingSlots`: the tighter of the user and pool remaining.
pub fn dual_remaining_slots(
    conn: &Connection,
    user_id: i64,
    metric: QuotaMetric,
) -> repo::Result<Option<i64>> {
    dual_remaining_for_ctx(conn, resolve_context(conn, user_id)?, user_id, metric)
}

/// `isTeamMemberLimitReached`: the row's limit against the live distinct
/// member count of the scope (`additional` pending inserts).
fn team_member_limit_reached(
    conn: &Connection,
    limit_user: i64,
    tenant_scope: Option<i64>,
    additional: i64,
) -> repo::Result<bool> {
    let Some(quota) = user_quota::get_for_user(conn, limit_user)? else {
        return Ok(false);
    };
    let Some(limit) = quota.team_members_limit else {
        return Ok(false);
    };
    let count = match tenant_scope {
        Some(tenant) => user_quota::count_distinct_team_members_for_tenant(conn, tenant)?,
        None => user_quota::count_distinct_team_members(conn, limit_user)?,
    };
    Ok(count + additional > limit)
}

/// `anyLevelAtLimit`: `at_limit(limit_user, tenant_scope)` per level; the
/// reseller acting directly is governed by the pool only.
fn any_level_at_limit(
    ctx: QuotaContext,
    user_id: i64,
    mut at_limit: impl FnMut(i64, Option<i64>) -> repo::Result<bool>,
) -> repo::Result<bool> {
    match ctx.pool() {
        Some(owner) if owner == user_id => at_limit(owner, Some(ctx.tenant_id)),
        Some(owner) => Ok(at_limit(user_id, None)? || at_limit(owner, Some(ctx.tenant_id))?),
        None => at_limit(user_id, None),
    }
}

fn at_limit_for_metric(
    conn: &Connection,
    ctx: QuotaContext,
    user_id: i64,
    metric: QuotaMetric,
) -> repo::Result<bool> {
    any_level_at_limit(ctx, user_id, |limit_user, tenant| {
        if metric == QuotaMetric::TeamMembers {
            team_member_limit_reached(conn, limit_user, tenant, 1)
        } else {
            limit_reached(conn, limit_user, metric)
        }
    })
}

/// `hasReachedLimit`: read-only gate (team members use the live distinct
/// count).
pub fn has_reached_limit(
    conn: &Connection,
    user_id: i64,
    metric: QuotaMetric,
) -> repo::Result<bool> {
    at_limit_for_metric(conn, resolve_context(conn, user_id)?, user_id, metric)
}

/// `hasRoomForTeamMembers`: `additional` more members fit at every level.
pub fn has_room_for_team_members(
    conn: &Connection,
    user_id: i64,
    additional: i64,
) -> repo::Result<bool> {
    let ctx = resolve_context(conn, user_id)?;
    Ok(!any_level_at_limit(ctx, user_id, |limit_user, tenant| {
        team_member_limit_reached(conn, limit_user, tenant, additional)
    })?)
}

/// `getAtLimitMap`: every metric's `hasReachedLimit`.
pub fn at_limit_map(conn: &Connection, user_id: i64) -> repo::Result<Vec<(QuotaMetric, bool)>> {
    let ctx = resolve_context(conn, user_id)?;
    QuotaMetric::ALL
        .iter()
        .map(|&m| Ok((m, at_limit_for_metric(conn, ctx, user_id, m)?)))
        .collect()
}

/// One metric of a usage summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricUsage {
    pub used: i64,
    pub limit: Option<i64>,
    /// `getWorkspaceUsageSummary` only.
    pub workspace_used: Option<i64>,
}

/// `getUsageSummary`: the reseller acting directly reads the pool row (team
/// members over the tenant); everyone else reads its own row (a
/// sub-account's pool is not disclosed). Team members are the live count.
pub fn usage_summary(
    conn: &Connection,
    user_id: i64,
) -> repo::Result<Vec<(QuotaMetric, MetricUsage)>> {
    let ctx = resolve_context(conn, user_id)?;
    let quota = user_quota::get_for_user(conn, user_id)?;
    let members = match ctx.pool() {
        Some(owner) if owner == user_id => {
            user_quota::count_distinct_team_members_for_tenant(conn, ctx.tenant_id)?
        }
        _ => user_quota::count_distinct_team_members(conn, user_id)?,
    };
    Ok(QuotaMetric::ALL
        .iter()
        .map(|&metric| {
            let (limit, used) = quota.as_ref().map_or((None, 0), |q| metric.values(q));
            let used = if metric == QuotaMetric::TeamMembers {
                members
            } else {
                used
            };
            (
                metric,
                MetricUsage {
                    used,
                    limit,
                    workspace_used: None,
                },
            )
        })
        .collect())
}

/// `getWorkspaceUsageSummary`: the account summary (without `workspaces`)
/// plus this workspace's display-only share. `mac` is the current
/// `WorkspaceMac` period; `monthlyBotMessages` reuses the lifetime count.
pub fn workspace_usage_summary(
    conn: &Connection,
    user_id: i64,
    workspace_id: i64,
) -> repo::Result<Vec<(QuotaMetric, MetricUsage)>> {
    let [contacts, channels, members, bot_messages]: [i64; 4] = conn
        .query_row(
            r#"SELECT "contactsUsed", "channelsUsed", "teamMembersUsed", "botMessagesUsed" FROM "WorkspaceUsage" WHERE "workspaceId" = ?1"#,
            [workspace_id],
            |row| Ok([row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?]),
        )
        .optional()?
        .unwrap_or([0; 4]);
    let mac: i64 = conn
        .query_row(
            r#"SELECT "macCount" FROM "WorkspaceMac" WHERE "workspaceId" = ?1 AND "periodStart" <= ?2 AND "periodEnd" > ?2 ORDER BY "id" DESC LIMIT 1"#,
            chatbotx_db::rusqlite::params![workspace_id, now_ms()],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);
    Ok(usage_summary(conn, user_id)?
        .into_iter()
        .filter_map(|(metric, entry)| {
            let share = match metric {
                QuotaMetric::Workspaces => return None,
                QuotaMetric::Contacts => contacts,
                QuotaMetric::Channels => channels,
                QuotaMetric::TeamMembers => members,
                QuotaMetric::Mac => mac,
                QuotaMetric::BotMessages | QuotaMetric::MonthlyBotMessages => bot_messages,
            };
            Some((
                metric,
                MetricUsage {
                    workspace_used: Some(share),
                    ..entry
                },
            ))
        })
        .collect())
}

/// The info-only `contacts` counters of a brand-new contact (user + pool,
/// and the workspace breakdown).
fn count_new_contact(
    conn: &Connection,
    ctx: QuotaContext,
    owner_id: i64,
    workspace_id: i64,
) -> repo::Result<()> {
    adjust_levels(conn, ctx, owner_id, QuotaMetric::Contacts, 1)?;
    workspace_usage::adjust(conn, workspace_id, WorkspaceUsageMetric::Contacts, 1)
}

/// `createNewContactWithMac`, run inside the caller's write transaction:
/// refuses (creating nothing) when MAC has no room; else runs `create` with
/// the owner's period anchor. `create` returns its value and whether the
/// MAC ledger claim counted the contact. MAC is consumed for a counted
/// contact, or for a period-less owner that has a finite MAC limit; the
/// `contacts` counters always move. A `create` error leaves the counters
/// untouched (the caller's transaction rolls back).
pub fn create_new_contact_with_mac<T>(
    conn: &Connection,
    owner_id: i64,
    workspace_id: i64,
    create: impl FnOnce(&Connection, Option<i64>) -> crate::Result<(T, bool)>,
) -> crate::Result<Result<T, ConsumeLevel>> {
    let ctx = resolve_context(conn, owner_id)?;
    let remaining = dual_remaining_for_ctx(conn, ctx, owner_id, QuotaMetric::Mac)?;
    if remaining == Some(0) {
        let level = match ctx.pool() {
            Some(owner) if limit_reached(conn, owner, QuotaMetric::Mac)? => ConsumeLevel::Pool,
            _ => ConsumeLevel::User,
        };
        return Ok(Err(level));
    }
    let period_start = user_quota::get_for_user(conn, owner_id)?.and_then(|q| q.period_start);
    let (value, counted) = create(conn, period_start)?;
    if counted || (period_start.is_none() && remaining.is_some()) {
        adjust_levels(conn, ctx, owner_id, QuotaMetric::Mac, 1)?;
    }
    if counted {
        workspace_usage::adjust(conn, workspace_id, WorkspaceUsageMetric::Mac, 1)?;
    }
    count_new_contact(conn, ctx, owner_id, workspace_id)?;
    Ok(Ok(value))
}

/// `createContactWithoutMac`: no gate, no MAC; only the `contacts` counters.
pub fn create_contact_without_mac<T>(
    conn: &Connection,
    owner_id: i64,
    workspace_id: i64,
    create: impl FnOnce(&Connection) -> crate::Result<T>,
) -> crate::Result<T> {
    let value = create(conn)?;
    let ctx = resolve_context(conn, owner_id)?;
    count_new_contact(conn, ctx, owner_id, workspace_id)?;
    Ok(value)
}
