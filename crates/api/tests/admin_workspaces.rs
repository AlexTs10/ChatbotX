//! `packages/business/__tests__/admin-workspace-service.test.ts` and
//! `apps/builder/__tests__/admin-workspaces-queries.test.ts` (Q.D11 slice 11)
//! on a real database: `createWithMembers` and the `adminWorkspaceActions`.

mod rpc_common;

use std::sync::Mutex;

use axum::http::StatusCode;
use chatbotx_api::mail::{Mailer, SendFuture};
use chatbotx_api::rpc::actions::ActionError;
use chatbotx_api::rpc::admin_workspaces::create_with_members;
use chatbotx_business::keys::Edition;
use rpc_common::{
    AGENT, AGENT_TOKEN, OUTSIDER, OWNER, OWNER_TOKEN, WS, action, fixture_as, scalar,
};
use serde_json::{Value, json};

#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<String>>,
}

impl Mailer for Recorder {
    fn send<'a>(&'a self, to: &'a str, _subject: &'a str, _text: &'a str) -> SendFuture<'a> {
        Box::pin(async move {
            self.sent.lock().unwrap().push(to.to_string());
            Ok(())
        })
    }
}

fn permissions() -> Value {
    json!({ "superAdmin": false, "analytics": false, "flows": true, "contacts": true, "onlyAssignedContacts": false, "emailAndPhone": false, "broadcast": false, "ecommerce": false })
}

async fn fixture() -> rpc_common::Fixture {
    let fx = fixture_as(Edition::Community).await;
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "PlatformAdmin" ("id", "userId", "role") VALUES (1, ?1, 'super-admin')"#,
                [OWNER],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    fx
}

fn code(err: ActionError) -> (Option<String>, Option<u16>) {
    match err {
        ActionError::Exception(e) => (e.code().map(str::to_owned), e.status()),
        other => panic!("not an exception: {other:?}"),
    }
}

const WORKSPACES_OF: &str = r#"SELECT COUNT(*) FROM "Workspace" WHERE "ownerId" = ?1"#;

/// "trims and lowercases the owner email, and dedupes + strips the owner
/// from the member list" + the audit row.
#[tokio::test]
async fn normalizes_emails_and_creates_the_workspace_with_its_members() {
    let fx = fixture().await;
    let mail = Recorder::default();
    let members = vec![
        "OUTSIDER@rpc.test".to_string(),
        " Agent@RPC.test ".into(),
        "agent@rpc.test".into(),
    ];
    let created = create_with_members(
        &fx.api,
        &mail,
        OWNER,
        "Support",
        " Outsider@RPC.test ",
        &members,
        &permissions(),
    )
    .await
    .unwrap();
    assert_eq!(
        (
            created.owner.email.as_str(),
            created.owner.user_id,
            created.owner.created
        ),
        ("outsider@rpc.test", OUTSIDER, false)
    );
    assert_eq!(
        created
            .members
            .iter()
            .map(|m| m.user_id)
            .collect::<Vec<_>>(),
        vec![AGENT]
    );
    assert!(mail.sent.lock().unwrap().is_empty());
    let ws = created.workspace.id;
    assert_eq!(created.workspace.owner_id, OUTSIDER);
    let role = r#"SELECT "role" || ':' || "permissions" FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = 102"#;
    let row = scalar(&fx, role, ws).await;
    let (role, perms) = row.as_str().unwrap().split_once(':').unwrap();
    assert_eq!(
        (role, serde_json::from_str::<Value>(perms).unwrap()),
        ("agent", permissions())
    );
    let detail =
        r#"SELECT "detail" FROM "AuditLog" WHERE "workspaceId" = ?1 AND "action" = 'create'"#;
    assert_eq!(
        scalar(&fx, detail, ws).await,
        json!(format!(
            "created the workspace (#{ws}) for outsider@rpc.test with members: agent@rpc.test"
        ))
    );
}

/// "provisions a fresh user for an email with no existing account".
#[tokio::test]
async fn provisions_a_new_owner_and_new_members_with_mailed_credentials() {
    let fx = fixture().await;
    let mail = Recorder::default();
    let created = create_with_members(
        &fx.api,
        &mail,
        OWNER,
        "New",
        "boss@new.test",
        &["staff@new.test".into()],
        &permissions(),
    )
    .await
    .unwrap();
    assert!(created.owner.created && created.members[0].created);
    assert_eq!(
        *mail.sent.lock().unwrap(),
        vec!["boss@new.test".to_string(), "staff@new.test".into()]
    );
    let ws = created.workspace.id;
    let detail = r#"SELECT "detail" FROM "AuditLog" WHERE "workspaceId" = ?1"#;
    assert_eq!(
        scalar(&fx, detail, ws).await,
        json!(format!(
            "created the workspace (#{ws}) for boss@new.test (new account) with members: staff@new.test (new account)"
        ))
    );
}

