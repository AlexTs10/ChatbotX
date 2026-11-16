//! `/rpc/pages/*`: the SPA page loaders (conversion/specs/P4-web.md §5.2),
//! merged with the `/rpc/*` catch-all as `chatbotx-server` does.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chatbotx_api::mail::NoMailer;
use chatbotx_api::rpc::rpc_router;
use chatbotx_api::session::{AuthSettings, AuthState, TrustedOriginSettings, sign_cookie_value};
use chatbotx_api::spa::spa_router;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};
use tower::ServiceExt as _;

const SECRET: &str = "spa-test-secret";
const NOW_MS: i64 = 1_800_000_000_000;

fn now_ms() -> i64 {
    NOW_MS
}

async fn state() -> Arc<AuthState> {
    let db = Db::open_in_memory().expect("open db");
    db.migrate().await.expect("migrate");
    db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "name", "email", "emailVerified") VALUES (101, 'Ann', 'ann@spa.test', 1)"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "Session" ("id", "token", "userId", "expiresAt") VALUES (301, 'anntoken', 101, ?1)"#,
            params![NOW_MS + 60_000],
        )?;
        Ok(())
    })
    .await
    .expect("seed");
    Arc::new(AuthState {
        db: Arc::new(db),
        settings: AuthSettings {
            secret: SECRET.to_string(),
            base_url: "http://app".to_string(),
            sign_up_disabled: true,
            trusted_origins: TrustedOriginSettings {
                broker_url: None,
                builder_url: "http://app".to_string(),
                next_phase: None,
            },
            extra_trusted_origins: None,
        },
        mailer: Arc::new(NoMailer),
        now_ms,
    })
}

async fn page(
    state: &Arc<AuthState>,
    name: &str,
    signed_in: bool,
    input: Value,
) -> (StatusCode, Value) {
    let app = spa_router(Arc::clone(state)).merge(rpc_router(Arc::clone(state)));
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/rpc/pages/{name}"))
        .header(header::CONTENT_TYPE, "application/json");
    if signed_in {
        request = request.header(
            header::COOKIE,
            format!(
                "better-auth.session_token={}",
                sign_cookie_value("anntoken", SECRET)
            ),
        );
    }
    let body = json!({ "json": input, "meta": [] }).to_string();
    let response = app
        .oneshot(request.body(Body::from(body)).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value: Value = serde_json::from_slice(&bytes).expect("json body");
    (status, value["json"].clone())
}

#[tokio::test]
async fn session_and_auth_options_need_no_sign_in() {
    let state = state().await;
    let (status, body) = page(&state, "session", false, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "user": null, "tenant": null }));

    let (_, body) = page(&state, "session", true, Value::Null).await;
    assert_eq!(body["user"]["id"], "101");
    assert_eq!(body["user"]["emailVerified"], true);
    assert_eq!(body["user"]["mustChangePassword"], false);

    let (_, body) = page(&state, "authOptions", false, Value::Null).await;
    assert_eq!(body["signUpAllowed"], false);
}

#[tokio::test]
async fn home_creates_the_first_workspace_once() {
    let state = state().await;
    let (status, _) = page(&state, "home", false, Value::Null).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, first) = page(&state, "home", true, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let id = first["createdWorkspaceId"]
        .as_str()
        .expect("created")
        .to_string();
    assert_eq!(first["workspaces"][0]["id"], id.as_str());
    assert_eq!(first["workspaces"][0]["name"], "Ann's workspace");
    assert!(first["workspaces"][0].get("token").is_none());
    assert_eq!(first["workspaces"][0]["isActive"], true);
    assert_eq!(first["ownerWorkspaceIds"], json!([id]));

    let (_, second) = page(&state, "home", true, Value::Null).await;
    assert_eq!(second["createdWorkspaceId"], Value::Null);
    assert_eq!(second["workspaces"].as_array().map(Vec::len), Some(1));

    let (status, shell) = page(&state, "workspace", true, json!({ "workspaceId": id })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(shell["permissions"]["superAdmin"], true);
    assert_eq!(shell["scheduledForDeletion"], false);

    let (_, access) = page(&state, "contactsAccess", true, json!({ "workspaceId": id })).await;
    assert_eq!(access, json!({ "canViewEmailAndPhone": true }));

    let (status, _) = page(&state, "workspace", true, json!({ "workspaceId": "999" })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (_, folder) = page(
        &state,
        "folder",
        true,
        json!({ "workspaceId": id, "id": "5" }),
    )
    .await;
    assert_eq!(folder, json!({ "folder": null, "parents": [] }));
}

#[tokio::test]
async fn flow_page_loads_the_editor_props_and_channels_create_checks_the_workspace() {
    let state = state().await;
    let (_, home) = page(&state, "home", true, Value::Null).await;
    let id = home["createdWorkspaceId"]
        .as_str()
        .expect("created")
        .to_string();
    let ws: i64 = id.parse().expect("id");
    state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "name", "workspaceId") VALUES (601, 'Welcome', ?1)"#,
                params![ws],
            )?;
            // Draft and published differ only in an editor-only key: no revert.
            for (version, draft, node) in [
                (701, 1, r#"[{"id":"1","selected":true,"data":{"forceToolbarVisible":true}}]"#),
                (702, 0, r#"[{"id":"1","data":{}}]"#),
            ] {
                tx.execute(
                    r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "isLatest", "startNodeId")
                       VALUES (?1, ?2, 601, ?3, '[]', ?4, 1, 1)"#,
                    params![version, ws, node, draft],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed flow");

    let (status, flow) = page(
        &state,
        "flow",
        true,
        json!({ "workspaceId": id, "id": "601" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{flow}");
    assert_eq!(flow["flow"]["name"], "Welcome");
    assert_eq!(
        flow["flow"]["flowVersions"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(flow["flowVersion"]["id"], "701");
    assert_eq!(flow["hasPublishedVersion"], true);
    assert_eq!(flow["canRevertToPublished"], false);
    assert_eq!(flow["isTemplateLocked"], false);
    assert_eq!(flow["openaiCompatibleIntegrations"], json!([]));

    let (status, _) = page(
        &state,
        "flow",
        true,
        json!({ "workspaceId": id, "id": "999" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, channels) = page(&state, "channelsCreate", true, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(channels["offeredChannels"], json!(["webchat", "api"]));
    let (status, _) = page(
        &state,
        "channelsCreate",
        true,
        json!({ "workspaceId": "999" }),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
