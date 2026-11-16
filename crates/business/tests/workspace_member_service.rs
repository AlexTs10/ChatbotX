//! Ported from `packages/business/__tests__/workspace-member-permissions.test.ts`,
//! `workspace-member-predicates.test.ts`, `workspace-member.update.test.ts`
//! and `workspace-member.service.test.ts` (the member writes; cache-tag,
//! caller-tx and tracker tests check mocks only and are n-a, see
//! `conversion/specs/Q.D11-test-map.tsv`).

mod common;

use chatbotx_business::workspace_member::{
    self, MemberUpdate, has_contacts_access, has_permission, is_support_access_enabled,
    is_workspace_admin_member,
};
use chatbotx_db::rusqlite::params;
use common::{fresh_db, scalar_i64, setup};
use serde_json::json;

const WS: i64 = 21;
const OTHER_WS: i64 = 22;
const OWNER: i64 = 11;
const AGENT: i64 = 12;
const MEMBER: i64 = 31;

// --- permissions ---

#[test]
fn super_admin_bypasses_any_specific_flag() {
    assert!(has_permission(&json!({ "superAdmin": true }), "flows"));
}

#[test]
fn requires_the_specific_flag_when_not_super_admin() {
    assert!(has_permission(
        &json!({ "superAdmin": false, "flows": true }),
        "flows"
    ));
    assert!(!has_permission(&json!({ "superAdmin": false }), "flows"));
}

#[test]
fn fails_closed_on_a_missing_key() {
    assert!(!has_permission(&json!({}), "flows"));
}

#[test]
fn contacts_access_for_contacts_or_only_assigned() {
    assert!(has_contacts_access(&json!({ "contacts": true })));
    assert!(has_contacts_access(
        &json!({ "onlyAssignedContacts": true })
    ));
    assert!(!has_contacts_access(&json!({ "analytics": true })));
}

// --- predicates ---

#[test]
fn support_access_needs_a_future_until() {
    let now = 1_800_000_000_000;
    assert!(!is_support_access_enabled(None, now));
    assert!(!is_support_access_enabled(Some(now - 60_000), now));
    assert!(is_support_access_enabled(Some(now + 60_000), now));
}

#[test]
fn workspace_admin_member() {
    assert!(is_workspace_admin_member(
        "owner",
        &json!({ "superAdmin": false })
    ));
    assert!(is_workspace_admin_member(
        "agent",
        &json!({ "superAdmin": true })
    ));
    assert!(!is_workspace_admin_member(
        "agent",
        &json!({ "superAdmin": false, "broadcast": true })
    ));
    assert!(!is_workspace_admin_member(
        "agent",
        &serde_json::Value::Null
    ));
    assert!(!is_workspace_admin_member(
        "agent",
        &json!({ "superAdmin": "yes" })
    ));
}

// --- update / delete ---

async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    setup(&db, |tx| {
        common::workspace(tx, OWNER, WS)?;
        common::workspace(tx, 13, OTHER_WS)?;
        tx.execute(
            r#"INSERT INTO "User" ("id", "email", "name") VALUES (?1, 'agent@example.test', 'Agent')"#,
            [AGENT],
        )?;
        tx.execute(
            r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role", "permissions") VALUES (?1, ?2, ?3, 'agent', '{"contacts":true}')"#,
            params![MEMBER, WS, AGENT],
        )?;
        tx.execute(
            r#"INSERT INTO "WorkspaceUsage" ("id", "workspaceId", "teamMembersUsed") VALUES (1, ?1, 2)"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    db
}

fn data() -> MemberUpdate {
    MemberUpdate {
        permissions: json!({ "superAdmin": true }),
        notification_types: json!({ "notifyAdmin": true, "newMessageToHuman": false, "newOrder": false }),
        notification_channels: json!({ "messenger": false, "email": true, "telegram": false, "browser": true }),
    }
}

#[tokio::test]
async fn update_returns_none_when_zero_rows_are_updated() {
    let db = seeded().await;
    assert_eq!(
        workspace_member::update(&db, WS, 999, data())
            .await
            .expect("update"),
        None
    );
}

#[tokio::test]
async fn update_returns_the_member_user_on_success() {
    let db = seeded().await;
    assert_eq!(
        workspace_member::update(&db, WS, MEMBER, data())
            .await
            .expect("update"),
        Some(AGENT)
    );
    let member = workspace_member::find_by_id_or_fail(&db, WS, MEMBER)
        .await
        .expect("member");
    assert_eq!(member.permissions.0, json!({ "superAdmin": true }));
    assert_eq!(member.notification_channels.0["email"], json!(true));
}

#[tokio::test]
async fn update_is_scoped_to_both_the_member_id_and_the_workspace() {
    let db = seeded().await;
    assert_eq!(
        workspace_member::update(&db, OTHER_WS, MEMBER, data())
            .await
            .expect("update"),
        None
    );
    let member = workspace_member::find_by_id_or_fail(&db, WS, MEMBER)
        .await
        .expect("member");
    assert_eq!(member.permissions.0, json!({ "contacts": true }));
}

#[tokio::test]
async fn find_by_id_or_fail_is_404_outside_the_workspace() {
    let db = seeded().await;
    let error = workspace_member::find_by_id_or_fail(&db, OTHER_WS, MEMBER)
        .await
        .expect_err("404");
    assert_eq!(error.message(), "Workspace member not found");
    assert_eq!(error.status(), Some(404));
}

#[tokio::test]
async fn delete_removes_member_and_schedule_row_in_one_transaction() {
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute(r#"INSERT INTO "WorkingHours" ("id", "workspaceId", "name") VALUES (41, ?1, 'Hours')"#, [WS])?;
        tx.execute(
            r#"INSERT INTO "WorkingHoursMember" ("id", "workingHoursId", "workspaceId", "userId") VALUES (51, 41, ?1, ?2)"#,
            params![WS, AGENT],
        )?;
        Ok(())
    })
    .await;
    let removed = workspace_member::delete(&db, WS, MEMBER)
        .await
        .expect("delete")
        .expect("removed");
    assert_eq!((removed.user_id, removed.label.as_str()), (AGENT, "Agent"));
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT COUNT(*) FROM "WorkspaceMember" WHERE "id" = ?1"#,
            MEMBER
        )
        .await,
        Some(0)
    );
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT COUNT(*) FROM "WorkingHoursMember" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(0)
    );
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "teamMembersUsed" FROM "WorkspaceUsage" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(1)
    );
}

#[tokio::test]
async fn delete_skips_the_cleanup_when_the_member_does_not_exist() {
    let db = seeded().await;
    assert_eq!(
        workspace_member::delete(&db, WS, 999)
            .await
            .expect("delete"),
        None
    );
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "teamMembersUsed" FROM "WorkspaceUsage" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(2)
    );
}
