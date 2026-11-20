//! Ports of `apps/builder/__tests__/email-topic-open-route.test.ts` and
//! `email-topic-click-route.test.ts` (Q.P6-U5). Adapted: the mocked
//! services become a real recipient row behind
//! `email_topic_routes::router`; "recordOpen / recordClick called" becomes
//! the recipient's counters. The leaderboard award is covered in
//! `crates/business/tests/email_tracking.rs` (no queue in this fixture).

mod rpc_common;

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chatbotx_api::email_topic_routes::router;
use chatbotx_business::email_tracking::{NewRecipient, create_recipient, sign_click_url};
use chatbotx_db::rusqlite::params;
use rpc_common::{Fixture, WS, fixture};
use serde_json::{Value, json};
use tower::ServiceExt as _;

struct Answer {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    bytes: Vec<u8>,
}

impl Answer {
    fn header(&self, name: header::HeaderName) -> &str {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
    }
}

async fn get(fx: &Fixture, uri: &str) -> Answer {
    let response = router(Arc::clone(&fx.api))
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(header::HOST, "app.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Answer {
        status,
        headers,
        bytes,
    }
}

/// A recipient of topic 71 (no contact).
async fn token(fx: &Fixture) -> String {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "EmailTopic" ("id", "name", "workspaceId") VALUES (71, 'News', ?1)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    create_recipient(
        fx.db(),
        NewRecipient {
            topic_id: Some(71),
            workspace_id: WS,
            email: "ada@example.com".into(),
            ..NewRecipient::default()
        },
    )
    .await
    .unwrap()
}

/// `(seenCount, clickCount)` of the recipient.
async fn counts(fx: &Fixture, token: &str) -> (i64, i64) {
    let token = token.to_owned();
    fx.db()
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "seenCount", "clickCount" FROM "AnalyticsEmailTopic" WHERE "token" = ?1"#,
                [token],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap()
}

async fn schedule_deletion(fx: &Fixture) {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = ?1"#,
                [WS],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

// ---- email-topic-open-route.test.ts ---------------------------------------------

#[tokio::test]
async fn open_returns_a_1x1_gif_with_no_cache_headers_and_records_the_open() {
    let fx = fixture().await;
    let token = token(&fx).await;
    let answer = get(&fx, &format!("/email-topic/open?r={token}")).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.header(header::CONTENT_TYPE), "image/gif");
    assert_eq!(
        answer.header(header::CACHE_CONTROL),
        "no-store, no-cache, must-revalidate"
    );
    assert!(answer.bytes.starts_with(b"GIF89a"));
    assert_eq!(counts(&fx, &token).await, (1, 0));
    // Without `r` nothing is recorded.
    assert_eq!(get(&fx, "/email-topic/open").await.status, StatusCode::OK);
    assert_eq!(counts(&fx, &token).await, (1, 0));
}

#[tokio::test]
async fn open_returns_the_gif_but_skips_the_open_for_a_workspace_scheduled_for_deletion() {
    let fx = fixture().await;
    let token = token(&fx).await;
    schedule_deletion(&fx).await;
    let answer = get(&fx, &format!("/email-topic/open?r={token}")).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert!(answer.bytes.starts_with(b"GIF89a"));
    assert_eq!(counts(&fx, &token).await, (0, 0));
}

// ---- email-topic-click-route.test.ts --------------------------------------------

#[tokio::test]
async fn click_redirects_to_the_verified_destination_and_records_the_click() {
    let fx = fixture().await;
    let token = token(&fx).await;
    let signed = sign_click_url("https://example.com/landing", WS).unwrap();
    let answer = get(&fx, &format!("/email-topic/click?r={token}&u={signed}")).await;
    assert_eq!(answer.status, StatusCode::FOUND);
    assert_eq!(
        answer.header(header::LOCATION),
        "https://example.com/landing"
    );
    assert_eq!(counts(&fx, &token).await, (0, 1));
}

#[tokio::test]
async fn click_redirects_to_the_same_origin_for_a_tampered_or_missing_destination() {
    let fx = fixture().await;
    let token = token(&fx).await;
    let tampered = get(&fx, &format!("/email-topic/click?r={token}&u=bogus")).await;
    assert_eq!(tampered.status, StatusCode::FOUND);
    assert_eq!(tampered.header(header::LOCATION), "http://app.test/");
    // A tampered `u` records no click (the original only records with a
    // verified destination or without `u`).
    assert_eq!(counts(&fx, &token).await, (0, 0));
    let missing = get(&fx, &format!("/email-topic/click?r={token}")).await;
    assert_eq!(missing.header(header::LOCATION), "http://app.test/");
    assert_eq!(counts(&fx, &token).await, (0, 1));
}

#[tokio::test]
async fn click_returns_410_for_a_workspace_scheduled_for_deletion() {
    let fx = fixture().await;
    let token = token(&fx).await;
    let signed = sign_click_url("https://example.com/landing", WS).unwrap();
    schedule_deletion(&fx).await;
    for uri in [
        format!("/email-topic/click?r={token}&u={signed}"),
        // A `u`-only token of a scheduled workspace.
        format!("/email-topic/click?u={signed}"),
    ] {
        let answer = get(&fx, &uri).await;
        assert_eq!(answer.status, StatusCode::GONE, "{uri}");
        assert_eq!(
            serde_json::from_slice::<Value>(&answer.bytes).unwrap(),
            json!({ "code": "workspaceScheduledDeletion" })
        );
    }
    assert_eq!(counts(&fx, &token).await, (0, 0));
}

#[tokio::test]
async fn click_checks_the_signed_tokens_workspace_without_a_tracking_token() {
    let fx = fixture().await;
    let signed = sign_click_url("https://example.com/landing", WS).unwrap();
    let answer = get(&fx, &format!("/email-topic/click?u={signed}")).await;
    assert_eq!(answer.status, StatusCode::FOUND);
    assert_eq!(
        answer.header(header::LOCATION),
        "https://example.com/landing"
    );
}

#[tokio::test]
async fn click_does_not_redirect_when_the_tracking_and_signed_tokens_disagree() {
    let fx = fixture().await;
    let token = token(&fx).await;
    let signed = sign_click_url("https://evil.example/", WS + 1).unwrap();
    let answer = get(&fx, &format!("/email-topic/click?r={token}&u={signed}")).await;
    assert_eq!(answer.status, StatusCode::FOUND);
    assert_eq!(answer.header(header::LOCATION), "http://app.test/");
    assert_eq!(counts(&fx, &token).await, (0, 0));
}
