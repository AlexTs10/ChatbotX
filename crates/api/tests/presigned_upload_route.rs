//! Port of `apps/builder/__tests__/presigned-upload-route.test.ts` and
//! `apps/builder/__tests__/upload-handlers.test.ts` (P3.6-D2): the session
//! upload route `POST /api/presigned-upload` over a migrated in-memory
//! database and an in-memory file store (the original mocks the session,
//! the services and the uploader).

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chatbotx_api::handlers::ApiState;
use chatbotx_api::mail::NoMailer;
use chatbotx_api::presigned_upload::{
    UploadHandlerInput, UploadState, UploadType, router, upload_path,
};
use chatbotx_api::session::{AuthSettings, AuthState, TrustedOriginSettings, sign_cookie_value};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use chatbotx_files::{LocalUrlSigner, Uploader};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const SECRET: &str = "upload-test-secret";
const USER_ID: i64 = 101;
const ADMIN_ID: i64 = 102;
/// Workspace `1`, as the original fixture's `workspaces/1/...` paths.
const WS: i64 = 1;

fn now_ms() -> i64 {
    chatbotx_db::repo::now_ms()
}

struct Setup {
    app: Router,
    db: Arc<Db>,
}

async fn setup(scheduled_deletion: bool, super_admin: bool) -> Setup {
    let db = Db::open_in_memory().expect("open db");
    db.migrate().await.expect("migrate");
    let permissions = if super_admin {
        r#"{"superAdmin":true}"#
    } else {
        "{}"
    };
    let permissions = permissions.to_string();
    db.write(move |tx| {
        for (id, email) in [
            (USER_ID, "member@upload.test"),
            (ADMIN_ID, "admin@upload.test"),
        ] {
            tx.execute(
                r#"INSERT INTO "User" ("id", "email", "emailVerified") VALUES (?1, ?2, 1)"#,
                params![id, email],
            )?;
        }
        for (id, token, user_id) in [(301, "membertoken", USER_ID), (302, "admintoken", ADMIN_ID)] {
            tx.execute(
                r#"INSERT INTO "Session" ("id", "token", "userId", "expiresAt")
                   VALUES (?1, ?2, ?3, ?4)"#,
                params![id, token, user_id, now_ms() + 3_600_000],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId", "scheduledDeletionAt")
               VALUES (?1, 'ws', ?2, ?3)"#,
            params![WS, USER_ID, scheduled_deletion.then(now_ms)],
        )?;
        tx.execute(
            r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role", "permissions")
               VALUES (401, ?1, ?2, 'agent', ?3)"#,
            params![WS, USER_ID, permissions],
        )?;
        tx.execute(
            r#"INSERT INTO "PlatformAdmin" ("id", "userId") VALUES (501, ?1)"#,
            params![ADMIN_ID],
        )?;
        Ok(())
    })
    .await
    .expect("seed");
    let db = Arc::new(db);
    let files = Uploader::in_memory()
        .with_url_signer(LocalUrlSigner::new("files-secret", "http://app/files").expect("signer"));
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
    let api = Arc::new(ApiState::new(Arc::clone(&db), "http://app").with_files(Arc::new(files)));
    Setup {
        app: router(UploadState {
            auth,
            api,
            is_cloud: false,
        }),
        db,
    }
}

async fn post(setup: &Setup, token: Option<&str>, body: &Value) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri("/api/presigned-upload")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        request = request.header(
            header::COOKIE,
            format!(
                "better-auth.session_token={}",
                sign_cookie_value(token, SECRET)
            ),
        );
    }
    let response = setup
        .app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn file_rows(setup: &Setup) -> i64 {
    setup
        .db
        .read(|conn| Ok(conn.query_row(r#"SELECT count(*) FROM "File""#, [], |row| row.get(0))?))
        .await
        .expect("count")
}

fn valid_body() -> Value {
    json!({
        "fileName": "logo.png",
        "mimeType": "image/png",
        "path": "workspaces/1/logo.png",
        "subType": "generic",
        "type": "generic",
        "workspaceId": "1",
    })
}

fn ads_creative_body() -> Value {
    let mut body = valid_body();
    body["type"] = json!("adsCampaignCreative");
    body["subType"] = json!("adsCampaignCreative");
    body["path"] = json!("public/space/1/ads-campaign/creatives/abc123");
    body
}

/// Port of: apps/builder/__tests__/presigned-upload-route.test.ts:111 "POST /api/presigned-upload > mints a presigned url for a servable workspace"
#[tokio::test]
async fn post_api_presigned_upload_mints_a_presigned_url_for_a_servable_workspace() {
    let setup = setup(false, false).await;
    let (status, body) = post(&setup, Some("membertoken"), &valid_body()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Adapted: serde_json writes keys sorted, so the same key set is checked in that order.
    let keys: Vec<&str> = body
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["fileId", "path", "presignedPostUrl", "publicUrl"]);
    // The real uploader and tenant storage URL stand in for the mocked ones.
    assert!(
        body["presignedPostUrl"]
            .as_str()
            .expect("url")
            .starts_with("http://app/files/workspaces/1/logo.png?X-Amz-Expires=300&")
    );
    assert_eq!(
        body["publicUrl"],
        json!("http://app/storage/workspaces/1/logo.png")
    );
    assert_eq!(body["path"], json!("workspaces/1/logo.png"));
    let file_id: i64 = body["fileId"]
        .as_str()
        .expect("id")
        .parse()
        .expect("digits");
    let row: (String, String, String, i64) = setup
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "contextType", "subType", "status", "userId" FROM "File" WHERE "id" = ?1"#,
                params![file_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?)
        })
        .await
        .expect("file row");
    assert_eq!(
        row,
        (
            "generic".into(),
            "generic".into(),
            "pending".into(),
            USER_ID
        )
    );
}

/// Port of: apps/builder/__tests__/presigned-upload-route.test.ts:123 "POST /api/presigned-upload > rejects the upload when the workspace is frozen"
#[tokio::test]
async fn post_api_presigned_upload_rejects_the_upload_when_the_workspace_is_frozen() {
    let setup = setup(true, false).await;
    let (status, body) = post(&setup, Some("membertoken"), &valid_body()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(file_rows(&setup).await, 0);
}

/// Port of: apps/builder/__tests__/presigned-upload-route.test.ts:136 "POST /api/presigned-upload > rejects the upload when the workspace row no longer exists"
#[tokio::test]
async fn post_api_presigned_upload_rejects_the_upload_when_the_workspace_row_no_longer_exists() {
    let setup = setup(false, false).await;
    let mut body = valid_body();
    body["workspaceId"] = json!("999");
    body["path"] = json!("workspaces/999/logo.png");
    let (status, body) = post(&setup, Some("membertoken"), &body).await;
    // Changed: the original mocks the membership check; with real rows a
    // missing workspace has no member, so `assertCurrentUserCanAccessChatbot`
    // refuses first (400), before the freeze gate (403).
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(file_rows(&setup).await, 0);
}

/// Port of: apps/builder/__tests__/presigned-upload-route.test.ts:156 "POST /api/presigned-upload > adsCampaignCreative uploads — super-admin gated > mints a presigned url when the caller is a workspace super-admin"
#[tokio::test]
async fn post_api_presigned_upload_adscampaigncreative_uploads_super_admin_gated_mints_a_presigned_url_when_the_caller_is_a_workspace_super_admin()
 {
    let setup = setup(false, true).await;
    let (status, body) = post(&setup, Some("membertoken"), &ads_creative_body()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Port of: apps/builder/__tests__/presigned-upload-route.test.ts:163 "POST /api/presigned-upload > adsCampaignCreative uploads — super-admin gated > rejects a workspace member who is not a super-admin"
#[tokio::test]
async fn post_api_presigned_upload_adscampaigncreative_uploads_super_admin_gated_rejects_a_workspace_member_who_is_not_a_super_admin()
 {
    let setup = setup(false, false).await;
    let (status, body) = post(&setup, Some("membertoken"), &ads_creative_body()).await;
    // The thrown `ChatbotXException` is answered by `serverErrorHandler` (400).
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["message"],
        json!("You need to be a workspace super admin to do this.")
    );
    assert_eq!(file_rows(&setup).await, 0);
}

/// Port of: apps/builder/__tests__/presigned-upload-route.test.ts:174 "POST /api/presigned-upload > adsCampaignCreative uploads — super-admin gated > does not gate a plain generic upload on super-admin"
#[tokio::test]
async fn post_api_presigned_upload_adscampaigncreative_uploads_super_admin_gated_does_not_gate_a_plain_generic_upload_on_super_admin()
 {
    let setup = setup(false, false).await;
    let (status, body) = post(&setup, Some("membertoken"), &valid_body()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// Port of: apps/builder/__tests__/presigned-upload-route.test.ts:180 "POST /api/presigned-upload > adsCampaignCreative uploads — super-admin gated > gates on subType too — type:generic + subType:adsCampaignCreative still requires super-admin (authz-bypass guard)"
#[tokio::test]
async fn post_api_presigned_upload_adscampaigncreative_uploads_super_admin_gated_gates_on_subtype_too_type_generic_subtype_adscampaigncreative_still_requires_super_admin_authz_bypass_guard()
 {
    let setup = setup(false, false).await;
    let mut body = valid_body();
    body["subType"] = json!("adsCampaignCreative");
    let (status, body) = post(&setup, Some("membertoken"), &body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["message"],
        json!("You need to be a workspace super admin to do this.")
    );
    assert_eq!(file_rows(&setup).await, 0);
}

// ---- route behaviour the TS tests mock (own tests) --------------------------

#[tokio::test]
async fn a_request_without_a_session_is_401_and_a_bad_body_is_422() {
    let setup = setup(false, false).await;
    let (status, body) = post(&setup, None, &valid_body()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body, json!({ "error": "Unauthorized" }));
    let mut bad = valid_body();
    bad["path"] = json!("../etc/passwd");
    let (status, body) = post(&setup, Some("membertoken"), &bad).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["message"], json!("Validation error"));
}

#[tokio::test]
async fn a_flow_import_upload_gets_a_json_key_and_an_import_file_row() {
    let setup = setup(false, false).await;
    let body = json!({
        "type": "import",
        "subType": "flow",
        "workspaceId": "1",
        "fileName": "flow.json",
        "mimeType": "application/json",
    });
    let (status, body) = post(&setup, Some("membertoken"), &body).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let path = body["path"].as_str().expect("path");
    assert!(
        path.starts_with("workspaces/1/imports/flows/import_"),
        "{path}"
    );
    assert!(path.ends_with(".json"), "{path}");
    let (status, body) = post(
        &setup,
        Some("membertoken"),
        &json!({ "type": "import", "subType": "flow", "fileName": "f.json", "mimeType": "application/json" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body,
        json!({ "error": "workspaceId is required for import uploads" })
    );
}

#[tokio::test]
async fn a_workspace_less_upload_needs_a_platform_admin_unless_it_is_an_own_avatar() {
    let setup = setup(false, false).await;
    let body = json!({ "type": "generic", "subType": "generic", "path": "branding/logo.png", "fileName": "logo.png", "mimeType": "image/png" });
    assert_eq!(
        post(&setup, Some("membertoken"), &body).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, out) = post(&setup, Some("admintoken"), &body).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        out["path"],
        json!(format!("public/platform/{ADMIN_ID}/branding/logo.png"))
    );
    let avatar = json!({ "type": "generic", "subType": "generic", "path": "avatar/me.png", "fileName": "me.png", "mimeType": "image/png" });
    assert_eq!(
        post(&setup, Some("membertoken"), &avatar).await.0,
        StatusCode::OK
    );
    let mut not_image = avatar;
    not_image["mimeType"] = json!("text/plain");
    assert_eq!(
        post(&setup, Some("membertoken"), &not_image).await,
        (
            StatusCode::BAD_REQUEST,
            json!({ "error": "Avatar uploads must be an image" })
        )
    );
}

// ---- upload-handlers.test.ts ---------------------------------------------------

fn handler_input(
    workspace_id: Option<&str>,
    sub_type: &str,
    path: Option<&str>,
) -> UploadHandlerInput {
    UploadHandlerInput {
        workspace_id: workspace_id.map(str::to_string),
        user_id: None,
        file_name: "photo.png".into(),
        mime_type: "image/png".into(),
        sub_type: sub_type.into(),
        path: path.map(str::to_string),
    }
}

fn ads(
    workspace_id: Option<&str>,
    path: Option<&str>,
) -> Result<String, chatbotx_api::presigned_upload::UploadRefusal> {
    upload_path(
        UploadType::AdsCampaignCreative,
        &handler_input(workspace_id, "adsCampaignCreative", path),
    )
}

fn generic(path: &str) -> Result<String, chatbotx_api::presigned_upload::UploadRefusal> {
    upload_path(
        UploadType::Generic,
        &handler_input(Some("1"), "generic", Some(path)),
    )
}

/// Port of: apps/builder/__tests__/upload-handlers.test.ts:23 "adsCampaignCreative upload handler > accepts a path inside this workspace's ads-creative prefix"
#[test]
fn adscampaigncreative_upload_handler_accepts_a_path_inside_this_workspace_s_ads_creative_prefix() {
    assert_eq!(
        ads(
            Some("1"),
            Some("public/space/1/ads-campaign/creatives/abc123")
        ),
        Ok("public/space/1/ads-campaign/creatives/abc123".to_string())
    );
}

/// Port of: apps/builder/__tests__/upload-handlers.test.ts:38 "adsCampaignCreative upload handler > rejects a path outside the ads-creative prefix (e.g. the general public/space namespace)"
#[test]
fn adscampaigncreative_upload_handler_rejects_a_path_outside_the_ads_creative_prefix_e_g_the_general_public_space_namespace()
 {
    assert!(ads(Some("1"), Some("public/space/1/products/images/abc123")).is_err());
}

/// Port of: apps/builder/__tests__/upload-handlers.test.ts:50 "adsCampaignCreative upload handler > rejects a cross-workspace path"
#[test]
fn adscampaigncreative_upload_handler_rejects_a_cross_workspace_path() {
    assert!(
        ads(
            Some("1"),
            Some("public/space/2/ads-campaign/creatives/abc123")
        )
        .is_err()
    );
}

/// Port of: apps/builder/__tests__/upload-handlers.test.ts:62 "adsCampaignCreative upload handler > rejects when workspaceId is missing"
#[test]
fn adscampaigncreative_upload_handler_rejects_when_workspaceid_is_missing() {
    assert!(ads(None, Some("public/space/1/ads-campaign/creatives/abc123")).is_err());
}

/// Port of: apps/builder/__tests__/upload-handlers.test.ts:73 "adsCampaignCreative upload handler > rejects when path is missing"
#[test]
fn adscampaigncreative_upload_handler_rejects_when_path_is_missing() {
    assert!(ads(Some("1"), None).is_err());
}

/// Port of: apps/builder/__tests__/upload-handlers.test.ts:88 "generic upload handler — privileged-prefix guard > accepts an ordinary public/space path"
#[test]
fn generic_upload_handler_privileged_prefix_guard_accepts_an_ordinary_public_space_path() {
    assert_eq!(
        generic("public/space/1/products/images/abc123"),
        Ok("public/space/1/products/images/abc123".to_string())
    );
}

/// Port of: apps/builder/__tests__/upload-handlers.test.ts:103 "generic upload handler — privileged-prefix guard > REJECTS a path inside the super-admin-gated ads-creative prefix (authz-bypass guard)"
#[test]
fn generic_upload_handler_privileged_prefix_guard_rejects_a_path_inside_the_super_admin_gated_ads_creative_prefix_authz_bypass_guard()
 {
    assert!(generic("public/space/1/ads-campaign/creatives/abc123").is_err());
}

/// Port of: apps/builder/__tests__/upload-handlers.test.ts:117 "generic upload handler — privileged-prefix guard > REJECTS the privileged prefix regardless of letter case"
#[test]
fn generic_upload_handler_privileged_prefix_guard_rejects_the_privileged_prefix_regardless_of_letter_case()
 {
    assert!(generic("public/space/1/Ads-Campaign/Creatives/abc123").is_err());
}

/// Port of: apps/builder/__tests__/upload-handlers.test.ts:129 "generic upload handler — privileged-prefix guard > REJECTS a percent-encoded separator that decodes into the privileged prefix"
#[test]
fn generic_upload_handler_privileged_prefix_guard_rejects_a_percent_encoded_separator_that_decodes_into_the_privileged_prefix()
 {
    for path in [
        "public/space/1/ads-campaign%2Fcreatives%2Fabc123",
        "public/space/1/ads-campaign%2fcreatives%2fabc123",
    ] {
        assert!(generic(path).is_err(), "{path}");
    }
}
