//! P3.6-D2 connections rest: the AI key credential connects
//! (`chatbotx_business::connection::credentials`). Ports of
//! `packages/business/src/connection/__tests__/credential-providers.test.ts`
//! (the AI API-key provider block) and the credential row of
//! `apps/builder/__tests__/connections-public-api.test.ts`, over a migrated
//! database and a scripted upstream (the original mocks `ky` and the
//! verify helper).

mod common;

use std::sync::Mutex;

use chatbotx_business::connection::credentials::{
    self, KeyValidation, Upstream, UpstreamReply, UpstreamRequest, ai_key_provider, connect_ai_key,
    parse_config, verify_ai_key_connection,
};
use common::{WS, scalar_i64, two_workspaces};
use serde_json::{Map, Value, json};

/// Answers each request with the next scripted reply and records it.
#[derive(Default)]
struct Script {
    replies: Mutex<Vec<UpstreamReply>>,
    seen: Mutex<Vec<UpstreamRequest>>,
}

impl Script {
    fn with(replies: Vec<UpstreamReply>) -> Script {
        Script {
            replies: Mutex::new(replies.into_iter().rev().collect()),
            seen: Mutex::new(Vec::new()),
        }
    }
    fn seen(&self) -> Vec<UpstreamRequest> {
        self.seen.lock().expect("lock").clone()
    }
}

impl Upstream for Script {
    async fn send(&self, request: UpstreamRequest) -> UpstreamReply {
        self.seen.lock().expect("lock").push(request);
        self.replies
            .lock()
            .expect("lock")
            .pop()
            .unwrap_or_else(|| Err("no scripted reply".into()))
    }
}

fn ok() -> UpstreamReply {
    Ok((200, r#"{"object":"list","data":[]}"#.into()))
}

fn config(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("object")
}

const AI_PROVIDERS: [&str; 6] = [
    "claude",
    "deepseek",
    "gemini",
    "muse",
    "openai",
    "openrouter",
];

/// Port of: packages/business/src/connection/__tests__/credential-providers.test.ts:162 "AI API-key provider: %s > rejects a credential when verification is inconclusive"
#[tokio::test]
async fn ai_api_key_provider_rejects_a_credential_when_verification_is_inconclusive() {
    for name in AI_PROVIDERS {
        let db = two_workspaces().await;
        let provider = ai_key_provider(name).expect("provider");
        let upstream = Script::with(vec![Ok((500, "{}".into()))]);
        let error = connect_ai_key(
            &db,
            &upstream,
            WS,
            provider,
            &config(json!({ "apiKey": "possibly-valid-key" })),
            None,
        )
        .await
        .expect_err(name);
        // `fromCredentials` throws "Unable to verify …"; the connect path
        // maps the inconclusive check to 502 `connectionProviderUnavailable`.
        assert_eq!(
            error.code(),
            Some("connectionProviderUnavailable"),
            "{name}"
        );
        assert_eq!(
            scalar_i64(
                &db,
                r#"SELECT count(*) FROM "Connection" WHERE "workspaceId" = ?1"#,
                WS
            )
            .await,
            Some(0)
        );
    }
}

/// Port of: packages/business/src/connection/__tests__/credential-providers.test.ts:174 "AI API-key provider: %s > marks invalid credentials revoked and unknown verification degraded"
#[tokio::test]
async fn ai_api_key_provider_marks_invalid_credentials_revoked_and_unknown_verification_degraded() {
    for name in AI_PROVIDERS {
        let db = two_workspaces().await;
        let provider = ai_key_provider(name).expect("provider");
        let upstream = Script::with(vec![ok(), Ok((500, "{}".into())), Ok((401, "{}".into()))]);
        let row = connect_ai_key(
            &db,
            &upstream,
            WS,
            provider,
            &config(json!({ "apiKey": "sk-1" })),
            None,
        )
        .await
        .expect(name);
        // `verify` returns `{ ok: false, revoked: false }` for an unknown
        // check (degraded) and `{ ok: false, revoked: true }` for an invalid
        // key (needs_reauth); the outcome is read from the connection.
        let degraded = verify_ai_key_connection(&db, &upstream, provider, row)
            .await
            .expect("verify");
        assert_eq!(
            (degraded.status.as_str(), degraded.status_reason.as_deref()),
            ("degraded", Some("verify_failed")),
            "{name}"
        );
        let revoked = verify_ai_key_connection(&db, &upstream, provider, degraded)
            .await
            .expect("verify");
        assert_eq!(
            (revoked.status.as_str(), revoked.status_reason.as_deref()),
            ("needs_reauth", Some("token_revoked")),
            "{name}"
        );
    }
}

/// Port of: apps/builder/__tests__/connections-public-api.test.ts:389 "POST /v1/connections > connects a credential-strategy provider immediately and returns connection, session: null"
#[tokio::test]
async fn post_v1_connections_connects_a_credential_strategy_provider_immediately_and_returns_connection_session_null()
 {
    let db = two_workspaces().await;
    let upstream = Script::with(vec![ok()]);
    let row = connect_ai_key(
        &db,
        &upstream,
        WS,
        ai_key_provider("claude").expect("claude"),
        &config(json!({ "apiKey": "sk-live" })),
        None,
    )
    .await
    .expect("connect");
    // The handler wraps the row as `{ connection, session: null, secret:
    // null }`; here the real row stands in for the mocked service result.
    assert_eq!(
        (
            row.kind.as_str(),
            row.provider.as_str(),
            row.status.as_str(),
            row.source_id.as_str(),
            row.display_name.as_str()
        ),
        ("integration", "claude", "connected", "workspace", "Claude")
    );
    let seen = upstream.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].url, "https://api.anthropic.com/v1/models");
    assert!(
        seen[0]
            .headers
            .contains(&("x-api-key".to_string(), "sk-live".to_string()))
    );
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT "maxOutputTokens" FROM "IntegrationClaude" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(1024)
    );
}

