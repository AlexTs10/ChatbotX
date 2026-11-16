//! `user-quota/service.ts` and `quota-enforcement/service.ts` (Q.D11): the
//! `UserQuota` row of a user (`getForUser`), `hasCapacity`, `tryConsume` and
//! `release` of one metric.
//!
//! The original keeps a Redis live counter next to the DB column and a
//! distributed lock around check-then-consume. Here the DB column is the only
//! counter and the check and the write run in one SQLite write transaction
//! (the single writer serializes them).

use chatbotx_db::repo::models::UserQuota;
use chatbotx_db::repo::{self, find_one, now_ms};
use chatbotx_db::rusqlite::Connection;

use crate::Result;

/// `QuotaMetric`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaMetric {
    Contacts,
    Workspaces,
    Channels,
    TeamMembers,
    Mac,
    BotMessages,
    MonthlyBotMessages,
}

impl QuotaMetric {
    /// `ALL_METRICS` (summary order).
    pub const ALL: [QuotaMetric; 7] = [
        QuotaMetric::Workspaces,
        QuotaMetric::Channels,
        QuotaMetric::TeamMembers,
        QuotaMetric::Contacts,
        QuotaMetric::Mac,
        QuotaMetric::BotMessages,
        QuotaMetric::MonthlyBotMessages,
    ];

    /// The TS metric name.
    pub fn as_str(self) -> &'static str {
        match self {
            QuotaMetric::Contacts => "contacts",
            QuotaMetric::Workspaces => "workspaces",
            QuotaMetric::Channels => "channels",
            QuotaMetric::TeamMembers => "teamMembers",
            QuotaMetric::Mac => "mac",
            QuotaMetric::BotMessages => "botMessages",
            QuotaMetric::MonthlyBotMessages => "monthlyBotMessages",
        }
    }

    /// The `(limit, used)` columns of the metric.
    pub fn columns(self) -> (&'static str, &'static str) {
        match self {
            QuotaMetric::Contacts => ("contactsLimit", "contactsUsed"),
            QuotaMetric::Workspaces => ("workspacesLimit", "workspacesUsed"),
            QuotaMetric::Channels => ("channelsLimit", "channelsUsed"),
            QuotaMetric::TeamMembers => ("teamMembersLimit", "teamMembersUsed"),
            QuotaMetric::Mac => ("macLimit", "macUsed"),
            QuotaMetric::BotMessages => ("botMessagesLimit", "botMessagesUsed"),
            QuotaMetric::MonthlyBotMessages => {
                ("monthlyBotMessagesLimit", "monthlyBotMessagesUsed")
            }
        }
    }

    /// `readMetricValues`: `(limit, used)` of a row.
    pub fn values(self, quota: &UserQuota) -> (Option<i64>, i64) {
        match self {
            QuotaMetric::Contacts => (quota.contacts_limit, quota.contacts_used),
            QuotaMetric::Workspaces => (quota.workspaces_limit, quota.workspaces_used),
            QuotaMetric::Channels => (quota.channels_limit, quota.channels_used),
            QuotaMetric::TeamMembers => (quota.team_members_limit, quota.team_members_used),
            QuotaMetric::Mac => (quota.mac_limit, quota.mac_used),
            QuotaMetric::BotMessages => (quota.bot_messages_limit, quota.bot_messages_used),
            QuotaMetric::MonthlyBotMessages => (
                quota.monthly_bot_messages_limit,
                quota.monthly_bot_messages_used,
            ),
        }
    }
}

/// `getForUser`: the user's quota row.
pub fn get_for_user(conn: &Connection, user_id: i64) -> repo::Result<Option<UserQuota>> {
    find_one::<UserQuota>(conn, "\"userId\" = ?1", &[&user_id])
}

/// `hasCapacity`: no row = unlimited; else `limit === null || used < limit`.
pub fn has_capacity(conn: &Connection, user_id: i64, metric: QuotaMetric) -> repo::Result<bool> {
    Ok(match get_for_user(conn, user_id)? {
        None => true,
        Some(quota) => {
            let (limit, used) = metric.values(&quota);
            limit.is_none_or(|limit| used < limit)
        }
    })
}