/// "throws ownerAlreadyHasWorkspace (409) without creating anything".
#[tokio::test]
async fn an_owner_with_a_workspace_gets_409() {
    let fx = fixture().await;
    let err = create_with_members(
        &fx.api,
        &Recorder::default(),
        OWNER,
        "Again",
        "owner@rpc.test",
        &[],
        &permissions(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        code(err),
        (Some("ownerAlreadyHasWorkspace".into()), Some(409))
    );
    assert_eq!(scalar(&fx, WORKSPACES_OF, OWNER).await, json!(1));
}

/// "throws memberNotFound for an email that doesn't resolve in a reseller
/// tenant" / "resolves an existing member … in the owner's reseller tenant".
#[tokio::test]
async fn members_of_a_reseller_tenant_must_exist_there() {
    let fx = fixture().await;
    fx.db()
        .write(|tx| {
            tx.execute(r#"INSERT INTO "Tenant" ("id", "ownerId", "status") VALUES (7, ?1, 'active')"#, [OUTSIDER])?;
            tx.execute(r#"INSERT INTO "User" ("id", "email", "tenantId") VALUES (150, 'member@reseller.test', 7)"#, [])?;
            Ok(())
        })
        .await
        .unwrap();
    let mail = Recorder::default();
    let err = create_with_members(
        &fx.api,
        &mail,
        OWNER,
        "R",
        "outsider@rpc.test",
        &["agent@rpc.test".into()],
        &permissions(),
    )
    .await
    .unwrap_err();
    assert_eq!(code(err), (Some("memberNotFound".into()), Some(404)));
    assert_eq!(scalar(&fx, WORKSPACES_OF, OUTSIDER).await, json!(0));
    assert!(mail.sent.lock().unwrap().is_empty());

    let created = create_with_members(
        &fx.api,
        &mail,
        OWNER,
        "R",
        "outsider@rpc.test",
        &["member@reseller.test".into()],
        &permissions(),
    )
    .await
    .unwrap();
    assert_eq!(
        (
            created.workspace.tenant_id,
            created.members[0].user_id,
            created.members[0].created
        ),
        (7, 150, false)
    );
}

/// "throws teamMembersLimit before creating anything when there's no room
/// for the owner plus every member" (no member is provisioned or mailed).
#[tokio::test]
async fn no_team_member_room_refuses_before_any_member_is_resolved() {
    let fx = fixture().await;
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "UserQuota" ("id", "userId", "teamMembersLimit") VALUES (1, ?1, 1)"#,
                [OUTSIDER],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let mail = Recorder::default();
    let err = create_with_members(
        &fx.api,
        &mail,
        OWNER,
        "Q",
        "outsider@rpc.test",
        &["staff@new.test".into()],
        &permissions(),
    )
    .await
    .unwrap_err();
    assert_eq!(code(err), (Some("teamMembersLimit".into()), Some(400)));
    assert!(mail.sent.lock().unwrap().is_empty());
    assert_eq!(scalar(&fx, WORKSPACES_OF, OUTSIDER).await, json!(0));
}

/// `listAdminWorkspaces`: "throws notFound … when the current user is not a
/// super admin" / "passes the keyword through".
#[tokio::test]
async fn list_admin_workspaces_is_for_super_admins_and_filters_by_keyword() {
    let fx = fixture().await;
    let key = "adminWorkspaceActions";
    let (status, _) = action(&fx, key, "listAdminWorkspaces", AGENT_TOKEN, json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, out) = action(
        &fx,
        key,
        "listAdminWorkspaces",
        OWNER_TOKEN,
        json!({ "keyword": null }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let page = &out["json"];
    assert_eq!(page["pageCount"], json!(1));
    assert_eq!(page["data"][0]["id"], json!(WS.to_string()));
    assert_eq!(page["data"][0]["ownerEmail"], json!("owner@rpc.test"));
    let (_, out) = action(
        &fx,
        key,
        "listAdminWorkspaces",
        OWNER_TOKEN,
        json!({ "keyword": "nothing-matches" }),
    )
    .await;
    assert_eq!(out["json"]["data"], json!([]));
    let (_, out) = action(
        &fx,
        key,
        "listAdminWorkspaces",
        OWNER_TOKEN,
        json!({ "keyword": "OWNER@rpc" }),
    )
    .await;
    assert_eq!(out["json"]["data"].as_array().unwrap().len(), 1);
}

/// The action: super admin, schema, then `assertEnterpriseFeatures()`.
#[tokio::test]
async fn create_admin_workspace_action_gates() {
    let fx = fixture().await;
    let key = "adminWorkspaceActions";
    let input = json!({ "name": "X", "ownerEmail": "boss@new.test", "permissions": permissions() });
    let (status, _) = action(&fx, key, "createAdminWorkspace", AGENT_TOKEN, input.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let bad = json!({ "name": "", "ownerEmail": "nope", "memberEmails": ["bad"], "permissions": permissions() });
    let (status, out) = action(&fx, key, "createAdminWorkspace", OWNER_TOKEN, bad).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    let (status, out) = action(&fx, key, "createAdminWorkspace", OWNER_TOKEN, input).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{out}");
    assert_eq!(out["json"]["code"], json!("enterpriseFeatureRequired"));
}
