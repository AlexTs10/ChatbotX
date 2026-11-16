//! Ported from `packages/business/__tests__/user-quota-reconcile-self.test.ts`
//! (Q.D11 slice 6). The TS test checks the drizzle upsert arguments and the
//! Redis `hset`; here the counts come from real rows and the test reads the
//! stored `UserQuota` row. There is no live Redis counter (DECISIONS).

mod common;

use chatbotx_business::user_quota::{self, SelfUsageMarkers};
use chatbotx_db::Db;
use chatbotx_db::DbError;
use chatbotx_db::repo::models::UserQuota;
use chatbotx_db::rusqlite::{Connection, params};
use common::{contact, fresh_db, inbox, setup, workspace};

const OWNER: i64 = 1001;
const TENANT: i64 = 42;

fn user(tx: &Connection, id: i64) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "User" ("id", "email") VALUES (?1, ?2)"#,
        params![id, format!("user-{id}@example.test")],
    )?;
    Ok(())
}

fn member(tx: &Connection, id: i64, workspace_id: i64, user_id: i64) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role") VALUES (?1, ?2, ?3, 'agent')"#,
        params![id, workspace_id, user_id],
    )?;
    Ok(())
}

/// A channel `Connection` (with its own inbox) in `status`.
fn channel(tx: &Connection, id: i64, workspace_id: i64, status: &str) -> Result<(), DbError> {
    inbox(tx, id, workspace_id, "messenger")?;
    let reason = (status != "connected").then_some("refresh_failed");
    let disconnected = (status == "disconnected").then_some(1_i64);
    tx.execute(
        r#"INSERT INTO "Connection" ("id", "workspaceId", "provider", "kind", "inboxId", "channel", "sourceId", "displayName", "status", "statusReason", "disconnectedAt")
           VALUES (?1, ?2, 'meta', 'channel', ?1, 'messenger', ?3, 'Page', ?4, ?5, ?6)"#,
        params![id, workspace_id, format!("src-{id}"), status, reason, disconnected],
    )?;
    Ok(())
}

fn quota_row(
    tx: &Connection,
    user_id: i64,
    extra_cols: &str,
    extra_vals: &str,
) -> Result<(), DbError> {
    tx.execute(
        &format!(
            r#"INSERT INTO "UserQuota" ("id", "userId"{extra_cols}) VALUES (?1, ?1{extra_vals})"#
        ),
        [user_id],
    )?;
    Ok(())
}

async fn stored(db: &Db, user_id: i64) -> UserQuota {
    user_quota::find_for_user(db, user_id)
        .await
        .expect("read")
        .expect("quota row")
}

fn used(q: &UserQuota) -> [i64; 4] {
    [
        q.contacts_used,
        q.team_members_used,
        q.workspaces_used,
        q.channels_used,
    ]
}

/// The owner's workspace 2001 (2 contacts, 1 agent, 1 connected + 1 paused
/// channel) and a reseller tenant whose sub-account 1002 owns workspace 2002
/// (3 contacts, 1 degraded channel). The owner's workspace is in the tenant.
async fn reseller_fixture(tenant_status: &'static str) -> common::TestDb {
    let db = fresh_db().await;
    setup(&db, move |tx| {
        workspace(tx, OWNER, 2001)?;
        user(tx, 1002)?;
        user(tx, 1003)?;
        tx.execute(
            r#"INSERT INTO "Tenant" ("id", "ownerId", "status") VALUES (?1, ?2, ?3)"#,
            params![TENANT, OWNER, tenant_status],
        )?;
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId", "tenantId") VALUES (2002, 'Sub', 1002, ?1)"#,
            [TENANT],
        )?;
        tx.execute(r#"UPDATE "Workspace" SET "tenantId" = ?1 WHERE "id" = 2001"#, [TENANT])?;
        for (id, ws) in [(1, 2001), (2, 2001), (3, 2002), (4, 2002), (5, 2002)] {
            contact(tx, id, ws)?;
        }
        member(tx, 1, 2001, 1003)?;
        member(tx, 2, 2002, 1002)?;
        channel(tx, 11, 2001, "connected")?;
        channel(tx, 12, 2001, "paused")?;
        channel(tx, 13, 2002, "degraded")
    })
    .await;
    db
}

// --- reconcileOwnerAfterTeardown ---

