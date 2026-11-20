//! Port of `apps/worker/__tests__/refresh-tiktok-tokens.test.ts`: the TikTok
//! leg of the `refreshChannelTokens` schedule over a migrated database and
//! a scripted provider.

mod channel_fake;
mod common;

use channel_fake::{
    FakeHttp, auth_row, channel_row, connection_row, connection_status, ok, status, text,
};
use chatbotx_business::audit::AuditService;
use chatbotx_business::channel_token_refresh::{RefreshDeps, refresh_scheduled};
use chatbotx_business::connection::credentials::UpstreamReply;
use common::{OTHER_WS, WS, setup, two_workspaces};
use serde_json::{Value, json};

const TABLE: &str = "IntegrationTiktok";

fn make_auth(token: &str) -> Value {
    json!({
        "authType": "oauth2",
        "clientId": "client",
        "clientSecret": "secret",
        "tokens": { "accessToken": token, "refreshToken": format!("refresh-{token}") },
        "metadata": {},
    })
}

fn refreshed(n: &str) -> UpstreamReply {
    ok(json!({ "code": 0, "data": {
        "access_token": format!("new-token{n}"),
        "refresh_token": format!("new-refresh{n}"),
        "expires_in": 3600,
        "refresh_token_expires_in": 86_400,
        "scope": "user.info.basic",
    }}))
}

/// Rows `base + 1` (`WS`) and, with `two`, `base + 2` (`OTHER_WS`).
async fn seed(base: i64, two: bool, auth: fn(i64) -> Value) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        for (n, ws) in [(1, WS), (2, OTHER_WS)]
            .into_iter()
            .take(if two { 2 } else { 1 })
        {
            let id = base + n;
            channel_row(tx, TABLE, id, ws, &format!("open-{id}"), &auth(n))?;
            connection_row(tx, id, ws, "tiktok", id, &format!("open-{id}"))?;
        }
        Ok(())
    })
    .await;
    db
}

async fn run(db: &chatbotx_db::Db, http: &FakeHttp) {
    let audit = AuditService::disabled();
    let deps = RefreshDeps {
        db,
        http,
        audit: &audit,
        buses: None,
    };
    refresh_scheduled(deps, Some(&["tiktok".to_owned()])).await;
}

#[tokio::test]
async fn refreshes_and_writes_the_new_tokens_and_the_parsed_scopes() {
    let db = seed(76_000, false, |_| make_auth("old-token")).await;
    let http = FakeHttp::new(|_| refreshed(""));
    run(&db, &http).await;
    let (auth, error) = auth_row(&db, TABLE, 76_001).await;
    assert_eq!(error, None);
    assert_eq!(auth["tokens"]["accessToken"], "new-token");
    assert_eq!(auth["tokens"]["refreshToken"], "new-refresh");
    assert_eq!(auth["metadata"]["scopes"], json!(["user.info.basic"]));
    let body: Value = serde_json::from_str(http.seen()[0].body.as_deref().unwrap()).unwrap();
    assert_eq!(body["refresh_token"], "refresh-old-token");
    assert_eq!(body["grant_type"], "refresh_token");
}

#[tokio::test]
async fn marks_the_connection_unhealthy_when_the_provider_confirms_the_token_was_revoked() {
    let db = seed(76_100, false, |_| make_auth("old-token")).await;
    let http = FakeHttp::new(|_| status(401, json!({})));
    run(&db, &http).await;
    assert_eq!(
        auth_row(&db, TABLE, 76_101).await.1.as_deref(),
        Some("Unauthorized")
    );
    assert_eq!(
        connection_status(&db, 76_101).await,
        ("needs_reauth".into(), Some("token_revoked".into()))
    );
}

#[tokio::test]
async fn degrades_on_a_transient_refresh_failure() {
    let db = seed(76_200, false, |_| make_auth("old-token")).await;
    let http = FakeHttp::new(|_| ok(json!({ "code": 40100, "message": "rate limited" })));
    run(&db, &http).await;
    assert_eq!(
        auth_row(&db, TABLE, 76_201).await.1.as_deref(),
        Some("rate limited")
    );
    assert_eq!(
        connection_status(&db, 76_201).await,
        ("degraded".into(), Some("refresh_failed".into()))
    );
}

#[tokio::test]
async fn isolates_a_row_1_failure_row_2_still_refreshes_in_the_same_batch() {
    let db = seed(76_300, true, |n| make_auth(&format!("old-token-{n}"))).await;
    let http = FakeHttp::new(|r| {
        if text(r).contains("refresh-old-token-1") {
            ok(json!({ "code": 1, "message": "boom" }))
        } else {
            refreshed("-2")
        }
    });
    run(&db, &http).await;
    assert_eq!(
        auth_row(&db, TABLE, 76_301).await.1.as_deref(),
        Some("boom")
    );
    let (second, error) = auth_row(&db, TABLE, 76_302).await;
    assert_eq!(error, None);
    assert_eq!(second["tokens"]["accessToken"], "new-token-2");
}

#[tokio::test]
async fn skips_a_row_with_no_refresh_token_without_marking_an_error() {
    let db = seed(
        76_400,
        false,
        |_| json!({ "authType": "oauth2", "tokens": { "accessToken": "old-token" } }),
    )
    .await;
    let http = FakeHttp::new(|_| refreshed(""));
    run(&db, &http).await;
    assert!(http.seen().is_empty());
    let (auth, error) = auth_row(&db, TABLE, 76_401).await;
    assert_eq!(error, None);
    assert_eq!(auth["tokens"]["accessToken"], "old-token");
}