/// `consume` (`upsertMetricBy`, `delta > 0`: creates a missing row) /
/// `releaseBy` (`delta < 0`: never below 0, a missing row stays missing).
pub fn adjust(
    conn: &Connection,
    user_id: i64,
    metric: QuotaMetric,
    delta: i64,
) -> repo::Result<()> {
    let (_, used) = metric.columns();
    let now = now_ms();
    if delta > 0 {
        conn.execute(
            &format!(
                r#"INSERT INTO "UserQuota" ("id", "userId", "{used}", "syncedAt") VALUES (?1, ?2, ?3, ?4)
                   ON CONFLICT ("userId") DO UPDATE SET "{used}" = "UserQuota"."{used}" + ?3, "updatedAt" = ?4"#
            ),
            chatbotx_db::rusqlite::params![repo::new_id(), user_id, delta, now],
        )?;
    } else if delta < 0 {
        conn.execute(
            &format!(
                r#"UPDATE "UserQuota" SET "{used}" = MAX(0, "{used}" + ?2), "updatedAt" = ?3 WHERE "userId" = ?1"#
            ),
            chatbotx_db::rusqlite::params![user_id, delta, now],
        )?;
    }
    Ok(())
}

/// `tryConsume` (user level): `false` when the metric is at its limit, else
/// one unit is consumed.
pub fn try_consume(conn: &Connection, user_id: i64, metric: QuotaMetric) -> repo::Result<bool> {
    if !has_capacity(conn, user_id, metric)? {
        return Ok(false);
    }
    adjust(conn, user_id, metric, 1)?;
    Ok(true)
}

/// `release`: hands one unit back.
pub fn release(conn: &Connection, user_id: i64, metric: QuotaMetric) -> repo::Result<()> {
    adjust(conn, user_id, metric, -1)
}

/// [`get_for_user`] on a reader connection.
pub async fn find_for_user(db: &chatbotx_db::Db, user_id: i64) -> Result<Option<UserQuota>> {
    crate::tx::read(db, move |conn| Ok(get_for_user(conn, user_id)?)).await
}

/// `countDistinctTeamMembers({ ownerId })`: distinct member users over the
/// owner's workspaces.
pub fn count_distinct_team_members(conn: &Connection, owner_id: i64) -> repo::Result<i64> {
    Ok(conn.query_row(
        r#"SELECT COUNT(DISTINCT m."userId") FROM "WorkspaceMember" m
           JOIN "Workspace" w ON w."id" = m."workspaceId" WHERE w."ownerId" = ?1"#,
        [owner_id],
        |row| row.get(0),
    )?)
}

/// `countDistinctTeamMembers({ tenantId })`: distinct member users over the
/// tenant's workspaces (a reseller pool).
pub fn count_distinct_team_members_for_tenant(
    conn: &Connection,
    tenant_id: i64,
) -> repo::Result<i64> {
    Ok(conn.query_row(
        r#"SELECT COUNT(DISTINCT m."userId") FROM "WorkspaceMember" m
           JOIN "Workspace" w ON w."id" = m."workspaceId" WHERE w."tenantId" = ?1"#,
        [tenant_id],
        |row| row.get(0),
    )?)
}

/// `quotaEnforcementService.hasReachedLimit({ metric: "teamMembers" })` for
/// one tenant (no reseller pool): no quota row = never; else
/// `limit !== null && distinctMembers + 1 > limit`.
pub fn team_member_limit_reached(conn: &Connection, owner_id: i64) -> repo::Result<bool> {
    let Some(quota) = get_for_user(conn, owner_id)? else {
        return Ok(false);
    };
    let (limit, _) = QuotaMetric::TeamMembers.values(&quota);
    Ok(match limit {
        None => false,
        Some(limit) => count_distinct_team_members(conn, owner_id)? + 1 > limit,
    })
}

// --- plans and access state (Q.D11 slice 6) ---

/// `DEFAULT_PLAN_ENTITLEMENT_KEY` (written by the license sync).
pub const DEFAULT_PLAN_ENTITLEMENT_KEY: &str = "entitlements:default-plan";

