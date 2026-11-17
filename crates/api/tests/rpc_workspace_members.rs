//! The workspace member actions and list procedure under `/rpc` (Q.D11
//! slice 2). Ported from `apps/builder/__tests__/workspace-members-actions.test.ts`:
//! the TS test mocks the services and calls the action body; here the
//! actions run through `/rpc/workspaceMemberActions/<name>` against a
//! migrated in-memory database (audit rows written inline) and the stored
//! rows are checked. Realtime is off in the fixture, so a revoke always
//! "succeeds" (`revokeWarning: false`).

mod rpc_common;

use axum::http::StatusCode;
use chatbotx_business::keys::Edition;
use chatbotx_business::workspace_member::normalize_contacts_permissions;
use rpc_common::{AGENT, AGENT_TOKEN, OWNER_TOKEN, WS, action, call, fixture, fixture_as, scalar};
use serde_json::{Value, json};

const KEY: &str = "workspaceMemberActions";
const AGENT_MEMBER: i64 = 402;
const OWNER_MEMBER: i64 = 401;

fn granular() -> Value {
    json!({
        "superAdmin": false,
        "analytics": true,
        "flows": false,
        "contacts": true,
        "onlyAssignedContacts": true,
        "emailAndPhone": false,
        "broadcast": false,
        "ecommerce": false,
    })
}

fn assigned_only() -> Value {
    let mut p = granular();
    p["contacts"] = json!(false);
    p["onlyAssignedContacts"] = json!(true);
    p
}

fn full() -> Value {
    chatbotx_business::workspace::full_permissions()
}

fn invite(permissions: Value) -> Value {
    json!({ "workspaceId": WS.to_string(), "permissions": permissions })
}

fn notification_types() -> Value {
    json!({ "notifyAdmin": true, "newMessageToHuman": false, "newOrder": false })
}

fn notification_channels() -> Value {
    json!({ "messenger": false, "email": true, "telegram": false, "browser": true })
}

fn update(permissions: Value) -> Value {
    json!({
        "workspaceId": WS.to_string(),
        "id": AGENT_MEMBER.to_string(),
        "permissions": permissions,
        "notificationTypes": notification_types(),
        "notificationChannels": notification_channels(),
    })
}

async fn invitation_permissions(fx: &rpc_common::Fixture) -> Value {
    let text = scalar(
        fx,
        r#"SELECT "permissions" FROM "Invitation" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await;
    serde_json::from_str(text.as_str().expect("invitation row")).unwrap()
}

async fn member_permissions(fx: &rpc_common::Fixture) -> Value {
    let text = scalar(
        fx,
        r#"SELECT "permissions" FROM "WorkspaceMember" WHERE "id" = ?1"#,
        AGENT_MEMBER,
    )
    .await;
    serde_json::from_str(text.as_str().unwrap()).unwrap()
}

async fn audit_details(fx: &rpc_common::Fixture) -> Vec<(String, String)> {
    fx.db()
        .read(|conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "action", "detail" FROM "AuditLog" ORDER BY "createdAt", "id""#,
            )?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .expect("audit rows")
}

async fn set_member(fx: &rpc_common::Fixture, permissions: Value, types: Value, channels: Value) {
    fx.db()
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "WorkspaceMember" SET "permissions" = ?2, "notificationTypes" = ?3, "notificationChannels" = ?4 WHERE "id" = ?1"#,
                chatbotx_db::rusqlite::params![AGENT_MEMBER, permissions.to_string(), types.to_string(), channels.to_string()],
            )?;
            Ok(())
        })
        .await
        .expect("set member");
}

// --- workspace member permission helpers ---

#[test]
fn normalizes_mutually_exclusive_contact_access_without_mutating_input() {
    let input = granular();
    let mut expected = granular();
    expected["onlyAssignedContacts"] = json!(false);
    assert_eq!(normalize_contacts_permissions(&input), expected);
    assert_eq!(input["onlyAssignedContacts"], json!(true));
    assert_eq!(
        normalize_contacts_permissions(&assigned_only()),
        assigned_only()
    );
}

// --- inviteWorkspaceMemberAction ---

