//! Ported from `packages/business/__tests__/user-quota-access-state.test.ts`,
//! `user-quota-bootstrap-plan.test.ts`, `user-quota-default-plan.test.ts`,
//! `user-quota-plan-identity.test.ts` and `user-quota-due-expired-trials.test.ts`
//! (Q.D11 slice 6). The TS tests mock the drizzle client and Redis; here the
//! rows are real SQLite rows and the default-plan snapshot sits in a real
//! durable kv store. The live Redis mac counter is the DB column (DECISIONS).

mod common;

use std::sync::Arc;

use chatbotx_business::keys::Edition;
use chatbotx_business::user_quota::{
    self, DEFAULT_PLAN_ENTITLEMENT_KEY, TRIAL_TEARDOWN_GRACE_MS, access_state_from_quota,
};
use chatbotx_db::Db;
use chatbotx_db::repo::models::UserQuota;
use chatbotx_db::rusqlite::params;
use chatbotx_kv::SystemClock;
use chatbotx_kv::durable::DurableKv;
use common::{fresh_db, setup};
use serde_json::{Value, json};

const USER: i64 = 11;
const NOW: i64 = 1_800_000_000_000;
const DAY: i64 = 86_400_000;

fn quota(status: Option<&str>) -> UserQuota {
    UserQuota {
        id: 1,
        created_at: NOW,
        updated_at: NOW,
        user_id: USER,
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
        plan_name: Some("Pro".into()),
        plan_status: status.map(String::from),
        selected_trial_plan_id: None,
        period_start: Some(NOW - 30 * DAY),
        period_end: None,
        channels_torn_down_at: None,
        synced_at: NOW,
    }
}

fn reason(q: &UserQuota) -> (bool, Option<&'static str>) {
    let state = access_state_from_quota(Some(q), NOW);
    (state.blocked, state.reason)
}

// --- getAccessStateFromQuota ---

#[test]
fn no_quota_row_is_never_blocked() {
    let state = access_state_from_quota(None, NOW);
    assert_eq!(
        (
            state.blocked,
            state.status,
            state.plan_name,
            state.trial_ends_at,
            state.reason
        ),
        (false, None, None, None, None)
    );
}

#[test]
fn active_plan_is_allowed() {
    assert_eq!(reason(&quota(Some("active"))), (false, None));
}

#[test]
fn non_expired_trial_is_allowed() {
    let mut q = quota(Some("trial"));
    q.period_end = Some(NOW + DAY);
    assert_eq!(reason(&q), (false, None));
    assert_eq!(
        access_state_from_quota(Some(&q), NOW).trial_ends_at,
        Some(NOW + DAY)
    );
}

#[test]
fn expired_trial_blocks_with_reason_status() {
    let mut q = quota(Some("trial"));
    q.period_end = Some(NOW - DAY);
    assert_eq!(reason(&q), (true, Some("status")));
}

#[test]
fn expired_past_due_and_unknown_statuses_block_with_reason_status() {
    for status in ["expired", "past_due", "some_future_status"] {
        assert_eq!(
            reason(&quota(Some(status))),
            (true, Some("status")),
            "{status}"
        );
    }
}

#[test]
fn active_plan_at_the_db_mac_limit_blocks_with_reason_mac() {
    let mut q = quota(Some("active"));
    (q.mac_limit, q.mac_used) = (Some(100), 100);
    assert_eq!(reason(&q), (true, Some("mac")));
}

#[test]
fn active_plan_under_the_db_mac_limit_is_allowed() {
    let mut q = quota(Some("active"));
    (q.mac_limit, q.mac_used) = (Some(100), 99);
    assert_eq!(reason(&q), (false, None));
}

#[test]
fn status_block_takes_precedence_over_a_mac_block() {
    let mut q = quota(Some("past_due"));
    (q.mac_limit, q.mac_used) = (Some(100), 100);
    assert_eq!(reason(&q), (true, Some("status")));
}

#[test]
fn unlimited_mac_never_mac_blocks() {
    let mut q = quota(Some("active"));
    q.mac_used = 999_999;
    assert_eq!(reason(&q), (false, None));
}

// --- DB fixtures ---

