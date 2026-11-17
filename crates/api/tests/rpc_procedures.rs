//! The `/rpc` procedure table and the ported private procedures.
//!
//! Ported from `apps/builder/__tests__/list-questionnaires-for-flow-api.test.ts`,
//! `places-near-me-category-options.test.ts` and `poll-queries.test.ts`. The
//! TypeScript tests mock the procedure builder and the services; here the
//! registration checks read the generated procedure table and the handler
//! checks call the procedures through `/rpc` against a migrated in-memory
//! database. The table and realtime tests are Rust-native (no TS test).

use std::collections::HashSet;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chatbotx_api::handlers::ApiState;
use chatbotx_api::mail::NoMailer;
use chatbotx_api::rpc::procedures::{
    self, PORTED, find_poll, list_place_category_options, list_questionnaires_for_flow,
};
use chatbotx_api::rpc::{
    RPC_PROCEDURES, RPC_ROUTER_KEYS, RpcAuth, RpcProcedure, RpcState, find_procedure, rpc_router,
    rpc_router_with,
};
use chatbotx_api::session::{AuthSettings, AuthState, TrustedOriginSettings, sign_cookie_value};
use chatbotx_business::BusinessError;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use chatbotx_rt::TestClock;
use chatbotx_rt::token::{ChatScope, verify_member_connect_token};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const SECRET: &str = "rpc-procedures-secret";
const RT_SECRET: &str = "rpc-procedures-realtime-secret-0123456789";
const NOW_MS: i64 = 1_800_000_000_000;
const USER_ID: i64 = 101;
const ASSIGNED_USER_ID: i64 = 102;
const NO_CONTACTS_USER_ID: i64 = 103;
const OWNER_ID: i64 = 104;
const WORKSPACE_ID: i64 = 201;
const FOREIGN_WORKSPACE_ID: i64 = 202;
const POLL_ID: i64 = 701;

fn now_ms() -> i64 {
    NOW_MS
}

struct Fixture {
    auth: Arc<AuthState>,
    api: Arc<ApiState>,
}