#[tokio::test]
async fn reconciles_a_root_owner_against_only_the_owners_self_usage() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        workspace(tx, OWNER, 2001)?;
        contact(tx, 1, 2001)?;
        channel(tx, 11, 2001, "connected")
    })
    .await;
    user_quota::reconcile_owner_after_teardown(&db, OWNER, None)
        .await
        .expect("reconcile");
    assert_eq!(used(&stored(&db, OWNER).await), [1, 0, 1, 1]);
}

#[tokio::test]
async fn reconciles_an_active_reseller_owner_against_the_tenant_pool() {
    let db = reseller_fixture("active").await;
    user_quota::reconcile_owner_after_teardown(&db, OWNER, None)
        .await
        .expect("reconcile");
    // The pool (both workspaces) overwrites the self count (2001 only).
    assert_eq!(used(&stored(&db, OWNER).await), [5, 2, 2, 2]);
}

#[tokio::test]
async fn an_inactive_reseller_tenant_reconciles_only_the_self_usage() {
    let db = reseller_fixture("suspended").await;
    user_quota::reconcile_owner_after_teardown(&db, OWNER, None)
        .await
        .expect("reconcile");
    assert_eq!(used(&stored(&db, OWNER).await), [2, 1, 1, 1]);
}

#[tokio::test]
async fn uses_the_reseller_tenant_rather_than_the_root_tenant_for_root_owned_workspaces() {
    let db = reseller_fixture("active").await;
    user_quota::reconcile_owner_after_teardown(&db, OWNER, Some(1))
        .await
        .expect("reconcile");
    assert_eq!(used(&stored(&db, OWNER).await), [5, 2, 2, 2]);
}

// --- reconcileOwnerPoolUsage ---

async fn mac_fixture(stored_mac: i64, period: i64, mac_period: i64) -> common::TestDb {
    let db = reseller_fixture("active").await;
    let now = chatbotx_db::repo::now_ms();
    setup(&db, move |tx| {
        quota_row(tx, OWNER, r#", "macUsed", "periodStart""#, &format!(", {stored_mac}, {period}"))?;
        tx.execute(
            r#"INSERT INTO "WorkspaceMac" ("id", "workspaceId", "tenantId", "periodStart", "periodEnd", "macCount") VALUES (1, 2002, ?1, ?2, ?3, 9)"#,
            params![TENANT, mac_period, now + 86_400_000],
        )?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn raises_mac_to_the_tenant_period_sum_in_the_same_period() {
    let period = chatbotx_db::repo::now_ms() - 1_000;
    let db = mac_fixture(4, period, period).await;
    user_quota::reconcile_owner_pool_usage(&db, OWNER, TENANT)
        .await
        .expect("reconcile");
    assert_eq!(stored(&db, OWNER).await.mac_used, 9);
}

#[tokio::test]
async fn does_not_lower_a_higher_stored_mac() {
    // Adapted from "does not restore MAC after a quota rollover races the
    // reconcile": the guarded CASE keeps MAX(stored, sum) in the read period.
    let period = chatbotx_db::repo::now_ms() - 1_000;
    let db = mac_fixture(20, period, period).await;
    user_quota::reconcile_owner_pool_usage(&db, OWNER, TENANT)
        .await
        .expect("reconcile");
    let q = stored(&db, OWNER).await;
    assert_eq!((q.mac_used, q.period_start), (20, Some(period)));
}

// --- reconcileUserSelfUsage ---

#[tokio::test]
async fn writes_the_recomputed_count_even_when_lower_than_the_stored_value() {
    // Also "assigns counts directly (not GREATEST)".
    let db = reseller_fixture("active").await;
    setup(&db, |tx| {
        quota_row(
            tx,
            OWNER,
            r#", "contactsUsed", "teamMembersUsed", "workspacesUsed", "channelsUsed""#,
            ", 50, 50, 50, 50",
        )
    })
    .await;
    user_quota::reconcile_user_self_usage(&db, OWNER)
        .await
        .expect("reconcile");
    assert_eq!(used(&stored(&db, OWNER).await), [2, 1, 1, 1]);
}

#[tokio::test]
async fn counts_a_human_shared_across_workspaces_once() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        workspace(tx, OWNER, 2001)?;
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (2002, 'Second', ?1)"#,
            [OWNER],
        )?;
        user(tx, 1003)?;
        // Owner + shared teammate in both workspaces: four rows, two people.
        member(tx, 1, 2001, OWNER)?;
        member(tx, 2, 2002, OWNER)?;
        member(tx, 3, 2001, 1003)?;
        member(tx, 4, 2002, 1003)
    })
    .await;
    user_quota::reconcile_user_self_usage(&db, OWNER)
        .await
        .expect("reconcile");
    let q = stored(&db, OWNER).await;
    assert_eq!((q.team_members_used, q.workspaces_used), (2, 2));
}