/// `planStatuses` values the access check knows.
pub const PLAN_STATUS_TRIAL: &str = "trial";
pub const PLAN_STATUS_ACTIVE: &str = "active";

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// `DefaultPlanSnapshot` (a JSON object in the durable kv store).
#[derive(Debug, Clone, PartialEq)]
pub struct DefaultPlanSnapshot {
    pub plan_name: String,
    pub trial_days: Option<f64>,
    pub workspaces_limit: Option<i64>,
    pub mac_limit: Option<i64>,
    pub channels_limit: Option<i64>,
    pub team_members_limit: Option<i64>,
    pub contacts_limit: Option<i64>,
    pub bot_messages_limit: Option<i64>,
    pub monthly_bot_messages_limit: Option<i64>,
    pub white_label: bool,
    pub sso_saml: bool,
    pub saas_mode: bool,
    pub custom_app_credentials: bool,
}

impl DefaultPlanSnapshot {
    /// Reads the stored JSON (`None` when it is not an object).
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        let object = value.as_object()?;
        let int = |k: &str| object.get(k).and_then(serde_json::Value::as_i64);
        let flag = |k: &str| object.get(k) == Some(&serde_json::Value::Bool(true));
        Some(Self {
            plan_name: object
                .get("planName")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            trial_days: object.get("trialDays").and_then(serde_json::Value::as_f64),
            workspaces_limit: int("workspacesLimit"),
            mac_limit: int("macLimit"),
            channels_limit: int("channelsLimit"),
            team_members_limit: int("teamMembersLimit"),
            contacts_limit: int("contactsLimit"),
            bot_messages_limit: int("botMessagesLimit"),
            monthly_bot_messages_limit: int("monthlyBotMessagesLimit"),
            white_label: flag("whiteLabel"),
            sso_saml: flag("ssoSaml"),
            saas_mode: flag("saasMode"),
            custom_app_credentials: flag("customAppCredentials"),
        })
    }

    /// `BOOTSTRAP_TRIAL_FALLBACK`: a one-day trial with every limit 0.
    pub fn bootstrap_fallback() -> Self {
        Self {
            plan_name: "Trial".into(),
            trial_days: Some(1.0),
            workspaces_limit: Some(0),
            mac_limit: Some(0),
            channels_limit: Some(0),
            team_members_limit: Some(0),
            contacts_limit: Some(0),
            bot_messages_limit: Some(0),
            monthly_bot_messages_limit: Some(0),
            white_label: false,
            sso_saml: false,
            saas_mode: false,
            custom_app_credentials: false,
        }
    }
}

/// The kv key of a tenant's default plan (root = the global key).
pub fn default_plan_key(tenant_id: Option<i64>) -> String {
    match tenant_id {
        Some(id) if id != chatbotx_db::repo::platform_admin::ROOT_TENANT_ID => {
            format!("{DEFAULT_PLAN_ENTITLEMENT_KEY}:{id}")
        }
        _ => DEFAULT_PLAN_ENTITLEMENT_KEY.to_string(),
    }
}

/// `readDefaultPlanSnapshot`: a read failure is logged and reads as none.
pub async fn read_default_plan_snapshot(
    kv: Option<&chatbotx_kv::durable::DurableKv>,
    tenant_id: Option<i64>,
) -> Option<DefaultPlanSnapshot> {
    let kv = kv?;
    match kv
        .get::<serde_json::Value>(&default_plan_key(tenant_id))
        .await
    {
        Ok(value) => value.as_ref().and_then(DefaultPlanSnapshot::from_json),
        Err(err) => {
            tracing::warn!(err = %err, "user-quota: default-plan snapshot read failed");
            None
        }
    }
}

