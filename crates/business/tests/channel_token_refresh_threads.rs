//! Port of `apps/worker/__tests__/refresh-threads-tokens.test.ts`: the
//! Threads leg of the `refreshChannelTokens` schedule over a migrated
//! database and a scripted provider.
//! ponytail: the stale compare-and-set case needs a reconnect between the
//! read and the write, which this harness cannot inject; the CAS is the
//! `accessToken` guard of the UPDATE.

mod channel_fake;
mod common;

use channel_fake::{
    FakeHttp, auth_row, channel_row, connection_row, connection_status, graph_error, ok,
};
use chatbotx_business::audit::AuditService;
use chatbotx_business::channel_token_refresh::{RefreshDeps, refresh_locks, refresh_scheduled};
use common::{OTHER_WS, WS, setup, two_workspaces};
use serde_json::{Value, json};

/// The schedule lock is process-wide: one Threads run at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn iso_in(ms: i64) -> String {
    let now = chatbotx_db::repo::now_ms();
    chatbotx_core::js::to_iso_string(now + ms).expect("iso")
}

fn auth(token: &str, expires_at: Option<String>) -> Value {
    let mut tokens = json!({ "accessToken": token });
    if let Some(at) = expires_at {
        tokens["expiresAt"] = json!(at);
    }
    json!({ "authType": "oauth2", "tokens": tokens, "metadata": { "version": "v1.0" } })
}

async fn run(db: &chatbotx_db::Db, http: &FakeHttp) {
    let audit = AuditService::disabled();
    let deps = RefreshDeps {
        db,
        http,
        audit: &audit,
        buses: None,
    };
    refresh_scheduled(deps, Some(&["threads".to_owned()])).await;
}

#[tokio::test]
async fn runs_under_the_distributed_lock_and_refreshes_due_integrations() {
    let _serial = SERIAL.lock().await;
    let db = two_workspaces().await;
    setup(&db, |tx| {
        channel_row(
            tx,
            "IntegrationThreads",
            81,
            WS,
            "t-81",
            &auth("token-1", Some(iso_in(DAY_MS))),
        )?;
        // Not due: expires after the 14-day window.
        channel_row(
            tx,
            "IntegrationThreads",
            82,
            WS,
            "t-82",
            &auth("token-2", Some(iso_in(30 * DAY_MS))),
        )?;
        Ok(())
    })
    .await;
    let http = FakeHttp::new(|_| {
        ok(json!({ "access_token": "refreshed-token", "expires_in": 5_184_000 }))
    });

    // Held lock: the run is skipped.
    let held = refresh_locks()
        .try_acquire("schedule:refresh-threads-tokens")
        .expect("hold the schedule lock");
    run(&db, &http).await;
    drop(held);
    assert!(http.seen().is_empty());

    run(&db, &http).await;
    let urls = http.urls();
    assert_eq!(urls.len(), 1);
    assert!(urls[0].starts_with("https://graph.threads.com/refresh_access_token?"));
    assert!(urls[0].contains("grant_type=th_refresh_token"));
    assert!(urls[0].contains("access_token=token-1"));
    let (refreshed, error) = auth_row(&db, "IntegrationThreads", 81).await;
    assert_eq!(error, None);
    assert_eq!(refreshed["tokens"]["accessToken"], "refreshed-token");
    assert_eq!(refreshed["metadata"], json!({ "version": "v1.0" }));
    let expires =
        chatbotx_core::js::parse_date(refreshed["tokens"]["expiresAt"].as_str().unwrap()).unwrap();
    assert!(expires > chatbotx_db::repo::now_ms() + 59 * DAY_MS);
    assert_eq!(
        auth_row(&db, "IntegrationThreads", 82).await.0["tokens"]["accessToken"],
        "token-2"
    );
}

#[tokio::test]
async fn isolates_per_integration_failures_and_continues_refreshing_later_rows() {
    let _serial = SERIAL.lock().await;
    let db = two_workspaces().await;
    setup(&db, |tx| {
        channel_row(
            tx,
            "IntegrationThreads",
            83,
            WS,
            "t-83",
            &auth("token-1", None),
        )?;
        channel_row(
            tx,
            "IntegrationThreads",
            84,
            OTHER_WS,
            "t-84",
            &auth("token-2", None),
        )?;
        connection_row(tx, 83, WS, "threads", 83, "t-83")?;
        Ok(())
    })
    .await;
    let http = FakeHttp::new(|r| {
        if r.url.contains("access_token=token-1") {
            graph_error(401, "revoked", 190, Some(460))
        } else {
            ok(json!({ "access_token": "refreshed-token-2", "expires_in": 3600 }))
        }
    });
    run(&db, &http).await;
    assert_eq!(
        auth_row(&db, "IntegrationThreads", 83).await.1.as_deref(),
        Some("revoked")
    );
    assert_eq!(
        connection_status(&db, 83).await,
        ("needs_reauth".into(), Some("token_revoked".into()))
    );
    let (second, error) = auth_row(&db, "IntegrationThreads", 84).await;
    assert_eq!(error, None);
    assert_eq!(second["tokens"]["accessToken"], "refreshed-token-2");
}