#[tokio::test]
async fn writes_increases_too() {
    let db = reseller_fixture("active").await;
    setup(&db, |tx| quota_row(tx, OWNER, "", "")).await;
    user_quota::reconcile_user_self_usage(&db, OWNER)
        .await
        .expect("reconcile");
    assert_eq!(used(&stored(&db, OWNER).await), [2, 1, 1, 1]);
}

#[tokio::test]
async fn the_self_reconcile_path_does_not_touch_mac() {
    // Adapted from "hset field list has no `mac` field": no Redis hash; the
    // stored macUsed is left as is.
    let db = reseller_fixture("active").await;
    setup(&db, |tx| quota_row(tx, OWNER, r#", "macUsed""#, ", 77")).await;
    user_quota::reconcile_user_self_usage(&db, OWNER)
        .await
        .expect("reconcile");
    assert_eq!(stored(&db, OWNER).await.mac_used, 77);
}

#[tokio::test]
async fn returns_the_four_markers_from_the_read_back() {
    let db = reseller_fixture("active").await;
    setup(&db, |tx| {
        quota_row(
            tx,
            OWNER,
            r#", "macUsed", "periodStart", "periodEnd", "monthlyBotMessagesPeriodStart""#,
            ", 10, 100, 200, 100",
        )
    })
    .await;
    let markers = user_quota::reconcile_user_self_usage(&db, OWNER)
        .await
        .expect("reconcile");
    assert_eq!(
        markers,
        SelfUsageMarkers {
            mac_used: 10,
            period_start: Some(100),
            period_end: Some(200),
            monthly_bot_messages_period_start: Some(100),
        }
    );
}

#[tokio::test]
async fn returns_zeroed_null_markers_when_no_quota_row_exists_yet() {
    let db = reseller_fixture("active").await;
    let markers = user_quota::reconcile_user_self_usage(&db, OWNER)
        .await
        .expect("reconcile");
    assert_eq!(markers, SelfUsageMarkers::default());
    // The upsert created the row with the counts.
    assert_eq!(used(&stored(&db, OWNER).await), [2, 1, 1, 1]);
}

// --- raiseMacUsed ---

#[tokio::test]
async fn upserts_a_period_pinned_monotonic_mac_value() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        user(tx, 1)?;
        user(tx, 2)?;
        user(tx, 3)?;
        quota_row(tx, 1, r#", "macUsed", "periodStart""#, ", 50, 100")?;
        quota_row(tx, 2, r#", "macUsed", "periodStart""#, ", 5, 999")
    })
    .await;
    for id in [1, 2, 3] {
        user_quota::raise_mac_used(&db, id, 42, 100)
            .await
            .expect("raise");
    }
    let q1 = stored(&db, 1).await;
    let q2 = stored(&db, 2).await;
    let q3 = stored(&db, 3).await;
    assert_eq!(q1.mac_used, 50, "never lowered");
    assert_eq!(
        (q2.mac_used, q2.period_start),
        (5, Some(999)),
        "other period kept"
    );
    assert_eq!((q3.mac_used, q3.period_start), (42, Some(100)), "inserted");
}

// --- applyMonthlyBotMessagesReset ---

#[tokio::test]
async fn reset_branch_zeroes_monthly_bot_messages_used_and_stamps_the_period() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        user(tx, 1)?;
        quota_row(tx, 1, r#", "monthlyBotMessagesUsed""#, ", 30")
    })
    .await;
    user_quota::apply_monthly_bot_messages_reset(&db, 1, Some(500), true)
        .await
        .expect("reset");
    let q = stored(&db, 1).await;
    assert_eq!(
        (
            q.monthly_bot_messages_used,
            q.monthly_bot_messages_period_start
        ),
        (0, Some(500))
    );
}

#[tokio::test]
async fn non_reset_branch_stamps_only_without_touching_the_counter() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        user(tx, 1)?;
        quota_row(tx, 1, r#", "monthlyBotMessagesUsed""#, ", 30")
    })
    .await;
    user_quota::apply_monthly_bot_messages_reset(&db, 1, Some(500), false)
        .await
        .expect("stamp");
    let q = stored(&db, 1).await;
    assert_eq!(
        (
            q.monthly_bot_messages_used,
            q.monthly_bot_messages_period_start
        ),
        (30, Some(500))
    );
}