// ---- own tests (parity behaviour of p1-05 03-connections-api-key) ----------

#[test]
fn parse_config_requires_each_field_as_a_string() {
    let fields = credentials::config_fields("deepseek");
    assert_eq!(
        parse_config(fields, &config(json!({})))
            .expect_err("missing")
            .to_string(),
        "validation: apiKey is required"
    );
    assert_eq!(
        parse_config(fields, &config(json!({ "apiKey": 42 })))
            .expect_err("number")
            .to_string(),
        "validation: apiKey must be a string"
    );
    assert!(parse_config(fields, &config(json!({ "apiKey": "sk" }))).is_ok());
}

#[tokio::test]
async fn a_second_connect_is_409_an_unknown_config_key_is_422_and_disconnect_removes_the_row() {
    let db = two_workspaces().await;
    let deepseek = ai_key_provider("deepseek").expect("deepseek");
    let upstream = Script::with(vec![ok(), ok(), ok()]);
    let error = connect_ai_key(
        &db,
        &upstream,
        WS,
        deepseek,
        &config(json!({ "apiKey": "sk-1", "workspaceId": "1" })),
        None,
    )
    .await
    .expect_err("unknown key");
    assert_eq!(
        error.to_string(),
        "validation: Unsupported config field(s) for deepseek: workspaceId"
    );
    assert!(
        upstream.seen().is_empty(),
        "no upstream call before the config check"
    );
    let first = connect_ai_key(
        &db,
        &upstream,
        WS,
        deepseek,
        &config(json!({ "apiKey": "sk-1" })),
        None,
    )
    .await
    .expect("connect");
    let error = connect_ai_key(
        &db,
        &upstream,
        WS,
        deepseek,
        &config(json!({ "apiKey": "sk-2" })),
        None,
    )
    .await
    .expect_err("second");
    assert_eq!(error.code(), Some("connectionAlreadyConnected"));
    let gone = credentials::disconnect_integration(&db, first.clone())
        .await
        .expect("disconnect");
    assert_eq!(gone.status, "disconnected");
    assert!(gone.disconnected_at.is_some());
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "Connection" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(0)
    );
    let again = connect_ai_key(
        &db,
        &upstream,
        WS,
        deepseek,
        &config(json!({ "apiKey": "sk-3" })),
        None,
    )
    .await
    .expect("reconnect");
    assert_ne!(again.id, first.id);
}

#[tokio::test]
async fn a_rejected_key_is_400_and_writes_nothing() {
    let db = two_workspaces().await;
    let upstream = Script::with(vec![Ok((401, "{}".into()))]);
    let error = connect_ai_key(
        &db,
        &upstream,
        WS,
        ai_key_provider("deepseek").expect("deepseek"),
        &config(json!({ "apiKey": "sk-bad" })),
        None,
    )
    .await
    .expect_err("rejected");
    assert_eq!(error.code(), Some("connectionCredentialsRejected"));
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "Integration" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(0)
    );
    assert_eq!(
        credentials::verify_ai_key(
            &Script::with(vec![Err("timeout".into())]),
            &ai_key_provider("openai").expect("openai"),
            "k"
        )
        .await,
        KeyValidation::Unknown
    );
}

#[tokio::test]
async fn a_needs_reauth_connection_is_inactive_for_verify_and_refresh() {
    let db = two_workspaces().await;
    let provider = ai_key_provider("deepseek").expect("provider");
    let upstream = Script::with(vec![ok(), Ok((401, "{}".into()))]);
    let row = connect_ai_key(
        &db,
        &upstream,
        WS,
        provider,
        &config(json!({ "apiKey": "sk-1" })),
        None,
    )
    .await
    .expect("connect");
    let revoked = verify_ai_key_connection(&db, &upstream, provider, row)
        .await
        .expect("verify");
    assert_eq!(revoked.status, "needs_reauth");
    // `ACTIVE_CONNECTION_STATUSES` is `connected | degraded` only.
    for result in [
        chatbotx_business::connection::verify(&db, revoked.id, WS).await,
        chatbotx_business::connection::refresh(&db, revoked.id, WS).await,
    ] {
        assert_eq!(
            result.expect_err("inactive").code(),
            Some("connectionInactive")
        );
    }
}