/// `ensureBootstrapPlan` (cloud only): stamps the user's first quota row
/// from the tenant's default plan (or the lockdown fallback); an existing
/// row is kept. Returns `true` when a row was inserted.
pub async fn ensure_bootstrap_plan(
    db: &chatbotx_db::Db,
    kv: Option<&chatbotx_kv::durable::DurableKv>,
    edition: crate::keys::Edition,
    user_id: i64,
    tenant_id: Option<i64>,
) -> Result<bool> {
    if !edition.is_cloud() {
        return Ok(false);
    }
    let snapshot = read_default_plan_snapshot(kv, tenant_id)
        .await
        .unwrap_or_else(DefaultPlanSnapshot::bootstrap_fallback);
    let fallback_days = DefaultPlanSnapshot::bootstrap_fallback()
        .trial_days
        .unwrap_or(1.0);
    let trial_days = match snapshot.trial_days {
        Some(days) if days.is_finite() => days.max(0.0),
        _ => fallback_days,
    };
    let now = now_ms();
    let is_trial = trial_days > 0.0;
    let period_end = is_trial.then(|| now + (trial_days * DAY_MS as f64) as i64);
    let status = if is_trial {
        PLAN_STATUS_TRIAL
    } else {
        PLAN_STATUS_ACTIVE
    };
    crate::tx::write(db, move |conn| {
        let inserted = conn.execute(
            r#"INSERT INTO "UserQuota" ("id", "userId", "contactsLimit", "workspacesLimit", "channelsLimit", "teamMembersLimit", "macLimit", "botMessagesLimit", "monthlyBotMessagesLimit", "whiteLabel", "ssoSaml", "saasMode", "customAppCredentials", "planName", "planStatus", "periodStart", "periodEnd", "syncedAt", "createdAt", "updatedAt")
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0, 0, 0, ?10, ?11, ?12, ?13, ?12, ?12, ?12)
               ON CONFLICT ("userId") DO NOTHING"#,
            chatbotx_db::rusqlite::params![
                repo::new_id(),
                user_id,
                snapshot.contacts_limit,
                snapshot.workspaces_limit,
                snapshot.channels_limit,
                snapshot.team_members_limit,
                snapshot.mac_limit,
                snapshot.bot_messages_limit,
                snapshot.monthly_bot_messages_limit,
                snapshot.plan_name,
                status,
                now,
                period_end,
            ],
        )?;
        Ok(inserted > 0)
    })
    .await
}

/// `applyDefaultPlan`: the effective row of a user with no plan (no row, or
/// `planStatus` null) under the default plan; stored limits win.
pub fn apply_default_plan(
    user_id: i64,
    quota: Option<UserQuota>,
    snapshot: &DefaultPlanSnapshot,
) -> UserQuota {
    let now = now_ms();
    let base = quota.unwrap_or(UserQuota {
        id: 0,
        created_at: now,
        updated_at: now,
        user_id,
        contacts_limit: None,
        contacts_used: 0,
        workspaces_limit: None,
        workspaces_used: 0,
        channels_limit: None,
        channels_used: 0,
        team_members_limit: None,
        team_members_used: 0,
        mac_limit: None,
        mac_used: 0,
        bot_messages_limit: None,
        bot_messages_used: 0,
        monthly_bot_messages_limit: None,
        monthly_bot_messages_used: 0,
        monthly_bot_messages_period_start: None,
        bot_messages_top_up_granted: 0,
        white_label: false,
        sso_saml: false,
        saas_mode: false,
        custom_app_credentials: false,
        plan_name: None,
        plan_status: None,
        selected_trial_plan_id: None,
        period_start: None,
        period_end: None,
        channels_torn_down_at: None,
        synced_at: now,
    });
    UserQuota {
        contacts_limit: base.contacts_limit.or(snapshot.contacts_limit),
        workspaces_limit: base.workspaces_limit.or(snapshot.workspaces_limit),
        channels_limit: base.channels_limit.or(snapshot.channels_limit),
        bot_messages_limit: base.bot_messages_limit.or(snapshot.bot_messages_limit),
        monthly_bot_messages_limit: base
            .monthly_bot_messages_limit
            .or(snapshot.monthly_bot_messages_limit),
        team_members_limit: base.team_members_limit.or(snapshot.team_members_limit),
        mac_limit: base.mac_limit.or(snapshot.mac_limit),
        white_label: base.white_label || snapshot.white_label,
        sso_saml: base.sso_saml || snapshot.sso_saml,
        saas_mode: base.saas_mode || snapshot.saas_mode,
        custom_app_credentials: base.custom_app_credentials || snapshot.custom_app_credentials,
        plan_name: base.plan_name.clone().or(Some(snapshot.plan_name.clone())),
        plan_status: base
            .plan_status
            .clone()
            .or(Some(PLAN_STATUS_ACTIVE.to_string())),
        ..base
    }
}

