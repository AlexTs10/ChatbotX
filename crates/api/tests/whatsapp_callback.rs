//! Q.P6-U2: `GET /integrations/whatsapp/callback`. Ports
//! `apps/builder/__tests__/whatsapp-oauth-callback-outcome.test.ts`, the
//! `sanitizeReferer` block of `oauth-referer.test.ts` and the state block of
//! `whatsapp-embedded-signup.test.ts` (real `CustomDomain` rows in place of
//! the mocked `findActiveByDomain`; the fixture app URL is `http://app`),
//! plus the relay page itself. Q.P7-V4 adds the `sanitizeOptionalReturnUrl`,
//! `resolveRelayTarget` and "with a dedicated broker host" blocks (the broker
//! origin is passed explicitly in place of `NEXT_PUBLIC_BROKER_URL`; without
//! one it is the builder origin).

mod handlers_common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use base64::Engine as _;
use chatbotx_api::whatsapp_callback::{
    FALLBACK_REDIRECT, decode_state, outcome, relay_html, resolve_relay_target, safe_json,
    sanitize_optional_return_url, sanitize_referer, sanitize_referer_via,
};
use handlers_common::fixture;
use tower::ServiceExt as _;

fn state_of(value: serde_json::Value) -> String {
    base64::engine::general_purpose::STANDARD.encode(value.to_string())
}