#[tokio::test]
async fn invite_rejects_non_super_admin_members_before_creating_invitations() {
    let fx = fixture_as(Edition::Cloud).await;
    let (status, out) = action(
        &fx,
        KEY,
        "inviteWorkspaceMember",
        AGENT_TOKEN,
        invite(granular()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(
        out["json"]["message"],
        json!(
            "You are not authorized to invite a workspace member. You need to be a super admin to do this."
        )
    );
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "Invitation" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        json!(0)
    );
}

#[tokio::test]
async fn invite_forces_full_super_admin_permissions_for_community() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        KEY,
        "inviteWorkspaceMember",
        OWNER_TOKEN,
        invite(granular()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(invitation_permissions(&fx).await, full());
}

#[tokio::test]
async fn invite_normalizes_full_contacts_permissions_outside_community() {
    let fx = fixture_as(Edition::Cloud).await;
    let (status, out) = action(
        &fx,
        KEY,
        "inviteWorkspaceMember",
        OWNER_TOKEN,
        invite(granular()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        invitation_permissions(&fx).await,
        normalize_contacts_permissions(&granular())
    );
}

#[tokio::test]
async fn invite_preserves_assigned_only_contacts_permissions_outside_community() {
    let fx = fixture_as(Edition::Cloud).await;
    let (status, out) = action(
        &fx,
        KEY,
        "inviteWorkspaceMember",
        OWNER_TOKEN,
        invite(assigned_only()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(invitation_permissions(&fx).await, assigned_only());
}

#[tokio::test]
async fn invite_returns_the_invitation_url() {
    let fx = fixture_as(Edition::Cloud).await;
    let (status, out) = action(
        &fx,
        KEY,
        "inviteWorkspaceMember",
        OWNER_TOKEN,
        invite(granular()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let code = out["json"]["code"].as_str().expect("code");
    assert_eq!(code.len(), 22, "16 bytes base64url");
    assert_eq!(
        out["json"]["inviteUrl"],
        json!(format!("http://app/invitations/{code}"))
    );
    assert!(out["json"]["id"].is_string());
}

#[tokio::test]
async fn invite_records_an_audit_event_labeled_with_the_granted_role() {
    let fx = fixture_as(Edition::Cloud).await;
    action(
        &fx,
        KEY,
        "inviteWorkspaceMember",
        OWNER_TOKEN,
        invite(granular()),
    )
    .await;
    assert_eq!(
        audit_details(&fx).await,
        vec![("invite".into(), "invited a new member".into())]
    );
}

#[tokio::test]
async fn invite_is_refused_at_the_team_member_limit() {
    // Rust-native: the `hasReachedLimit` gate over the stored quota.
    let fx = fixture_as(Edition::Cloud).await;
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "UserQuota" ("id", "userId", "teamMembersLimit") VALUES (1, ?1, 2)"#,
                [rpc_common::OWNER],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let (status, out) = action(
        &fx,
        KEY,
        "inviteWorkspaceMember",
        OWNER_TOKEN,
        invite(granular()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(
        out["json"]["message"],
        json!("Team member limit reached for this workspace plan")
    );
}

#[tokio::test]
async fn invite_needs_at_least_one_permission() {
    let fx = fixture_as(Edition::Cloud).await;
    let none: Value = full()
        .as_object()
        .unwrap()
        .keys()
        .map(|k| (k.clone(), json!(false)))
        .collect::<serde_json::Map<_, _>>()
        .into();
    let (status, out) = action(&fx, KEY, "inviteWorkspaceMember", OWNER_TOKEN, invite(none)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(
        out["json"]["data"]["validationErrors"]["permissions"]["permissions"]["_errors"],
        json!(["At least one permission must be selected."])
    );
}

// --- updateWorkspaceMemberAction ---

#[tokio::test]
async fn update_records_a_role_change_audit_event_with_the_target_members_name() {
    let fx = fixture_as(Edition::Cloud).await;
    let (status, out) = action(
        &fx,
        KEY,
        "updateWorkspaceMember",
        OWNER_TOKEN,
        update(granular()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        audit_details(&fx).await,
        vec![(
            "role_change".into(),
            "changed role of Agent to member".into()
        )]
    );
}

#[tokio::test]
async fn update_forces_full_super_admin_permissions_for_community() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        KEY,
        "updateWorkspaceMember",
        OWNER_TOKEN,
        update(granular()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(member_permissions(&fx).await, full());
}

#[tokio::test]
async fn update_normalizes_full_contacts_permissions_outside_community() {
    let fx = fixture_as(Edition::Cloud).await;
    action(
        &fx,
        KEY,
        "updateWorkspaceMember",
        OWNER_TOKEN,
        update(granular()),
    )
    .await;
    assert_eq!(
        member_permissions(&fx).await,
        normalize_contacts_permissions(&granular())
    );
}

#[tokio::test]
async fn update_preserves_assigned_only_contacts_permissions_outside_community() {
    let fx = fixture_as(Edition::Cloud).await;
    action(
        &fx,
        KEY,
        "updateWorkspaceMember",
        OWNER_TOKEN,
        update(assigned_only()),
    )
    .await;
    assert_eq!(member_permissions(&fx).await, assigned_only());
}

#[tokio::test]
async fn update_skips_the_write_and_audit_when_nothing_changed() {
    let fx = fixture_as(Edition::Cloud).await;
    set_member(
        &fx,
        normalize_contacts_permissions(&granular()),
        notification_types(),
        notification_channels(),
    )
    .await;
    let before = scalar(
        &fx,
        r#"SELECT "updatedAt" FROM "WorkspaceMember" WHERE "id" = ?1"#,
        AGENT_MEMBER,
    )
    .await;
    let (status, out) = action(
        &fx,
        KEY,
        "updateWorkspaceMember",
        OWNER_TOKEN,
        update(granular()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"], json!({ "revokeWarning": false }));
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "updatedAt" FROM "WorkspaceMember" WHERE "id" = ?1"#,
            AGENT_MEMBER
        )
        .await,
        before
    );
    assert!(audit_details(&fx).await.is_empty());
}

#[tokio::test]
async fn update_writes_notification_only_changes_without_a_role_change_audit() {
    let fx = fixture_as(Edition::Cloud).await;
    let off = json!({ "notifyAdmin": false, "newMessageToHuman": false, "newOrder": false });
    let channels_off =
        json!({ "messenger": false, "email": false, "telegram": false, "browser": false });
    set_member(
        &fx,
        normalize_contacts_permissions(&granular()),
        off,
        channels_off,
    )
    .await;
    let (status, out) = action(
        &fx,
        KEY,
        "updateWorkspaceMember",
        OWNER_TOKEN,
        update(granular()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let stored = scalar(
        &fx,
        r#"SELECT "notificationTypes" FROM "WorkspaceMember" WHERE "id" = ?1"#,
        AGENT_MEMBER,
    )
    .await;
    assert_eq!(
        serde_json::from_str::<Value>(stored.as_str().unwrap()).unwrap(),
        notification_types()
    );
    assert!(audit_details(&fx).await.is_empty());
}

#[tokio::test]
async fn update_returns_no_revoke_warning_when_sockets_close() {
    let fx = fixture_as(Edition::Cloud).await;
    let (_, out) = action(
        &fx,
        KEY,
        "updateWorkspaceMember",
        OWNER_TOKEN,
        update(granular()),
    )
    .await;
    assert_eq!(out["json"], json!({ "revokeWarning": false }));
}

#[tokio::test]
async fn update_needs_a_super_admin_and_a_member_of_the_workspace() {
    let fx = fixture_as(Edition::Cloud).await;
    let (status, out) = action(
        &fx,
        KEY,
        "updateWorkspaceMember",
        AGENT_TOKEN,
        update(granular()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(
        out["json"]["message"],
        json!(
            "You are not authorized to update this workspace member. You need to be a super admin to do this."
        )
    );
    let mut missing = update(granular());
    missing["id"] = json!("999");
    let (status, out) = action(&fx, KEY, "updateWorkspaceMember", OWNER_TOKEN, missing).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{out}");
    assert_eq!(out["json"]["message"], json!("Workspace member not found"));
}

// --- deleteWorkspaceMemberAction ---

fn delete(id: i64) -> Value {
    json!({ "workspaceId": WS.to_string(), "id": id.to_string() })
}

#[tokio::test]
async fn delete_rejects_deleting_the_workspace_owner() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        KEY,
        "deleteWorkspaceMember",
        OWNER_TOKEN,
        delete(OWNER_MEMBER),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(
        out["json"]["message"],
        json!("You cannot delete the owner of the workspace")
    );
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "WorkspaceMember" WHERE "id" = ?1"#,
            OWNER_MEMBER
        )
        .await,
        json!(1)
    );
}

#[tokio::test]
async fn delete_rejects_non_super_admin_members() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        KEY,
        "deleteWorkspaceMember",
        AGENT_TOKEN,
        delete(AGENT_MEMBER),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(
        out["json"]["message"],
        json!(
            "You are not authorized to delete this workspace member. You need to be a super admin to do this."
        )
    );
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "WorkspaceMember" WHERE "id" = ?1"#,
            AGENT_MEMBER
        )
        .await,
        json!(1)
    );
}

#[tokio::test]
async fn delete_removes_the_member_without_mutating_the_team_member_quota() {
    let fx = fixture().await;
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "UserQuota" ("id", "userId", "teamMembersUsed") VALUES (1, ?1, 2)"#,
                [rpc_common::OWNER],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let (status, out) = action(
        &fx,
        KEY,
        "deleteWorkspaceMember",
        OWNER_TOKEN,
        delete(AGENT_MEMBER),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"], json!({ "revokeWarning": false }));
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "WorkspaceMember" WHERE "id" = ?1"#,
            AGENT_MEMBER
        )
        .await,
        json!(0)
    );
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "teamMembersUsed" FROM "UserQuota" WHERE "userId" = ?1"#,
            rpc_common::OWNER
        )
        .await,
        json!(2)
    );
    assert_eq!(
        audit_details(&fx).await,
        vec![("delete".into(), "removed Agent from workspace".into())]
    );
}

// --- Rust-native: list procedure ---

#[tokio::test]
async fn list_workspace_members_pages_with_users_and_date_meta() {
    let fx = fixture().await;
    let (status, out) = call(
        &fx,
        "GET",
        "/rpc/workspaceMembersAPI/listWorkspaceMembersAuthenticatedAPI",
        Some(OWNER_TOKEN),
        json!({ "workspaceId": WS.to_string(), "keyword": "Agent" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let data = out["json"]["data"].as_array().expect("data");
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["userId"], json!(AGENT.to_string()));
    assert_eq!(data[0]["user"]["name"], json!("Agent"));
    assert_eq!(out["json"]["pageCount"], json!(1));
    assert!(
        out["meta"]
            .as_array()
            .unwrap()
            .contains(&json!([1, "data", 0, "createdAt"]))
    );
}