async fn fixture() -> Fixture {
    let db = Db::open_in_memory().expect("open db");
    db.migrate().await.expect("migrate");
    db.write(|tx| {
        for (id, email) in [
            (USER_ID, "member@rpc.test"),
            (ASSIGNED_USER_ID, "assigned@rpc.test"),
            (NO_CONTACTS_USER_ID, "nocontacts@rpc.test"),
            (OWNER_ID, "owner@rpc.test"),
        ] {
            tx.execute(
                r#"INSERT INTO "User" ("id", "email", "emailVerified") VALUES (?1, ?2, 1)"#,
                params![id, email],
            )?;
        }
        for (id, token, user_id) in [
            (301, "membertoken", USER_ID),
            (302, "assignedtoken", ASSIGNED_USER_ID),
            (303, "nocontactstoken", NO_CONTACTS_USER_ID),
        ] {
            tx.execute(
                r#"INSERT INTO "Session" ("id", "token", "userId", "expiresAt")
                   VALUES (?1, ?2, ?3, ?4)"#,
                params![id, token, user_id, NOW_MS + 60_000],
            )?;
        }
        for id in [WORKSPACE_ID, FOREIGN_WORKSPACE_ID] {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'ws', ?2)"#,
                params![id, OWNER_ID],
            )?;
        }
        for (id, user_id, permissions) in [
            (401, USER_ID, r#"{"contacts":true}"#),
            (402, ASSIGNED_USER_ID, r#"{"onlyAssignedContacts":true}"#),
            (403, NO_CONTACTS_USER_ID, r#"{"flows":true}"#),
        ] {
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role", "permissions")
                   VALUES (?1, ?2, ?3, 'agent', ?4)"#,
                params![id, WORKSPACE_ID, user_id, permissions],
            )?;
        }
        // Inbox teams: the assigned member is in team 902 of this workspace and
        // in team 903 of another workspace.
        for (id, workspace_id) in [(901, WORKSPACE_ID), (902, WORKSPACE_ID), (903, FOREIGN_WORKSPACE_ID)] {
            tx.execute(
                r#"INSERT INTO "InboxTeam" ("id", "workspaceId", "name") VALUES (?1, ?2, 'team')"#,
                params![id, workspace_id],
            )?;
        }
        for (id, team_id) in [(951, 902), (952, 903)] {
            tx.execute(
                r#"INSERT INTO "InboxTeamMember" ("id", "inboxTeamId", "userId") VALUES (?1, ?2, ?3)"#,
                params![id, team_id, ASSIGNED_USER_ID],
            )?;
        }
        for (id, workspace_id, name, deleted_at) in [
            (501, WORKSPACE_ID, "Lead capture", None),
            (502, WORKSPACE_ID, "Feedback", None),
            (503, WORKSPACE_ID, "Lead archive", Some(NOW_MS)),
            (504, FOREIGN_WORKSPACE_ID, "Lead elsewhere", None),
        ] {
            tx.execute(
                r#"INSERT INTO "Questionnaire" ("id", "workspaceId", "name", "deletedAt")
                   VALUES (?1, ?2, ?3, ?4)"#,
                params![id, workspace_id, name, deleted_at],
            )?;
        }
        for (id, workspace_id, name, rank) in [
            (601, WORKSPACE_ID, "Kiosks", 2),
            (602, WORKSPACE_ID, "Stores", 1),
            (603, FOREIGN_WORKSPACE_ID, "Elsewhere", 1),
        ] {
            tx.execute(
                r#"INSERT INTO "PlaceCategory" ("id", "workspaceId", "name", "rank")
                   VALUES (?1, ?2, ?3, ?4)"#,
                params![id, workspace_id, name, rank],
            )?;
        }
        for id in [801, 802, 803] {
            tx.execute(
                r#"INSERT INTO "PlaceLocation" ("id", "workspaceId", "name", "latitude", "longitude", "categoryId")
                   VALUES (?1, ?2, 'loc', 1.0, 2.0, 602)"#,
                params![id, WORKSPACE_ID],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "Poll" ("id", "workspaceId", "name", "imageUrl", "startsAt", "endsAt")
               VALUES (?1, ?2, 'Best pizza', 'https://img.test/p.png', ?3, ?4)"#,
            params![POLL_ID, WORKSPACE_ID, NOW_MS - 1000, NOW_MS + 1000],
        )?;
        Ok(())
    })
    .await
    .expect("seed");
    let db = Arc::new(db);
    let auth = Arc::new(AuthState {
        db: Arc::clone(&db),
        settings: AuthSettings {
            secret: SECRET.to_string(),
            base_url: "http://app".to_string(),
            sign_up_disabled: false,
            trusted_origins: TrustedOriginSettings {
                broker_url: None,
                builder_url: "http://app".to_string(),
                next_phase: None,
            },
            extra_trusted_origins: None,
        },
        mailer: Arc::new(NoMailer),
        now_ms,
    });
    let api = Arc::new(ApiState::new(db, "http://app"));
    Fixture { auth, api }
}

fn rpc_state(fixture: &Fixture) -> RpcState {
    RpcState {
        auth: Arc::clone(&fixture.auth),
        api: Some(Arc::clone(&fixture.api)),
        realtime_secret: Some(RT_SECRET.to_string()),
    }
}

fn cookie(token: &str) -> String {
    format!(
        "better-auth.session_token={}",
        sign_cookie_value(token, SECRET)
    )
}

async fn send(
    router: axum::Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    input: Value,
) -> (StatusCode, Value) {
    let payload = json!({ "json": input }).to_string();
    let mut builder = Request::builder().method(method);
    builder = if method == "GET" {
        let data: String = payload.bytes().map(|byte| format!("%{byte:02X}")).collect();
        builder.uri(format!("{path}?data={data}"))
    } else {
        builder
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json")
    };
    if let Some(token) = token {
        builder = builder.header(header::COOKIE, cookie(token));
    }
    let body = if method == "GET" {
        Body::empty()
    } else {
        Body::from(payload)
    };
    let response = router.oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    (status, value)
}

async fn call(
    fixture: &Fixture,
    method: &str,
    path: &str,
    token: Option<&str>,
    input: Value,
) -> (StatusCode, Value) {
    send(
        rpc_router_with(rpc_state(fixture)),
        method,
        path,
        token,
        input,
    )
    .await
}

fn rpc_error(code: &str, status: u16, message: &str) -> Value {
    json!({
        "json": { "defined": false, "code": code, "status": status, "message": message },
        "meta": [[3, "data"]],
    })
}

fn workspace_get(path: &'static str) -> RpcProcedure {
    RpcProcedure {
        path,
        auth: RpcAuth::Session,
        method: Some("GET"),
        workspace: Some(&["workspaceId"]),
        validate_first: true,
    }
}

// list-questionnaires-for-flow-api.test.ts

#[tokio::test]
async fn listquestionnairesforflowapi_registers_an_authenticated_workspace_scoped_get_endpoint() {
    let path = "questionnairesAPI.listQuestionnairesForFlowAPI";
    assert_eq!(find_procedure(path), Some(&workspace_get(path)));
    assert!(PORTED.contains(&path));
    let fixture = fixture().await;
    // The workspace mapper reads `input.workspaceId`: a non-member is refused.
    let (status, body) = call(
        &fixture,
        "GET",
        "/rpc/questionnairesAPI/listQuestionnairesForFlowAPI",
        Some("membertoken"),
        json!({ "workspaceId": FOREIGN_WORKSPACE_ID.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body, rpc_error("UNAUTHORIZED", 401, "Unauthorized"));
}

#[tokio::test]
async fn listquestionnairesforflowapi_calls_questionnaireservice_listforflow_with_validated_input()
{
    let fixture = fixture().await;
    let expected = json!([{ "id": "501", "name": "Lead capture" }]);
    let direct = list_questionnaires_for_flow(&fixture.api, WORKSPACE_ID, Some("lead".into()))
        .await
        .expect("listForFlow");
    assert_eq!(direct, expected);
    let (status, body) = call(
        &fixture,
        "GET",
        "/rpc/questionnairesAPI/listQuestionnairesForFlowAPI",
        Some("membertoken"),
        json!({ "workspaceId": WORKSPACE_ID.to_string(), "keyword": "lead" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "json": expected }));
    // Without a keyword: every live questionnaire of the workspace, by name.
    let (status, body) = call(
        &fixture,
        "GET",
        "/rpc/questionnairesAPI/listQuestionnairesForFlowAPI",
        Some("membertoken"),
        json!({ "workspaceId": WORKSPACE_ID.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({ "json": [
            { "id": "502", "name": "Feedback" },
            { "id": "501", "name": "Lead capture" },
        ] })
    );
}

// places-near-me-category-options.test.ts

#[tokio::test]
async fn listplacecategoryoptionsapi_registers_an_authenticated_workspace_scoped_get_endpoint() {
    let path = "placesNearMeAPI.listPlaceCategoryOptionsAPI";
    assert_eq!(find_procedure(path), Some(&workspace_get(path)));
    assert!(PORTED.contains(&path));
    let fixture = fixture().await;
    let (status, body) = call(
        &fixture,
        "GET",
        "/rpc/placesNearMeAPI/listPlaceCategoryOptionsAPI",
        Some("membertoken"),
        json!({ "workspaceId": FOREIGN_WORKSPACE_ID.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body, rpc_error("UNAUTHORIZED", 401, "Unauthorized"));
}

#[tokio::test]
async fn listplacecategoryoptionsapi_returns_id_and_name_in_service_order() {
    let fixture = fixture().await;
    let expected = json!({ "data": [
        { "id": "602", "name": "Stores" },
        { "id": "601", "name": "Kiosks" },
    ] });
    let direct = list_place_category_options(&fixture.api, WORKSPACE_ID)
        .await
        .expect("list");
    assert_eq!(direct, expected);
    let (status, body) = call(
        &fixture,
        "GET",
        "/rpc/placesNearMeAPI/listPlaceCategoryOptionsAPI",
        Some("membertoken"),
        json!({ "workspaceId": WORKSPACE_ID.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "json": expected }));
}

// poll-queries.test.ts

#[tokio::test]
async fn findpoll_returns_the_poll() {
    let fixture = fixture().await;
    let poll = find_poll(&fixture.api, WORKSPACE_ID, POLL_ID)
        .await
        .expect("findPoll")
        .expect("a poll");
    assert_eq!(poll["id"], json!(POLL_ID.to_string()));
    assert_eq!(poll["name"], json!("Best pizza"));
}

#[tokio::test]
async fn findpoll_returns_undefined_when_the_poll_does_not_exist() {
    let fixture = fixture().await;
    assert_eq!(
        find_poll(&fixture.api, WORKSPACE_ID, 999)
            .await
            .expect("findPoll"),
        None
    );
    // A poll of another workspace does not exist for this one.
    assert_eq!(
        find_poll(&fixture.api, FOREIGN_WORKSPACE_ID, POLL_ID)
            .await
            .expect("findPoll"),
        None
    );
}

#[tokio::test]
async fn findpoll_rethrows_any_other_failure_instead_of_turning_it_into_a_404() {
    let fixture = fixture().await;
    fixture
        .api
        .db
        .write(|tx| {
            tx.execute_batch(r#"ALTER TABLE "Poll" RENAME TO "PollMoved""#)?;
            Ok(())
        })
        .await
        .expect("rename");
    let error = find_poll(&fixture.api, WORKSPACE_ID, POLL_ID)
        .await
        .expect_err("a repository failure");
    assert!(matches!(error, BusinessError::Repo(_)), "{error}");
}

// The procedure table (Rust-native).

#[test]
fn the_table_covers_the_79_router_keys_with_unique_paths() {
    assert_eq!(RPC_ROUTER_KEYS.len(), 79);
    let mut seen = HashSet::new();
    for procedure in RPC_PROCEDURES {
        assert!(seen.insert(procedure.path), "duplicate {}", procedure.path);
        let key = procedure.path.split('.').next().unwrap();
        assert!(RPC_ROUTER_KEYS.contains(&key), "{}", procedure.path);
    }
    for key in RPC_ROUTER_KEYS {
        assert!(
            RPC_PROCEDURES
                .iter()
                .any(|procedure| procedure.path.split('.').next() == Some(key)),
            "no procedure under {key}"
        );
    }
    assert_eq!(RPC_PROCEDURES.len(), 274);
    // The private router is session-only.
    assert!(
        RPC_PROCEDURES
            .iter()
            .all(|procedure| procedure.auth == RpcAuth::Session)
    );
    for path in PORTED {
        assert!(find_procedure(path).is_some(), "{path}");
    }
}

#[test]
fn the_table_keeps_the_workspace_mapper_and_the_validation_order() {
    let analytics = find_procedure("analyticsRoutes.getFlowAnalytics").expect("analytics");
    // `authorizedAPI.use(workspaceAuthorizedMidddleware, ...).router(analyticsRoutes)`:
    // the router-level middleware runs before the procedure's input validation.
    assert_eq!(analytics.workspace, Some(&["workspaceId"][..]));
    assert!(!analytics.validate_first);
    assert_eq!(analytics.method, Some("GET"));
    let devices = find_procedure("deviceTokensAPI.registerDeviceTokenAPI").expect("devices");
    assert_eq!(devices.workspace, None);
    assert_eq!(devices.method, Some("PUT"));
    let polls = find_procedure("pollsAPI.deletePollAPI").expect("polls");
    assert_eq!(polls.method, Some("DELETE"));
    assert!(polls.validate_first);
}

#[tokio::test]
async fn an_unported_procedure_passes_auth_and_answers_not_ported() {
    let fixture = fixture().await;
    let (status, body) = call(
        &fixture,
        "GET",
        "/rpc/pollsAPI/listPollsAPI",
        Some("membertoken"),
        json!({ "workspaceId": WORKSPACE_ID.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        rpc_error("NOT_FOUND", 404, "Operation not ported yet")
    );
    // A procedure without a workspace mapper skips the membership check.
    // (Q.D11 ported `listMyWorkspacesAPI`; another unmapped procedure stands in.)
    let (status, body) = call(
        &fixture,
        "POST",
        "/rpc/integrationWhatsappAPIs/listWhatsappPhoneNumbersInternalAPI",
        Some("membertoken"),
        json!({ "workspaceId": FOREIGN_WORKSPACE_ID.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        rpc_error("NOT_FOUND", 404, "Operation not ported yet")
    );
}

#[tokio::test]
async fn a_missing_workspace_id_fails_the_input_validation_before_the_workspace_check() {
    let fixture = fixture().await;
    for input in [json!({}), json!({ "workspaceId": "abc" })] {
        let (status, body) = call(
            &fixture,
            "GET",
            "/rpc/questionnairesAPI/listQuestionnairesForFlowAPI",
            Some("membertoken"),
            input,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            body,
            rpc_error("BAD_REQUEST", 400, "Input validation failed")
        );
    }
    // The analytics router checks the workspace first: a missing id is a 401.
    let (status, _) = call(
        &fixture,
        "GET",
        "/rpc/analyticsRoutes/getFlowAnalytics",
        Some("membertoken"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_router_without_handler_state_answers_not_ported() {
    let fixture = fixture().await;
    let (status, body) = send(
        rpc_router(Arc::clone(&fixture.auth)),
        "GET",
        "/rpc/questionnairesAPI/listQuestionnairesForFlowAPI",
        Some("membertoken"),
        json!({ "workspaceId": WORKSPACE_ID.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        body,
        rpc_error("NOT_FOUND", 404, "Operation not ported yet")
    );
}

// realtimeAPI.mintWorkspaceConnectTokenAuthenticatedAPI (Rust-native).

#[tokio::test]
async fn the_realtime_mint_signs_a_member_connect_token_for_the_caller() {
    let fixture = fixture().await;
    let (status, body) = call(
        &fixture,
        "GET",
        "/rpc/realtimeAPI/mintWorkspaceConnectTokenAuthenticatedAPI",
        Some("membertoken"),
        json!({ "workspaceId": WORKSPACE_ID.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let token = body["json"]["token"].as_str().expect("token");
    assert_eq!(body["json"].as_object().map(|o| o.len()), Some(1));
    let claims = verify_member_connect_token(
        token,
        &WORKSPACE_ID.to_string(),
        RT_SECRET,
        &TestClock::new(NOW_MS),
    )
    .expect("verifies");
    assert_eq!(claims.user_id, USER_ID.to_string());
    assert_eq!(claims.chat_scope, ChatScope::All);
    assert!(claims.team_ids.is_empty());
    assert_eq!(claims.iat_ms, u64::try_from(NOW_MS).unwrap());
}

#[tokio::test]
async fn the_member_connect_scope_follows_the_member_permissions() {
    let fixture = fixture().await;
    let db = &fixture.api.db;
    let verify = |token: String| {
        verify_member_connect_token(
            &token,
            &WORKSPACE_ID.to_string(),
            RT_SECRET,
            &TestClock::new(NOW_MS),
        )
        .expect("verifies")
    };
    let assigned = procedures::mint_member_connect_token(
        db,
        RT_SECRET,
        WORKSPACE_ID,
        ASSIGNED_USER_ID,
        &json!({ "onlyAssignedContacts": true }),
        NOW_MS,
    )
    .await
    .expect("mint");
    let assigned = verify(assigned);
    assert_eq!(assigned.chat_scope, ChatScope::Assigned);
    assert_eq!(assigned.team_ids, vec!["902".to_string()]);
    let none = procedures::mint_member_connect_token(
        db,
        RT_SECRET,
        WORKSPACE_ID,
        NO_CONTACTS_USER_ID,
        &json!({ "flows": true }),
        NOW_MS,
    )
    .await
    .expect("mint");
    assert_eq!(verify(none).chat_scope, ChatScope::None);
    let super_admin = procedures::mint_member_connect_token(
        db,
        RT_SECRET,
        WORKSPACE_ID,
        ASSIGNED_USER_ID,
        &json!({ "superAdmin": true, "onlyAssignedContacts": true }),
        NOW_MS,
    )
    .await
    .expect("mint");
    let super_admin = verify(super_admin);
    assert_eq!(super_admin.chat_scope, ChatScope::All);
    assert!(super_admin.team_ids.is_empty());
    // Through `/rpc` the stored permissions of the member decide.
    let (status, body) = call(
        &fixture,
        "GET",
        "/rpc/realtimeAPI/mintWorkspaceConnectTokenAuthenticatedAPI",
        Some("assignedtoken"),
        json!({ "workspaceId": WORKSPACE_ID.to_string() }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let claims = verify(body["json"]["token"].as_str().unwrap().to_string());
    assert_eq!(claims.chat_scope, ChatScope::Assigned);
    assert_eq!(claims.team_ids, vec!["902".to_string()]);
}
