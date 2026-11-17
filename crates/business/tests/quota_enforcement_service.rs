//! Ported from `packages/business/__tests__/quota-enforcement.service.test.ts`,
//! `quota-enforcement-mac-admission.test.ts` and `quota-enforcement-env.test.ts`
//! (Q.D11 slice 6). The TS tests mock `userQuotaService` and check which rows
//! it was called with; here the levels are real `UserQuota` rows and the test
//! reads their `*Used` columns. Redis, lock and admission-strategy cases are
//! n-a (one SQLite write transaction; DECISIONS).

mod common;

use chatbotx_business::BusinessError;
use chatbotx_business::quota_enforcement::{self as qe, ConsumeLevel, MetricUsage};
use chatbotx_business::tx;
use chatbotx_business::user_quota::QuotaMetric;
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};
use common::{TestDb, fresh_db, scalar_i64, setup};

const ROOT_USER: i64 = 1;
const RESELLER: i64 = 2;
const CUSTOMER: i64 = 3;
const TENANT: i64 = 42;
const ROOT_WS: i64 = 2001;
const RESELLER_WS: i64 = 2002;
const CUSTOMER_WS: i64 = 2003;

/// Root user 1 (workspace 2001); reseller 2 owns active tenant 42 (workspace
/// 2002 in the tenant); customer 3 is a sub-account of tenant 42 (workspace
/// 2003). Each user has a `UserQuota` row with no limits.
async fn fixture() -> TestDb {
    let db = fresh_db().await;
    setup(&db, |tx| {
        for id in [ROOT_USER, RESELLER, CUSTOMER] {
            tx.execute(
                r#"INSERT INTO "User" ("id", "email") VALUES (?1, ?2)"#,
                params![id, format!("u{id}@example.test")],
            )?;
            tx.execute(
                r#"INSERT INTO "UserQuota" ("id", "userId") VALUES (?1, ?1)"#,
                [id],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "Tenant" ("id", "ownerId", "status") VALUES (?1, ?2, 'active')"#,
            params![TENANT, RESELLER],
        )?;
        tx.execute(
            r#"UPDATE "User" SET "tenantId" = ?1 WHERE "id" = ?2"#,
            params![TENANT, CUSTOMER],
        )?;
        for (ws, owner, tenant) in [
            (ROOT_WS, ROOT_USER, 1),
            (RESELLER_WS, RESELLER, TENANT),
            (CUSTOMER_WS, CUSTOMER, TENANT),
        ] {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId", "tenantId") VALUES (?1, 'W', ?2, ?3)"#,
                params![ws, owner, tenant],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

/// Sets columns of a user's quota row (`"col" = value, ...`).
async fn set_quota(db: &Db, user: i64, set: &'static str) {
    db.write(move |tx| {
        tx.execute(
            &format!(r#"UPDATE "UserQuota" SET {set} WHERE "userId" = ?1"#),
            [user],
        )?;
        Ok(())
    })
    .await
    .expect("set quota");
}

fn member(tx: &Connection, id: i64, ws: i64, user: i64) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role") VALUES (?1, ?2, ?3, 'agent')"#,
        params![id, ws, user],
    )?;
    Ok(())
}

async fn used(db: &Db, user: i64, metric: QuotaMetric) -> i64 {
    let sql: &'static str = match metric {
        QuotaMetric::Workspaces => {
            r#"SELECT "workspacesUsed" FROM "UserQuota" WHERE "userId" = ?1"#
        }
        QuotaMetric::Contacts => r#"SELECT "contactsUsed" FROM "UserQuota" WHERE "userId" = ?1"#,
        QuotaMetric::TeamMembers => {
            r#"SELECT "teamMembersUsed" FROM "UserQuota" WHERE "userId" = ?1"#
        }
        QuotaMetric::Mac => r#"SELECT "macUsed" FROM "UserQuota" WHERE "userId" = ?1"#,
        _ => unreachable!("not used by these tests"),
    };
    scalar_i64(db, sql, user).await.expect("quota row")
}

async fn workspace_usage(db: &Db, ws: i64, column: &'static str) -> i64 {
    let sql: &'static str = match column {
        "contactsUsed" => r#"SELECT "contactsUsed" FROM "WorkspaceUsage" WHERE "workspaceId" = ?1"#,
        "macUsed" => r#"SELECT "macUsed" FROM "WorkspaceUsage" WHERE "workspaceId" = ?1"#,
        _ => unreachable!(),
    };
    scalar_i64(db, sql, ws).await.unwrap_or(0)
}

/// Runs a sync gate in one write transaction.
async fn run<R: Send + 'static>(
    db: &Db,
    f: impl FnOnce(&Connection) -> chatbotx_business::Result<R> + Send + 'static,
) -> chatbotx_business::Result<R> {
    tx::write(db, move |conn| f(conn)).await
}

async fn consume(db: &Db, user: i64, metric: QuotaMetric) -> Result<(), ConsumeLevel> {
    run(db, move |c| Ok(qe::try_consume(c, user, metric)?))
        .await
        .expect("try consume")
}

// --- tryConsume ---

#[tokio::test]
async fn root_tenant_user_keeps_the_per_user_behavior() {
    let db = fixture().await;
    set_quota(&db, ROOT_USER, r#""workspacesLimit" = 5"#).await;
    assert_eq!(
        consume(&db, ROOT_USER, QuotaMetric::Workspaces).await,
        Ok(())
    );
    assert_eq!(used(&db, ROOT_USER, QuotaMetric::Workspaces).await, 1);
    assert_eq!(used(&db, RESELLER, QuotaMetric::Workspaces).await, 0);
}

#[tokio::test]
async fn reseller_acting_directly_consumes_the_owner_pool_row_only() {
    let db = fixture().await;
    assert_eq!(
        consume(&db, RESELLER, QuotaMetric::Workspaces).await,
        Ok(())
    );
    assert_eq!(used(&db, RESELLER, QuotaMetric::Workspaces).await, 1);
    assert_eq!(used(&db, CUSTOMER, QuotaMetric::Workspaces).await, 0);
}

#[tokio::test]
async fn customer_consumes_both_their_own_row_and_the_owner_pool_row() {
    let db = fixture().await;
    assert_eq!(
        consume(&db, CUSTOMER, QuotaMetric::Workspaces).await,
        Ok(())
    );
    assert_eq!(used(&db, RESELLER, QuotaMetric::Workspaces).await, 1);
    assert_eq!(used(&db, CUSTOMER, QuotaMetric::Workspaces).await, 1);
}

#[tokio::test]
async fn customer_is_blocked_by_the_owner_pool() {
    let db = fixture().await;
    set_quota(
        &db,
        RESELLER,
        r#""workspacesLimit" = 1, "workspacesUsed" = 1"#,
    )
    .await;
    assert_eq!(
        consume(&db, CUSTOMER, QuotaMetric::Workspaces).await,
        Err(ConsumeLevel::Pool)
    );
    assert_eq!(used(&db, RESELLER, QuotaMetric::Workspaces).await, 1);
    assert_eq!(used(&db, CUSTOMER, QuotaMetric::Workspaces).await, 0);
}

#[tokio::test]
async fn customer_is_blocked_by_their_own_quota_even_when_the_pool_has_room() {
    let db = fixture().await;
    set_quota(
        &db,
        CUSTOMER,
        r#""workspacesLimit" = 1, "workspacesUsed" = 1"#,
    )
    .await;
    assert_eq!(
        consume(&db, CUSTOMER, QuotaMetric::Workspaces).await,
        Err(ConsumeLevel::User)
    );
    assert_eq!(used(&db, RESELLER, QuotaMetric::Workspaces).await, 0);
}

#[tokio::test]
async fn a_suspended_tenant_has_no_pool() {
    let db = fixture().await;
    db.write(|tx| {
        tx.execute(
            r#"UPDATE "Tenant" SET "status" = 'suspended' WHERE "id" = 42"#,
            [],
        )?;
        Ok(())
    })
    .await
    .expect("suspend");
    assert_eq!(
        consume(&db, CUSTOMER, QuotaMetric::Workspaces).await,
        Ok(())
    );
    assert_eq!(used(&db, RESELLER, QuotaMetric::Workspaces).await, 0);
    assert_eq!(used(&db, CUSTOMER, QuotaMetric::Workspaces).await, 1);
}

// --- getDualRemainingSlots ---

async fn dual(db: &Db, user: i64, metric: QuotaMetric) -> Option<i64> {
    run(db, move |c| Ok(qe::dual_remaining_slots(c, user, metric)?))
        .await
        .expect("remaining")
}

#[tokio::test]
async fn returns_the_tighter_of_user_and_pool_remaining_for_a_customer() {
    let db = fixture().await;
    set_quota(&db, RESELLER, r#""contactsLimit" = 10, "contactsUsed" = 8"#).await;
    set_quota(&db, CUSTOMER, r#""contactsLimit" = 5"#).await;
    assert_eq!(dual(&db, CUSTOMER, QuotaMetric::Contacts).await, Some(2));
}

#[tokio::test]
async fn treats_null_unlimited_as_no_constraint() {
    let db = fixture().await;
    set_quota(&db, RESELLER, r#""contactsLimit" = 7"#).await;
    assert_eq!(dual(&db, CUSTOMER, QuotaMetric::Contacts).await, Some(7));
    assert_eq!(dual(&db, ROOT_USER, QuotaMetric::Contacts).await, None);
}

// --- increment / release ---

#[tokio::test]
async fn increments_both_the_subs_row_and_the_owner_pool_row_for_a_customer() {
    let db = fixture().await;
    run(&db, |c| {
        Ok(qe::increment_by(c, CUSTOMER, QuotaMetric::Contacts, 1)?)
    })
    .await
    .expect("increment");
    assert_eq!(used(&db, RESELLER, QuotaMetric::Contacts).await, 1);
    assert_eq!(used(&db, CUSTOMER, QuotaMetric::Contacts).await, 1);
}

#[tokio::test]
async fn increments_only_the_owner_pool_row_for_a_reseller() {
    let db = fixture().await;
    run(&db, |c| {
        Ok(qe::increment_by(c, RESELLER, QuotaMetric::Contacts, 1)?)
    })
    .await
    .expect("increment");
    assert_eq!(used(&db, RESELLER, QuotaMetric::Contacts).await, 1);
    assert_eq!(used(&db, CUSTOMER, QuotaMetric::Contacts).await, 0);
}

#[tokio::test]
async fn releases_both_the_sub_account_and_owner_pool_rows() {
    let db = fixture().await;
    set_quota(&db, RESELLER, r#""teamMembersUsed" = 4"#).await;
    set_quota(&db, CUSTOMER, r#""teamMembersUsed" = 2"#).await;
    run(&db, |c| {
        Ok(qe::release_by(c, CUSTOMER, QuotaMetric::TeamMembers, 1)?)
    })
    .await
    .expect("release");
    assert_eq!(used(&db, RESELLER, QuotaMetric::TeamMembers).await, 3);
    assert_eq!(used(&db, CUSTOMER, QuotaMetric::TeamMembers).await, 1);
}

#[tokio::test]
async fn releases_only_the_owner_pool_row_for_a_reseller() {
    let db = fixture().await;
    set_quota(&db, RESELLER, r#""teamMembersUsed" = 4"#).await;
    set_quota(&db, CUSTOMER, r#""teamMembersUsed" = 2"#).await;
    run(&db, |c| {
        Ok(qe::release_by(c, RESELLER, QuotaMetric::TeamMembers, 1)?)
    })
    .await
    .expect("release");
    assert_eq!(used(&db, RESELLER, QuotaMetric::TeamMembers).await, 3);
    assert_eq!(used(&db, CUSTOMER, QuotaMetric::TeamMembers).await, 2);
}

// --- createNewContactWithMac ---

const PERIOD: i64 = 1_780_272_000_000;

/// Runs `create_new_contact_with_mac` with a `create` that records the
/// period it got and returns `"c-1"` counted as `counted`.
async fn new_contact(
    db: &Db,
    owner: i64,
    ws: i64,
    counted: bool,
) -> (Result<&'static str, ConsumeLevel>, Option<Option<i64>>) {
    run(db, move |c| {
        let mut seen = None;
        let out = qe::create_new_contact_with_mac(c, owner, ws, |_, period| {
            seen = Some(period);
            Ok(("c-1", counted && period.is_some()))
        })?;
        Ok((out, seen))
    })
    .await
    .expect("create")
}

#[tokio::test]
async fn root_user_gates_creates_consumes_mac_and_bumps_counters() {
    let db = fixture().await;
    set_quota(
        &db,
        ROOT_USER,
        r#""macLimit" = 5, "periodStart" = 1780272000000"#,
    )
    .await;
    let (out, seen) = new_contact(&db, ROOT_USER, ROOT_WS, true).await;
    assert_eq!(out, Ok("c-1"));
    assert_eq!(seen, Some(Some(PERIOD)), "create gets the owner's period");
    assert_eq!(used(&db, ROOT_USER, QuotaMetric::Mac).await, 1);
    assert_eq!(used(&db, ROOT_USER, QuotaMetric::Contacts).await, 1);
    assert_eq!(workspace_usage(&db, ROOT_WS, "macUsed").await, 1);
    assert_eq!(workspace_usage(&db, ROOT_WS, "contactsUsed").await, 1);
}

#[tokio::test]
async fn rejects_and_creates_nothing_when_mac_is_exhausted() {
    let db = fixture().await;
    set_quota(&db, ROOT_USER, r#""macLimit" = 1, "macUsed" = 1"#).await;
    let (out, seen) = new_contact(&db, ROOT_USER, ROOT_WS, true).await;
    assert_eq!(out, Err(ConsumeLevel::User));
    assert_eq!(seen, None, "create not called");
    assert_eq!(used(&db, ROOT_USER, QuotaMetric::Contacts).await, 0);
}

#[tokio::test]
async fn reports_the_pool_level_when_the_pool_mac_is_exhausted() {
    let db = fixture().await;
    set_quota(&db, RESELLER, r#""macLimit" = 1, "macUsed" = 1"#).await;
    let (out, seen) = new_contact(&db, CUSTOMER, CUSTOMER_WS, true).await;
    assert_eq!((out, seen), (Err(ConsumeLevel::Pool), None));
}

#[tokio::test]
async fn period_less_owner_with_a_finite_mac_limit_still_consumes_quota() {
    let db = fixture().await;
    set_quota(&db, ROOT_USER, r#""macLimit" = 5"#).await;
    let (out, seen) = new_contact(&db, ROOT_USER, ROOT_WS, true).await;
    assert_eq!((out, seen), (Ok("c-1"), Some(None)));
    assert_eq!(used(&db, ROOT_USER, QuotaMetric::Mac).await, 1);
    assert_eq!(workspace_usage(&db, ROOT_WS, "macUsed").await, 0);
}

#[tokio::test]
async fn period_less_owner_without_a_mac_limit_records_contacts_but_no_mac() {
    let db = fixture().await;
    let (out, _) = new_contact(&db, ROOT_USER, ROOT_WS, true).await;
    assert_eq!(out, Ok("c-1"));
    assert_eq!(used(&db, ROOT_USER, QuotaMetric::Mac).await, 0);
    assert_eq!(used(&db, ROOT_USER, QuotaMetric::Contacts).await, 1);
    assert_eq!(workspace_usage(&db, ROOT_WS, "macUsed").await, 0);
}

#[tokio::test]
async fn propagates_a_create_failure_and_consumes_nothing() {
    let db = fixture().await;
    set_quota(
        &db,
        ROOT_USER,
        r#""macLimit" = 5, "periodStart" = 1780272000000"#,
    )
    .await;
    let err = run(&db, |c| {
        qe::create_new_contact_with_mac(
            c,
            ROOT_USER,
            ROOT_WS,
            |_, _| -> chatbotx_business::Result<((), bool)> {
                Err(BusinessError::exception(
                    "canceling statement",
                    "timeout",
                    500,
                ))
            },
        )
    })
    .await
    .expect_err("create fails");
    assert_eq!(err.message(), "canceling statement");
    assert_eq!(used(&db, ROOT_USER, QuotaMetric::Mac).await, 0);
    assert_eq!(used(&db, ROOT_USER, QuotaMetric::Contacts).await, 0);
    assert_eq!(workspace_usage(&db, ROOT_WS, "contactsUsed").await, 0);
}

#[tokio::test]
async fn customer_consumes_mac_at_both_the_owner_pool_and_their_own_row() {
    let db = fixture().await;
    set_quota(&db, RESELLER, r#""macLimit" = 5"#).await;
    set_quota(&db, CUSTOMER, r#""periodStart" = 1780272000000"#).await;
    let (out, _) = new_contact(&db, CUSTOMER, CUSTOMER_WS, true).await;
    assert_eq!(out, Ok("c-1"));
    assert_eq!(used(&db, RESELLER, QuotaMetric::Mac).await, 1);
    assert_eq!(used(&db, CUSTOMER, QuotaMetric::Mac).await, 1);
}

#[tokio::test]
async fn an_uncounted_claim_consumes_no_mac_but_still_counts_the_contact() {
    // `counted false revokes MAC and still runs contact counters`.
    let db = fixture().await;
    set_quota(
        &db,
        ROOT_USER,
        r#""macLimit" = 5, "periodStart" = 1780272000000"#,
    )
    .await;
    let (out, _) = new_contact(&db, ROOT_USER, ROOT_WS, false).await;
    assert_eq!(out, Ok("c-1"));
    assert_eq!(used(&db, ROOT_USER, QuotaMetric::Mac).await, 0);
    assert_eq!(used(&db, ROOT_USER, QuotaMetric::Contacts).await, 1);
}

// --- createContactWithoutMac ---

async fn passive_contact(db: &Db, owner: i64, ws: i64) -> &'static str {
    run(db, move |c| {
        qe::create_contact_without_mac(c, owner, ws, |_| Ok("c-1"))
    })
    .await
    .expect("create")
}

#[tokio::test]
async fn root_user_bumps_only_contacts_without_mac() {
    let db = fixture().await;
    set_quota(
        &db,
        ROOT_USER,
        r#""macLimit" = 5, "periodStart" = 1780272000000"#,
    )
    .await;
    assert_eq!(passive_contact(&db, ROOT_USER, ROOT_WS).await, "c-1");
    assert_eq!(used(&db, ROOT_USER, QuotaMetric::Contacts).await, 1);
    assert_eq!(used(&db, ROOT_USER, QuotaMetric::Mac).await, 0);
    assert_eq!(workspace_usage(&db, ROOT_WS, "contactsUsed").await, 1);
}

#[tokio::test]
async fn customer_bumps_both_the_subs_row_and_the_owner_pool_row() {
    let db = fixture().await;
    passive_contact(&db, CUSTOMER, CUSTOMER_WS).await;
    assert_eq!(used(&db, RESELLER, QuotaMetric::Contacts).await, 1);
    assert_eq!(used(&db, CUSTOMER, QuotaMetric::Contacts).await, 1);
}

#[tokio::test]
async fn never_rejects_there_is_no_mac_or_contacts_gate() {
    let db = fixture().await;
    set_quota(
        &db,
        ROOT_USER,
        r#""macLimit" = 0, "contactsLimit" = 1, "contactsUsed" = 1"#,
    )
    .await;
    assert_eq!(passive_contact(&db, ROOT_USER, ROOT_WS).await, "c-1");
}

// --- getUsageSummary ---

fn entry(summary: &[(QuotaMetric, MetricUsage)], metric: QuotaMetric) -> MetricUsage {
    summary
        .iter()
        .find(|(m, _)| *m == metric)
        .expect("metric")
        .1
}

fn usage(used: i64, limit: Option<i64>) -> MetricUsage {
    MetricUsage {
        used,
        limit,
        workspace_used: None,
    }
}

async fn summary(db: &Db, user: i64) -> Vec<(QuotaMetric, MetricUsage)> {
    run(db, move |c| Ok(qe::usage_summary(c, user)?))
        .await
        .expect("summary")
}

#[tokio::test]
async fn root_tenant_user_reports_used_against_their_user_quota_limit() {
    let db = fixture().await;
    set_quota(
        &db,
        ROOT_USER,
        r#""workspacesLimit" = 10, "workspacesUsed" = 3, "teamMembersLimit" = 10, "teamMembersUsed" = 99"#,
    )
    .await;
    let s = summary(&db, ROOT_USER).await;
    assert_eq!(entry(&s, QuotaMetric::Workspaces), usage(3, Some(10)));
    // Team members are the live distinct count, not the stored column.
    assert_eq!(entry(&s, QuotaMetric::TeamMembers), usage(0, Some(10)));
    assert_eq!(s.len(), QuotaMetric::ALL.len());
}

#[tokio::test]
async fn reseller_reports_the_pooled_usage_against_their_plan_limit() {
    let db = fixture().await;
    set_quota(
        &db,
        RESELLER,
        r#""workspacesLimit" = 10, "workspacesUsed" = 8, "teamMembersLimit" = 10"#,
    )
    .await;
    // A member of a sub-account workspace counts toward the tenant pool.
    setup(&db, |tx| member(tx, 1, CUSTOMER_WS, CUSTOMER)).await;
    let s = summary(&db, RESELLER).await;
    assert_eq!(entry(&s, QuotaMetric::Workspaces), usage(8, Some(10)));
    assert_eq!(entry(&s, QuotaMetric::TeamMembers), usage(1, Some(10)));
}

#[tokio::test]
async fn sub_account_reports_its_own_allocation_not_the_pool() {
    let db = fixture().await;
    set_quota(
        &db,
        RESELLER,
        r#""workspacesLimit" = 10, "workspacesUsed" = 8"#,
    )
    .await;
    set_quota(
        &db,
        CUSTOMER,
        r#""workspacesLimit" = 5, "workspacesUsed" = 2"#,
    )
    .await;
    let s = summary(&db, CUSTOMER).await;
    assert_eq!(entry(&s, QuotaMetric::Workspaces), usage(2, Some(5)));
}

#[tokio::test]
async fn reuses_the_workspace_lifetime_bot_message_count_for_the_monthly_metric() {
    let db = fixture().await;
    set_quota(
        &db,
        ROOT_USER,
        r#""monthlyBotMessagesLimit" = 100, "monthlyBotMessagesUsed" = 10, "botMessagesUsed" = 25"#,
    )
    .await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "WorkspaceUsage" ("id", "workspaceId", "contactsUsed", "channelsUsed", "teamMembersUsed", "botMessagesUsed") VALUES (1, ?1, 1, 2, 3, 7)"#,
            [ROOT_WS],
        )?;
        Ok(())
    })
    .await;
    let s = run(&db, |c| {
        Ok(qe::workspace_usage_summary(c, ROOT_USER, ROOT_WS)?)
    })
    .await
    .expect("summary");
    assert_eq!(
        entry(&s, QuotaMetric::MonthlyBotMessages),
        MetricUsage {
            used: 10,
            limit: Some(100),
            workspace_used: Some(7)
        }
    );
    assert_eq!(entry(&s, QuotaMetric::Channels).workspace_used, Some(2));
    assert!(s.iter().all(|(m, _)| *m != QuotaMetric::Workspaces));
}

// --- hasReachedLimit ---

async fn reached(db: &Db, user: i64, metric: QuotaMetric) -> bool {
    run(db, move |c| Ok(qe::has_reached_limit(c, user, metric)?))
        .await
        .expect("reached")
}

#[tokio::test]
async fn reads_the_current_owner_scoped_distinct_count_for_team_members() {
    let db = fixture().await;
    // The stored column says 0; the live distinct count (1) is used.
    set_quota(&db, ROOT_USER, r#""teamMembersLimit" = 1"#).await;
    setup(&db, |tx| member(tx, 1, ROOT_WS, ROOT_USER)).await;
    assert!(reached(&db, ROOT_USER, QuotaMetric::TeamMembers).await);
}

#[tokio::test]
async fn keeps_non_team_member_metrics_on_the_used_column() {
    let db = fixture().await;
    set_quota(
        &db,
        ROOT_USER,
        r#""workspacesLimit" = 1, "workspacesUsed" = 1"#,
    )
    .await;
    assert!(reached(&db, ROOT_USER, QuotaMetric::Workspaces).await);
    assert!(!reached(&db, ROOT_USER, QuotaMetric::TeamMembers).await);
}

#[tokio::test]
async fn checks_both_the_sub_account_owner_scope_and_reseller_tenant_pool() {
    let db = fixture().await;
    set_quota(&db, CUSTOMER, r#""teamMembersLimit" = 10"#).await;
    set_quota(&db, RESELLER, r#""teamMembersLimit" = 1"#).await;
    setup(&db, |tx| member(tx, 1, RESELLER_WS, RESELLER)).await;
    assert!(reached(&db, CUSTOMER, QuotaMetric::TeamMembers).await);
}

// --- hasRoomForTeamMembers ---

async fn room(db: &Db, user: i64, additional: i64) -> bool {
    run(db, move |c| {
        Ok(qe::has_room_for_team_members(c, user, additional)?)
    })
    .await
    .expect("room")
}

#[tokio::test]
async fn non_pooled_room_is_governed_by_the_users_own_row_alone() {
    let db = fixture().await;
    set_quota(&db, ROOT_USER, r#""teamMembersLimit" = 5"#).await;
    setup(&db, |tx| member(tx, 1, ROOT_WS, ROOT_USER)).await;
    assert!(room(&db, ROOT_USER, 3).await);
    assert!(!room(&db, ROOT_USER, 5).await);
}

#[tokio::test]
async fn pooled_sub_account_with_room_of_its_own_is_still_blocked_by_a_full_pool() {
    let db = fixture().await;
    set_quota(&db, CUSTOMER, r#""teamMembersLimit" = 10"#).await;
    set_quota(&db, RESELLER, r#""teamMembersLimit" = 1"#).await;
    setup(&db, |tx| member(tx, 1, RESELLER_WS, RESELLER)).await;
    assert!(!room(&db, CUSTOMER, 1).await);
}

#[tokio::test]
async fn reseller_acting_directly_only_the_pool_row_governs() {
    let db = fixture().await;
    set_quota(&db, RESELLER, r#""teamMembersLimit" = 10"#).await;
    // A full sub-account row does not limit the reseller.
    set_quota(&db, CUSTOMER, r#""teamMembersLimit" = 0"#).await;
    assert!(room(&db, RESELLER, 5).await);
}

// --- getAtLimitMap ---

#[tokio::test]
async fn includes_both_lifetime_and_monthly_bot_message_metrics() {
    let db = fixture().await;
    let map = run(&db, |c| Ok(qe::at_limit_map(c, ROOT_USER)?))
        .await
        .expect("map");
    let get = |m: QuotaMetric| map.iter().find(|(k, _)| *k == m).map(|(_, v)| *v);
    assert_eq!(get(QuotaMetric::BotMessages), Some(false));
    assert_eq!(get(QuotaMetric::MonthlyBotMessages), Some(false));
    assert_eq!(map.len(), QuotaMetric::ALL.len());
}