async fn active_domain(fx: &handlers_common::Fixture, domain: &'static str, status: &'static str) {
    fx.state
        .db
        .write(move |tx| {
            tx.execute(r#"INSERT INTO "Tenant" ("id") VALUES (77)"#, [])?;
            tx.execute(
                r#"INSERT INTO "CustomDomain" ("id", "tenantId", "domain", "status") VALUES (1, 77, ?1, ?2)"#,
                [domain, status],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

#[test]
fn whatsapp_callback_outcome_a_code_relays_success_and_says_signup_is_done() {
    assert_eq!(
        outcome(Some("abc"), None),
        (
            r#"{"type":"WA_OAUTH_RESULT","status":"success","code":"abc"}"#.into(),
            0
        )
    );
}

#[test]
fn whatsapp_callback_outcome_access_denied_relays_a_cancellation_and_never_claims_signup_completed()
{
    assert_eq!(
        outcome(None, Some("access_denied")),
        (
            r#"{"type":"WA_OAUTH_RESULT","status":"cancelled"}"#.into(),
            1
        )
    );
}

#[test]
fn whatsapp_callback_outcome_any_other_provider_error_relays_a_failure() {
    assert_eq!(
        outcome(None, Some("server_error")),
        (r#"{"type":"WA_OAUTH_RESULT","status":"error"}"#.into(), 2)
    );
}

#[test]
fn whatsapp_callback_outcome_an_error_wins_over_a_code_that_came_with_it() {
    assert_eq!(outcome(Some("abc"), Some("server_error")).1, 2);
}

#[test]
fn whatsapp_callback_outcome_no_code_and_no_error_is_a_failure_not_a_success() {
    assert_eq!(outcome(None, None).1, 2);
}

#[test]
fn decode_oauth_state_round_trips_referer_and_locale() {
    let encoded =
        state_of(serde_json::json!({ "referer": "https://chat.acme.com", "locale": "en" }));
    assert_eq!(
        decode_state(&encoded),
        Some(("https://chat.acme.com".into(), Some("en".into())))
    );
}

#[test]
fn decode_oauth_state_rejects_malformed_state() {
    assert_eq!(decode_state("not-base64-json"), None);
}

#[test]
fn decode_oauth_state_rejects_state_without_a_referer() {
    assert_eq!(
        decode_state(&state_of(serde_json::json!({ "locale": "en" }))),
        None
    );
}

#[tokio::test]
async fn sanitize_referer_accepts_the_platform_origin() {
    let fx = fixture().await;
    let referer = "http://app/space/42/dashboard";
    assert_eq!(sanitize_referer(&fx.state, referer).await, referer);
}

#[tokio::test]
async fn sanitize_referer_accepts_a_referer_that_maps_to_an_active_custom_domain() {
    let fx = fixture().await;
    active_domain(&fx, "chat.acme.com", "active").await;
    let referer = "https://chat.acme.com/space/42/dashboard";
    assert_eq!(sanitize_referer(&fx.state, referer).await, referer);
}

#[tokio::test]
async fn sanitize_referer_falls_back_when_the_referer_is_not_an_active_custom_domain() {
    let fx = fixture().await;
    active_domain(&fx, "chat.acme.com", "pending").await;
    for referer in ["https://evil.example.org/steal", "https://chat.acme.com/x"] {
        assert_eq!(
            sanitize_referer(&fx.state, referer).await,
            FALLBACK_REDIRECT
        );
    }
}

#[tokio::test]
async fn sanitize_referer_falls_back_when_the_referer_is_not_a_valid_url() {
    let fx = fixture().await;
    assert_eq!(
        sanitize_referer(&fx.state, "not-a-url").await,
        FALLBACK_REDIRECT
    );
}

#[test]
fn the_relay_script_cannot_be_broken_out_of() {
    assert_eq!(
        safe_json("\"</script>\u{2028}\u{2029}\""),
        "\"\\u003c/script>\\u2028\\u2029\""
    );
    let html = relay_html(
        r#"{"type":"WA_OAUTH_RESULT","status":"success","code":"</script><b>"}"#,
        "https://chat.acme.com",
        "<b>&\"",
    );
    assert!(!html.contains("</script><b>"));
    assert!(html.contains("<p>&lt;b&gt;&amp;&quot;</p>"));
    assert!(html.contains(r#"window.opener.postMessage({"type":"WA_OAUTH_RESULT","status":"success","code":"\u003c/script>\u003cb>"}, "https://chat.acme.com");"#));
}

async fn get(
    fx: &handlers_common::Fixture,
    query: &str,
) -> (StatusCode, axum::http::HeaderMap, String) {
    let app = chatbotx_api::connect_callback::router(Arc::new(fx.state.clone()));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/integrations/whatsapp/callback?{query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}

/// The route relays the code to the referer origin with the locked-down
/// headers and the localized text; an invalid state or a foreign referer is
/// a 400 and relays nothing.
#[tokio::test]
async fn the_callback_relays_the_code_to_an_allowed_origin_only() {
    let fx = fixture().await;
    active_domain(&fx, "chat.acme.com", "active").await;
    let state = state_of(
        serde_json::json!({ "referer": "https://chat.acme.com/space/1/whatsapp", "locale": "de" }),
    );
    let (status, headers, body) = get(&fx, &format!("code=abc&state={state}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8");
    assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(headers[header::REFERRER_POLICY], "no-referrer");
    assert!(
        headers[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    assert!(body.contains(r#"postMessage({"type":"WA_OAUTH_RESULT","status":"success","code":"abc"}, "https://chat.acme.com")"#), "{body}");
    assert!(body.contains("Registrierung abgeschlossen"));

    let state = state_of(serde_json::json!({ "referer": "http://app/x", "locale": "xx" }));
    let (_, _, body) = get(&fx, &format!("error=access_denied&state={state}")).await;
    assert!(body.contains("Signup was cancelled"));
    assert!(body.contains(r#""http://app")"#));

    let foreign = state_of(serde_json::json!({ "referer": "https://evil.example.org" }));
    for query in [
        "code=abc".to_string(),
        "code=abc&state=bad".into(),
        format!("code=abc&state={foreign}"),
    ] {
        let (status, _, body) = get(&fx, &query).await;
        assert_eq!(
            (status, body.as_str()),
            (StatusCode::BAD_REQUEST, "Invalid request")
        );
    }
}

// ---- sanitizeOptionalReturnUrl / resolveRelayTarget (Q.P7-V4) -----------

const PLATFORM: &str = "http://app";
const BROKER: &str = "https://auth.broker.example";

fn url(raw: &str) -> url::Url {
    url::Url::parse(raw).unwrap()
}

#[tokio::test]
async fn sanitize_optional_return_url_refuses_missing_malformed_and_foreign_urls() {
    let fx = fixture().await;
    for raw in [
        None,
        Some(""),
        Some("not-a-url"),
        Some("javascript:alert(document.cookie)"),
        Some("//evil.com/path"),
        Some("https://evil.example.org/steal"),
    ] {
        assert_eq!(
            sanitize_optional_return_url(&fx.state, PLATFORM, raw).await,
            None,
            "{raw:?}"
        );
    }
}

#[tokio::test]
async fn sanitize_optional_return_url_accepts_the_platform_and_an_active_custom_domain() {
    let fx = fixture().await;
    active_domain(&fx, "chat.acme.com", "active").await;
    for raw in [
        "http://app/connections/success",
        "https://chat.acme.com/connections/success",
    ] {
        assert_eq!(
            sanitize_optional_return_url(&fx.state, PLATFORM, Some(raw))
                .await
                .as_deref(),
            Some(raw)
        );
    }
}

#[tokio::test]
async fn resolve_relay_target_relays_to_the_originating_custom_domain_only() {
    let fx = fixture().await;
    active_domain(&fx, "chat.acme.com", "active").await;
    let callback = url("http://app/integrations/messenger/callback?code=abc&state=xyz");
    assert_eq!(
        resolve_relay_target(
            &fx.state,
            PLATFORM,
            &callback,
            "https://chat.acme.com/space/42/dashboard"
        )
        .await
        .as_deref(),
        Some("https://chat.acme.com/integrations/messenger/callback?code=abc&state=xyz")
    );
    // Already on the originating domain, the platform itself, an unknown
    // domain, an invalid referer: no relay.
    let on_domain = url("https://chat.acme.com/integrations/messenger/callback?code=abc");
    let on_platform = url("http://app/integrations/messenger/callback?code=abc");
    for (callback, referer) in [
        (&on_domain, "https://chat.acme.com/space/42/dashboard"),
        (&on_platform, "http://app/space/42/dashboard"),
        (&on_platform, "https://evil.example.org/space/42/dashboard"),
        (&on_platform, "not-a-url"),
    ] {
        assert_eq!(
            resolve_relay_target(&fx.state, PLATFORM, callback, referer).await,
            None,
            "{referer}"
        );
    }
}

#[tokio::test]
async fn with_a_dedicated_broker_host_both_hosts_are_ours() {
    let fx = fixture().await;
    active_domain(&fx, "chat.acme.com", "active").await;
    for referer in [
        format!("{BROKER}/welcome"),
        "http://app/space/42/dashboard".to_owned(),
    ] {
        assert_eq!(
            sanitize_referer_via(&fx.state, BROKER, &referer).await,
            referer
        );
    }
    let back = format!("{BROKER}/connections/success");
    assert_eq!(
        sanitize_optional_return_url(&fx.state, BROKER, Some(&back))
            .await
            .as_deref(),
        Some(back.as_str())
    );

    // From the broker to the builder URL and to an active custom domain;
    // from the builder host too (relay is not broker-only).
    let google = url(&format!(
        "{BROKER}/api/auth/callback/google?code=abc&state=xyz"
    ));
    assert_eq!(
        resolve_relay_target(&fx.state, BROKER, &google, "http://app/space/42/dashboard")
            .await
            .as_deref(),
        Some("http://app/api/auth/callback/google?code=abc&state=xyz")
    );
    let tiktok = url(&format!(
        "{BROKER}/integrations/tiktok/callback?code=abc&state=xyz"
    ));
    assert_eq!(
        resolve_relay_target(
            &fx.state,
            BROKER,
            &tiktok,
            "https://chat.acme.com/space/42/dashboard"
        )
        .await
        .as_deref(),
        Some("https://chat.acme.com/integrations/tiktok/callback?code=abc&state=xyz")
    );
    let platform = url("http://app/integrations/tiktok/callback?code=abc");
    assert_eq!(
        resolve_relay_target(
            &fx.state,
            BROKER,
            &platform,
            "https://chat.acme.com/space/42/dashboard"
        )
        .await
        .as_deref(),
        Some("https://chat.acme.com/integrations/tiktok/callback?code=abc")
    );
    // Never back to the broker itself.
    let on_broker = url(&format!("{BROKER}/integrations/tiktok/callback?code=abc"));
    assert_eq!(
        resolve_relay_target(&fx.state, BROKER, &on_broker, &format!("{BROKER}/welcome")).await,
        None
    );
}