/// `getForUser` with the default plan (cloud): a user with no plan gets the
/// tenant's default plan; without a published snapshot the stored row (or
/// none) is returned.
pub async fn get_effective_for_user(
    db: &chatbotx_db::Db,
    kv: Option<&chatbotx_kv::durable::DurableKv>,
    edition: crate::keys::Edition,
    user_id: i64,
) -> Result<Option<UserQuota>> {
    let (quota, tenant_id) = crate::tx::read(db, move |conn| {
        use chatbotx_db::rusqlite::OptionalExtension as _;
        let quota = get_for_user(conn, user_id)?;
        let tenant_id: Option<i64> = conn
            .query_row(
                r#"SELECT "tenantId" FROM "User" WHERE "id" = ?1"#,
                [user_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(repo::Error::from)?;
        Ok((quota, tenant_id))
    })
    .await?;
    let needs_plan = quota.as_ref().is_none_or(|q| q.plan_status.is_none());
    if needs_plan && edition.is_cloud() {
        if let Some(snapshot) = read_default_plan_snapshot(kv, tenant_id).await {
            return Ok(Some(apply_default_plan(user_id, quota, &snapshot)));
        }
    }
    Ok(quota)
}

/// `AccessState`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessState {
    pub blocked: bool,
    pub status: Option<String>,
    pub plan_name: Option<String>,
    /// ms; set for a trial only.
    pub trial_ends_at: Option<i64>,
    /// `"status"` / `"mac"`.
    pub reason: Option<&'static str>,
}

/// `getAccessStateFromQuota`: an active plan or an unexpired trial is
/// allowed; the DB mac column at its limit blocks; a status block wins.
pub fn access_state_from_quota(quota: Option<&UserQuota>, now_ms: i64) -> AccessState {
    let Some(quota) = quota else {
        return AccessState {
            blocked: false,
            status: None,
            plan_name: None,
            trial_ends_at: None,
            reason: None,
        };
    };
    let status = quota.plan_status.as_deref();
    let is_trial = status == Some(PLAN_STATUS_TRIAL);
    let trial_expired = is_trial && quota.period_end.is_some_and(|end| end <= now_ms);
    let status_allowed = status == Some(PLAN_STATUS_ACTIVE) || (is_trial && !trial_expired);
    let mac_reached = quota.mac_limit.is_some_and(|limit| quota.mac_used >= limit);
    let reason = if !status_allowed {
        Some("status")
    } else if mac_reached {
        Some("mac")
    } else {
        None
    };
    AccessState {
        blocked: reason.is_some(),
        status: quota.plan_status.clone(),
        plan_name: quota.plan_name.clone(),
        trial_ends_at: if is_trial { quota.period_end } else { None },
        reason,
    }
}

/// `getAccessState`: the effective row's state. (The live mac counter of the
/// original is the DB column here, so the second check is the same column.)
pub async fn get_access_state(
    db: &chatbotx_db::Db,
    kv: Option<&chatbotx_kv::durable::DurableKv>,
    edition: crate::keys::Edition,
    user_id: i64,
) -> Result<AccessState> {
    let quota = get_effective_for_user(db, kv, edition, user_id).await?;
    Ok(access_state_from_quota(quota.as_ref(), now_ms()))
}

/// `getPlanIdentity`: the raw row only (no default plan).
pub async fn get_plan_identity(
    db: &chatbotx_db::Db,
    user_id: i64,
) -> Result<(bool, Option<String>)> {
    let quota = find_for_user(db, user_id).await?;
    Ok(match quota {
        Some(q) => (
            q.plan_status.as_deref() == Some(PLAN_STATUS_TRIAL),
            q.plan_name,
        ),
        None => (false, None),
    })
}

/// `TRIAL_TEARDOWN_GRACE_MS`: seven days after a trial ends.
pub const TRIAL_TEARDOWN_GRACE_MS: i64 = 7 * DAY_MS;

/// `listDueExpiredTrials`: trial rows ended at or before `cutoff` whose
/// channels are not torn down, by user id after `cursor`; `next_cursor` only
/// for a full page.
pub async fn list_due_expired_trials(
    db: &chatbotx_db::Db,
    cutoff_ms: i64,
    cursor: Option<i64>,
    limit: i64,
) -> Result<(Vec<i64>, Option<i64>)> {
    crate::tx::read(db, move |conn| {
        let mut stmt = conn.prepare(
            r#"SELECT "userId" FROM "UserQuota" WHERE "planStatus" = 'trial' AND "periodEnd" IS NOT NULL AND "periodEnd" <= ?1 AND "channelsTornDownAt" IS NULL AND (?2 IS NULL OR "userId" > ?2) ORDER BY "userId" LIMIT ?3"#,
        )?;
        let ids = stmt
            .query_map(chatbotx_db::rusqlite::params![cutoff_ms, cursor, limit], |r| {
                r.get::<_, i64>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let next = (ids.len() as i64 == limit).then(|| ids.last().copied()).flatten();
        Ok((ids, next))
    })
    .await
}

/// `markChannelsTornDown`.
pub async fn mark_channels_torn_down(db: &chatbotx_db::Db, user_id: i64) -> Result<()> {
    crate::tx::write(db, move |conn| {
        conn.execute(
            r#"UPDATE "UserQuota" SET "channelsTornDownAt" = ?2 WHERE "userId" = ?1"#,
            chatbotx_db::rusqlite::params![user_id, now_ms()],
        )?;
        Ok(())
    })
    .await
}

/// `clearWhiteLabelEntitlements`.
pub async fn clear_white_label_entitlements(db: &chatbotx_db::Db, user_id: i64) -> Result<()> {
    crate::tx::write(db, move |conn| {
        conn.execute(
            r#"UPDATE "UserQuota" SET "whiteLabel" = 0, "ssoSaml" = 0, "saasMode" = 0, "updatedAt" = ?2 WHERE "userId" = ?1"#,
            chatbotx_db::rusqlite::params![user_id, now_ms()],
        )?;
        Ok(())
    })
    .await
}

/// `hasWhiteLabelEntitlement`.
pub async fn has_white_label_entitlement(db: &chatbotx_db::Db, user_id: i64) -> Result<bool> {
    Ok(find_for_user(db, user_id)
        .await?
        .is_some_and(|q| q.white_label))
}

/// `listWhiteLabelOwnerIds`.
pub async fn list_white_label_owner_ids(db: &chatbotx_db::Db) -> Result<Vec<i64>> {
    crate::tx::read(db, |conn| {
        let mut stmt = conn.prepare(
            r#"SELECT "userId" FROM "UserQuota" WHERE "whiteLabel" = 1 ORDER BY "userId""#,
        )?;
        let ids = stmt
            .query_map([], |r| r.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    })
    .await
}

// --- usage reconcile (Q.D11 slice 6) ---

/// The workspace scope of a usage recount.
#[derive(Debug, Clone, Copy)]
enum UsageScope {
    Owner(i64),
    Tenant(i64),
}

impl UsageScope {
    fn filter(self) -> (&'static str, i64) {
        match self {
            Self::Owner(id) => (r#"w."ownerId" = ?1"#, id),
            Self::Tenant(id) => (r#"w."tenantId" = ?1"#, id),
        }
    }
}

/// `countWorkspaceScopedUsage` + `countDistinctTeamMembers`:
/// `(contacts, workspaces, channels, teamMembers)`. Channels are distinct
/// active channel `Connection` rows, not `Inbox` rows.
fn count_scoped_usage(conn: &Connection, scope: UsageScope) -> repo::Result<[i64; 4]> {
    let (filter, id) = scope.filter();
    let active = repo::connection::ACTIVE_CONNECTION_STATUSES
        .iter()
        .map(|s| format!("'{s}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let queries = [
        format!(
            r#"SELECT COUNT(*) FROM "Contact" c JOIN "Workspace" w ON w."id" = c."workspaceId" WHERE {filter}"#
        ),
        format!(r#"SELECT COUNT(*) FROM "Workspace" w WHERE {filter}"#),
        format!(
            r#"SELECT COUNT(DISTINCT k."id") FROM "Connection" k JOIN "Workspace" w ON w."id" = k."workspaceId" WHERE {filter} AND k."kind" = 'channel' AND k."status" IN ({active})"#
        ),
        format!(
            r#"SELECT COUNT(DISTINCT m."userId") FROM "WorkspaceMember" m JOIN "Workspace" w ON w."id" = m."workspaceId" WHERE {filter}"#
        ),
    ];
    let mut out = [0; 4];
    for (slot, sql) in out.iter_mut().zip(queries) {
        *slot = conn.query_row(&sql, [id], |row| row.get(0))?;
    }
    Ok(out)
}

/// The markers `reconcileUserSelfUsage` reads back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelfUsageMarkers {
    pub mac_used: i64,
    pub period_start: Option<i64>,
    pub period_end: Option<i64>,
    pub monthly_bot_messages_period_start: Option<i64>,
}

/// `reconcileUserSelfUsage`: assigns the owner-scoped counts directly (so
/// deletions free quota; never `MAX`) and returns the stored mac markers.
/// There is no live Redis counter to mirror (DECISIONS).
pub async fn reconcile_user_self_usage(
    db: &chatbotx_db::Db,
    user_id: i64,
) -> Result<SelfUsageMarkers> {
    crate::tx::write(db, move |conn| {
        let [contacts, workspaces, channels, members] =
            count_scoped_usage(conn, UsageScope::Owner(user_id))?;
        let now = now_ms();
        conn.execute(
            r#"INSERT INTO "UserQuota" ("id", "userId", "contactsUsed", "teamMembersUsed", "workspacesUsed", "channelsUsed", "syncedAt")
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
               ON CONFLICT ("userId") DO UPDATE SET "contactsUsed" = excluded."contactsUsed", "teamMembersUsed" = excluded."teamMembersUsed", "workspacesUsed" = excluded."workspacesUsed", "channelsUsed" = excluded."channelsUsed", "syncedAt" = excluded."syncedAt", "updatedAt" = ?7"#,
            chatbotx_db::rusqlite::params![repo::new_id(), user_id, contacts, members, workspaces, channels, now],
        )?;
        let quota = get_for_user(conn, user_id)?;
        Ok(quota.map_or_else(SelfUsageMarkers::default, |q| SelfUsageMarkers {
            mac_used: q.mac_used,
            period_start: q.period_start,
            period_end: q.period_end,
            monthly_bot_messages_period_start: q.monthly_bot_messages_period_start,
        }))
    })
    .await
}

/// `reconcileOwnerPoolUsage`: a reseller owner's row is the pool of every
/// workspace of the tenant. Counts are assigned directly; `macUsed` (the
/// tenant's current-period sum) only rises, and only while the row is still
/// in the period read first (a rollover in between keeps its value).
pub async fn reconcile_owner_pool_usage(
    db: &chatbotx_db::Db,
    owner_id: i64,
    tenant_id: i64,
) -> Result<()> {
    crate::tx::write(db, move |conn| {
        let period_start = get_for_user(conn, owner_id)?.and_then(|q| q.period_start);
        let [contacts, workspaces, channels, members] =
            count_scoped_usage(conn, UsageScope::Tenant(tenant_id))?;
        let now = now_ms();
        let mac = repo::small_repos::mac_sum_for_tenant(conn, tenant_id, now)?;
        conn.execute(
            r#"INSERT INTO "UserQuota" ("id", "userId", "contactsUsed", "teamMembersUsed", "workspacesUsed", "channelsUsed", "macUsed", "periodStart", "syncedAt")
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
               ON CONFLICT ("userId") DO UPDATE SET "contactsUsed" = excluded."contactsUsed", "teamMembersUsed" = excluded."teamMembersUsed", "workspacesUsed" = excluded."workspacesUsed", "channelsUsed" = excluded."channelsUsed",
                 "macUsed" = CASE WHEN "UserQuota"."periodStart" IS ?8 THEN MAX("UserQuota"."macUsed", ?7) ELSE "UserQuota"."macUsed" END,
                 "syncedAt" = ?9, "updatedAt" = ?9"#,
            chatbotx_db::rusqlite::params![repo::new_id(), owner_id, contacts, members, workspaces, channels, mac, period_start, now],
        )?;
        Ok(())
    })
    .await
}

/// `reconcileOwnerAfterTeardown`: the owner's own usage, then (for an active
/// reseller tenant) the tenant pool. A root tenant id looks the tenant up by
/// owner.
pub async fn reconcile_owner_after_teardown(
    db: &chatbotx_db::Db,
    owner_id: i64,
    tenant_id: Option<i64>,
) -> Result<()> {
    reconcile_user_self_usage(db, owner_id).await?;
    let tenant = crate::tx::read(db, move |conn| {
        Ok(match tenant_id {
            Some(id) if id != chatbotx_db::repo::platform_admin::ROOT_TENANT_ID => {
                repo::tenant::find_by_id(conn, id)?
            }
            _ => repo::tenant::find_by_owner(conn, owner_id)?,
        })
    })
    .await?;
    if let Some(tenant) = tenant.filter(|t| t.status == "active") {
        if let Some(owner) = tenant.owner_id {
            reconcile_owner_pool_usage(db, owner, tenant.id).await?;
        }
    }
    Ok(())
}

/// `raiseMacUsed`: a monotonic `macUsed` raise pinned to `period_start` (a
/// row in another period is left as is).
pub async fn raise_mac_used(
    db: &chatbotx_db::Db,
    user_id: i64,
    value: i64,
    period_start: i64,
) -> Result<()> {
    crate::tx::write(db, move |conn| {
        let now = now_ms();
        conn.execute(
            r#"INSERT INTO "UserQuota" ("id", "userId", "macUsed", "periodStart", "syncedAt") VALUES (?1, ?2, ?3, ?4, ?5)
               ON CONFLICT ("userId") DO UPDATE SET "macUsed" = MAX("UserQuota"."macUsed", ?3), "updatedAt" = ?5
               WHERE "UserQuota"."periodStart" IS ?4"#,
            chatbotx_db::rusqlite::params![repo::new_id(), user_id, value, period_start, now],
        )?;
        Ok(())
    })
    .await
}

/// `applyMonthlyBotMessagesReset`: `reset` zeroes the monthly counter and
/// stamps the period; otherwise only the period is stamped.
pub async fn apply_monthly_bot_messages_reset(
    db: &chatbotx_db::Db,
    user_id: i64,
    period_start: Option<i64>,
    reset: bool,
) -> Result<()> {
    crate::tx::write(db, move |conn| {
        let now = now_ms();
        let sql = if reset {
            r#"INSERT INTO "UserQuota" ("id", "userId", "monthlyBotMessagesUsed", "monthlyBotMessagesPeriodStart", "syncedAt") VALUES (?1, ?2, 0, ?3, ?4)
               ON CONFLICT ("userId") DO UPDATE SET "monthlyBotMessagesUsed" = 0, "monthlyBotMessagesPeriodStart" = ?3, "updatedAt" = ?4"#
        } else {
            r#"INSERT INTO "UserQuota" ("id", "userId", "monthlyBotMessagesPeriodStart", "syncedAt") VALUES (?1, ?2, ?3, ?4)
               ON CONFLICT ("userId") DO UPDATE SET "monthlyBotMessagesPeriodStart" = ?3, "updatedAt" = ?4"#
        };
        conn.execute(
            sql,
            chatbotx_db::rusqlite::params![repo::new_id(), user_id, period_start, now],
        )?;
        Ok(())
    })
    .await
}