async fn db_with_user(tenant: Option<i64>) -> common::TestDb {
    let db = fresh_db().await;
    setup(&db, move |tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "email") VALUES (?1, 'u@example.test')"#,
            [USER],
        )?;
        if let Some(tenant) = tenant {
            tx.execute(
                r#"INSERT INTO "Tenant" ("id", "ownerId") VALUES (?1, ?2)"#,
                params![tenant, USER],
            )?;
            tx.execute(
                r#"UPDATE "User" SET "tenantId" = ?1 WHERE "id" = ?2"#,
                params![tenant, USER],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

async fn insert_quota(db: &Db, sql: &'static str) {
    db.write(move |tx| {
        tx.execute(sql, [USER])?;
        Ok(())
    })
    .await
    .expect("quota row");
}

/// A durable kv store holding `entries`.
async fn kv(entries: Vec<(String, Value)>) -> DurableKv {
    let db = Db::open_in_memory().expect("kv db");
    db.migrate().await.expect("kv migrate");
    let kv = DurableKv::new(Arc::new(db), Arc::new(SystemClock));
    for (key, value) in entries {
        kv.put(&key, &value, None).await.expect("kv put");
    }
    kv
}

fn snapshot() -> Value {
    json!({
        "channelsLimit": 2, "contactsLimit": 1000, "macLimit": 100, "planName": "Free",
        "saasMode": true, "ssoSaml": false, "teamMembersLimit": 3, "trialDays": 14,
        "whiteLabel": true, "workspacesLimit": 4,
    })
}

fn global(value: Value) -> Vec<(String, Value)> {
    vec![(DEFAULT_PLAN_ENTITLEMENT_KEY.to_string(), value)]
}

async fn stored(db: &Db) -> UserQuota {
    user_quota::find_for_user(db, USER)
        .await
        .expect("read")
        .expect("quota row")
}

// --- getAccessState ---

#[tokio::test]
async fn blocked_by_status_is_reported_from_the_row() {
    let db = db_with_user(None).await;
    insert_quota(
        &db,
        r#"INSERT INTO "UserQuota" ("id", "userId", "planStatus") VALUES (1, ?1, 'past_due')"#,
    )
    .await;
    let state = user_quota::get_access_state(&db, None, Edition::Cloud, USER)
        .await
        .expect("state");
    assert_eq!((state.blocked, state.reason), (true, Some("status")));
}

#[tokio::test]
async fn blocks_when_the_mac_count_is_at_the_limit() {
    // Adapted: the live mac counter is the DB column (no Redis counter).
    let db = db_with_user(None).await;
    insert_quota(&db, r#"INSERT INTO "UserQuota" ("id", "userId", "planStatus", "macLimit", "macUsed") VALUES (1, ?1, 'active', 100, 100)"#).await;
    let state = user_quota::get_access_state(&db, None, Edition::Cloud, USER)
        .await
        .expect("state");
    assert_eq!((state.blocked, state.reason), (true, Some("mac")));
}

#[tokio::test]
async fn allows_when_the_mac_count_is_under_the_limit() {
    let db = db_with_user(None).await;
    insert_quota(&db, r#"INSERT INTO "UserQuota" ("id", "userId", "planStatus", "macLimit", "macUsed") VALUES (1, ?1, 'active', 100, 50)"#).await;
    let state = user_quota::get_access_state(&db, None, Edition::Cloud, USER)
        .await
        .expect("state");
    assert_eq!((state.blocked, state.reason), (false, None));
}

// --- ensureBootstrapPlan ---

#[tokio::test]
async fn stamps_a_trial_row_from_the_default_plan_snapshot() {
    let db = db_with_user(None).await;
    let kv = kv(global(snapshot())).await;
    let before = chatbotx_db::repo::now_ms();
    let inserted = user_quota::ensure_bootstrap_plan(&db, Some(&kv), Edition::Cloud, USER, None)
        .await
        .expect("bootstrap");
    assert!(inserted);
    let q = stored(&db).await;
    assert_eq!(
        (
            q.contacts_limit,
            q.workspaces_limit,
            q.channels_limit,
            q.team_members_limit,
            q.mac_limit
        ),
        (Some(1000), Some(4), Some(2), Some(3), Some(100))
    );
    // The snapshot's white-label flags are never stamped.
    assert_eq!(
        (q.white_label, q.sso_saml, q.saas_mode),
        (false, false, false)
    );
    assert_eq!(
        (q.plan_name.as_deref(), q.plan_status.as_deref()),
        (Some("Free"), Some("trial"))
    );
    assert!(q.period_start.is_some());
    assert!(q.period_end.expect("period end") > before);
}

#[tokio::test]
async fn falls_back_to_the_lockdown_trial_length_when_snapshot_trial_days_is_invalid() {
    let db = db_with_user(None).await;
    let mut value = snapshot();
    value.as_object_mut().unwrap().remove("trialDays");
    let kv = kv(global(value)).await;
    let before = chatbotx_db::repo::now_ms();
    user_quota::ensure_bootstrap_plan(&db, Some(&kv), Edition::Cloud, USER, None)
        .await
        .expect("bootstrap");
    let q = stored(&db).await;
    assert_eq!(q.plan_status.as_deref(), Some("trial"));
    let end = q.period_end.expect("period end");
    assert!(end >= before + DAY && end <= chatbotx_db::repo::now_ms() + DAY);
}

#[tokio::test]
async fn stamps_an_active_never_expiring_row_when_snapshot_trial_days_is_0() {
    let db = db_with_user(None).await;
    let mut value = snapshot();
    value["trialDays"] = json!(0);
    let kv = kv(global(value)).await;
    user_quota::ensure_bootstrap_plan(&db, Some(&kv), Edition::Cloud, USER, None)
        .await
        .expect("bootstrap");
    let q = stored(&db).await;
    assert_eq!(
        (q.plan_status.as_deref(), q.period_end),
        (Some("active"), None)
    );
    let state = access_state_from_quota(Some(&q), NOW);
    assert_eq!(
        (state.blocked, state.status.as_deref(), state.trial_ends_at),
        (false, Some("active"), None)
    );
}

#[tokio::test]
async fn does_not_clobber_an_existing_quota_row() {
    // Also "skips the cache bust when the insert was a no-op conflict": the
    // second call reports no insert.
    let db = db_with_user(None).await;
    insert_quota(&db, r#"INSERT INTO "UserQuota" ("id", "userId", "planName", "planStatus") VALUES (1, ?1, 'Paid', 'active')"#).await;
    let kv = kv(global(snapshot())).await;
    let inserted = user_quota::ensure_bootstrap_plan(&db, Some(&kv), Edition::Cloud, USER, None)
        .await
        .expect("bootstrap");
    assert!(!inserted);
    assert_eq!(stored(&db).await.plan_name.as_deref(), Some("Paid"));
}

#[tokio::test]
async fn stamps_the_lockdown_fallback_when_the_snapshot_is_absent() {
    let db = db_with_user(None).await;
    user_quota::ensure_bootstrap_plan(&db, None, Edition::Cloud, USER, None)
        .await
        .expect("bootstrap");
    let q = stored(&db).await;
    assert_eq!(
        (
            q.contacts_limit,
            q.workspaces_limit,
            q.channels_limit,
            q.team_members_limit,
            q.mac_limit
        ),
        (Some(0), Some(0), Some(0), Some(0), Some(0))
    );
    assert_eq!(
        (q.plan_name.as_deref(), q.plan_status.as_deref()),
        (Some("Trial"), Some("trial"))
    );
    // `macLimit 0` means "cannot act": the mac gate blocks the fallback trial.
    let state = access_state_from_quota(Some(&q), chatbotx_db::repo::now_ms());
    assert_eq!((state.blocked, state.reason), (true, Some("mac")));
    assert!(state.trial_ends_at.is_some());
}

#[tokio::test]
async fn no_ops_outside_cloud_edition() {
    let db = db_with_user(None).await;
    let kv = kv(global(snapshot())).await;
    for edition in [Edition::Community, Edition::Enterprise] {
        let inserted = user_quota::ensure_bootstrap_plan(&db, Some(&kv), edition, USER, None)
            .await
            .expect("bootstrap");
        assert!(!inserted);
    }
    assert!(
        user_quota::find_for_user(&db, USER)
            .await
            .expect("read")
            .is_none()
    );
}

#[tokio::test]
async fn uses_a_tenant_snapshot_for_a_non_root_tenant() {
    let db = db_with_user(None).await;
    let mut tenant = snapshot();
    tenant["planName"] = json!("Tenant Free");
    tenant["macLimit"] = json!(250);
    let kv = kv(vec![
        (format!("{DEFAULT_PLAN_ENTITLEMENT_KEY}:42"), tenant),
        (DEFAULT_PLAN_ENTITLEMENT_KEY.to_string(), snapshot()),
    ])
    .await;
    user_quota::ensure_bootstrap_plan(&db, Some(&kv), Edition::Cloud, USER, Some(42))
        .await
        .expect("bootstrap");
    let q = stored(&db).await;
    assert_eq!(
        (q.mac_limit, q.plan_name.as_deref()),
        (Some(250), Some("Tenant Free"))
    );
}

#[tokio::test]
async fn uses_the_lockdown_fallback_when_a_tenant_snapshot_is_missing() {
    let db = db_with_user(None).await;
    let kv = kv(global(snapshot())).await;
    user_quota::ensure_bootstrap_plan(&db, Some(&kv), Edition::Cloud, USER, Some(42))
        .await
        .expect("bootstrap");
    let q = stored(&db).await;
    assert_eq!(
        (q.mac_limit, q.plan_name.as_deref()),
        (Some(0), Some("Trial"))
    );
}

// --- default plan (getForUser) ---

#[tokio::test]
async fn an_older_snapshot_without_monthly_bot_messages_limit_remains_unlimited() {
    let db = db_with_user(None).await;
    let kv = kv(global(snapshot())).await;
    let q = user_quota::get_effective_for_user(&db, Some(&kv), Edition::Cloud, USER)
        .await
        .expect("read")
        .expect("effective");
    assert_eq!(q.monthly_bot_messages_limit, None);
}

#[tokio::test]
async fn free_tier_user_with_no_row_inherits_limits_from_the_snapshot() {
    let db = db_with_user(None).await;
    let kv = kv(global(snapshot())).await;
    let q = user_quota::get_effective_for_user(&db, Some(&kv), Edition::Cloud, USER)
        .await
        .expect("read")
        .expect("effective");
    assert_eq!((q.mac_limit, q.contacts_limit), (Some(100), Some(1000)));
}

#[tokio::test]
async fn usage_only_row_inherits_mac_limit_and_keeps_its_usage() {
    let db = db_with_user(None).await;
    insert_quota(
        &db,
        r#"INSERT INTO "UserQuota" ("id", "userId", "macUsed") VALUES (1, ?1, 3)"#,
    )
    .await;
    let kv = kv(global(snapshot())).await;
    let q = user_quota::get_effective_for_user(&db, Some(&kv), Edition::Cloud, USER)
        .await
        .expect("read")
        .expect("effective");
    assert_eq!((q.mac_limit, q.mac_used), (Some(100), 3));
}

#[tokio::test]
async fn an_explicit_stored_mac_limit_is_preserved_over_the_snapshot() {
    let db = db_with_user(None).await;
    insert_quota(
        &db,
        r#"INSERT INTO "UserQuota" ("id", "userId", "macLimit") VALUES (1, ?1, 5000)"#,
    )
    .await;
    let kv = kv(global(snapshot())).await;
    let q = user_quota::get_effective_for_user(&db, Some(&kv), Edition::Cloud, USER)
        .await
        .expect("read")
        .expect("effective");
    assert_eq!(q.mac_limit, Some(5000));
}

#[tokio::test]
async fn no_published_snapshot_leaves_no_quota() {
    let db = db_with_user(None).await;
    let kv = kv(vec![]).await;
    let q = user_quota::get_effective_for_user(&db, Some(&kv), Edition::Cloud, USER)
        .await
        .expect("read");
    assert!(q.is_none());
}

#[tokio::test]
async fn sub_account_prefers_its_reseller_tenant_snapshot_over_the_global_default() {
    let db = db_with_user(Some(42)).await;
    let mut tenant = snapshot();
    tenant["planName"] = json!("Reseller Free");
    tenant["macLimit"] = json!(999);
    let kv = kv(vec![
        (format!("{DEFAULT_PLAN_ENTITLEMENT_KEY}:42"), tenant),
        (DEFAULT_PLAN_ENTITLEMENT_KEY.to_string(), snapshot()),
    ])
    .await;
    let q = user_quota::get_effective_for_user(&db, Some(&kv), Edition::Cloud, USER)
        .await
        .expect("read")
        .expect("effective");
    assert_eq!(
        (q.mac_limit, q.plan_name.as_deref()),
        (Some(999), Some("Reseller Free"))
    );
}

#[tokio::test]
async fn sub_account_with_no_reseller_default_does_not_read_the_global_default() {
    let db = db_with_user(Some(42)).await;
    let kv = kv(global(snapshot())).await;
    let q = user_quota::get_effective_for_user(&db, Some(&kv), Edition::Cloud, USER)
        .await
        .expect("read");
    assert!(q.is_none());
}

#[tokio::test]
async fn root_tenant_user_reads_the_global_default_and_ignores_any_tenant_key() {
    let db = db_with_user(None).await;
    let mut tenant = snapshot();
    tenant["macLimit"] = json!(999);
    let kv = kv(vec![
        (format!("{DEFAULT_PLAN_ENTITLEMENT_KEY}:1"), tenant),
        (DEFAULT_PLAN_ENTITLEMENT_KEY.to_string(), snapshot()),
    ])
    .await;
    let q = user_quota::get_effective_for_user(&db, Some(&kv), Edition::Cloud, USER)
        .await
        .expect("read")
        .expect("effective");
    assert_eq!(
        (q.mac_limit, q.plan_name.as_deref()),
        (Some(100), Some("Free"))
    );
}

// --- getPlanIdentity ---

#[tokio::test]
async fn plan_identity_reads_the_raw_row() {
    let db = db_with_user(None).await;
    insert_quota(&db, r#"INSERT INTO "UserQuota" ("id", "userId", "planName", "planStatus") VALUES (1, ?1, 'Stored plan', 'trial')"#).await;
    assert_eq!(
        user_quota::get_plan_identity(&db, USER)
            .await
            .expect("identity"),
        (true, Some("Stored plan".to_string()))
    );
}

#[tokio::test]
async fn returns_an_unrestricted_null_identity_when_the_raw_row_is_missing() {
    let db = db_with_user(None).await;
    assert_eq!(
        user_quota::get_plan_identity(&db, USER)
            .await
            .expect("identity"),
        (false, None)
    );
}

#[tokio::test]
async fn passes_through_a_null_raw_plan_name() {
    let db = db_with_user(None).await;
    insert_quota(
        &db,
        r#"INSERT INTO "UserQuota" ("id", "userId", "planStatus") VALUES (1, ?1, 'active')"#,
    )
    .await;
    assert_eq!(
        user_quota::get_plan_identity(&db, USER)
            .await
            .expect("identity"),
        (false, None)
    );
}

// --- listDueExpiredTrials ---

async fn trial_users(db: &Db) {
    db.write(|tx| {
        for (id, status, end, torn) in [
            (21, "trial", Some(NOW - 10 * DAY), None),
            (22, "trial", Some(NOW - 9 * DAY), None),
            (23, "trial", Some(NOW - 8 * DAY), Some(NOW)),
            (24, "active", Some(NOW - 8 * DAY), None),
            (25, "trial", Some(NOW), None),
            (26, "trial", None, None),
        ] {
            tx.execute(
                r#"INSERT INTO "User" ("id", "email") VALUES (?1, ?2)"#,
                params![id, format!("u{id}@example.test")],
            )?;
            tx.execute(
                r#"INSERT INTO "UserQuota" ("id", "userId", "planStatus", "periodEnd", "channelsTornDownAt") VALUES (?1, ?1, ?2, ?3, ?4)"#,
                params![id, status, end, torn],
            )?;
        }
        Ok(())
    })
    .await
    .expect("seed");
}

#[tokio::test]
async fn queries_only_trial_rows_past_the_cutoff_with_channels_not_yet_torn_down() {
    let db = fresh_db().await;
    trial_users(&db).await;
    let (ids, _) =
        user_quota::list_due_expired_trials(&db, NOW - TRIAL_TEARDOWN_GRACE_MS, None, 10)
            .await
            .expect("list");
    assert_eq!(ids, vec![21, 22]);
}

#[test]
fn uses_a_real_7_day_grace_window() {
    assert_eq!(TRIAL_TEARDOWN_GRACE_MS, 7 * DAY);
}

#[tokio::test]
async fn adds_a_user_id_cursor_filter_when_paging() {
    let db = fresh_db().await;
    trial_users(&db).await;
    let (ids, _) = user_quota::list_due_expired_trials(&db, NOW, Some(21), 10)
        .await
        .expect("list");
    assert_eq!(ids, vec![22, 25]);
}

#[tokio::test]
async fn returns_a_next_cursor_only_when_the_page_is_full() {
    let db = fresh_db().await;
    trial_users(&db).await;
    let full = user_quota::list_due_expired_trials(&db, NOW, None, 2)
        .await
        .expect("list");
    assert_eq!(full, (vec![21, 22], Some(22)));
    let short = user_quota::list_due_expired_trials(&db, NOW, Some(22), 2)
        .await
        .expect("list");
    assert_eq!(short, (vec![25], None));
}
