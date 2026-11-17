//! Ports of `integrations/threads/__tests__/{webhook,errors-and-integration,
//! comment}.test.ts` (the parts with no HTTP mock).

use std::time::Duration;

use chatbotx_core::utils::crypto::hmac_sha256_hex;
use serde_json::{Value, json};

use super::*;

const CLIENT_ID: &str = "threads-app-id";
const SECRET: &str = "threads-secret";
const VERIFY: &str = "threads-verify-token";

fn value(overrides: Value) -> Value {
    let mut v = json!({
        "id": "comment-1", "username": "Commenter", "text": "Hello there",
        "replied_to": { "id": "parent-1" },
        "root_post": { "id": "post-1", "owner_id": "owner-1", "username": "RootPoster" },
        "timestamp": "2026-07-16T01:02:03Z",
    });
    for (k, val) in overrides.as_object().cloned().unwrap_or_default() {
        if val.is_null() {
            v.as_object_mut().unwrap().remove(&k);
        } else {
            v[k] = val;
        }
    }
    v
}

fn body(overrides: Value, value_overrides: Value) -> String {
    let mut b = json!({
        "app_id": CLIENT_ID, "topic": "moderate", "target_id": "target-1",
        "time": 1_783_674_105, "subscription_id": "subscription-1",
        "values": [{ "field": "replies", "value": value(value_overrides) }],
    });
    for (k, v) in overrides.as_object().cloned().unwrap_or_default() {
        b[k] = v;
    }
    b.to_string()
}

fn post(body: &str, signature: Option<&str>) -> Result<Outcome, String> {
    handle_request(
        false,
        None,
        signature,
        body.as_bytes(),
        (Some(VERIFY), Some(CLIENT_ID), SECRET),
    )
}

fn signed(body: &str) -> Result<Vec<WebhookJob>, String> {
    let signature = format!("sha256={}", hmac_sha256_hex(SECRET, body));
    match post(body, Some(&signature))? {
        Outcome::Jobs(jobs) => Ok(jobs),
        Outcome::Challenge(c) => panic!("unexpected challenge {c}"),
    }
}

fn comment_job(comment: Value) -> Value {
    json!({ "type": "incomingComment", "data": {
        "integrationType": "threads", "integrationIdentifier": "owner-1", "commentData": comment } })
}

#[test]
fn handshake_returns_the_challenge_or_fails() {
    let get = |q: &str| {
        handle_request(
            true,
            Some(q),
            None,
            b"",
            (Some(VERIFY), Some(CLIENT_ID), SECRET),
        )
    };
    assert!(matches!(
        get("hub.mode=subscribe&hub.verify_token=threads-verify-token&hub.challenge=test-challenge"),
        Ok(Outcome::Challenge(c)) if c == "test-challenge"
    ));
    assert_eq!(
        get("hub.mode=subscribe&hub.verify_token=wrong-token&hub.challenge=test-challenge")
            .unwrap_err(),
        "Invalid webhook verification parameters"
    );
}

#[test]
fn enqueues_a_valid_reply_comment_event() {
    let jobs = signed(&body(json!({}), json!({}))).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].name, "incomingComment");
    assert_eq!(
        jobs[0].data,
        comment_job(json!({
            "commentId": "comment-1", "postId": "post-1", "parentId": "parent-1",
            "fromId": "commenter", "fromName": "Commenter", "message": "Hello there",
            "createdTime": 1_784_163_723,
        }))
    );
}

#[test]
fn enqueues_every_reply_and_skips_other_fields_and_self_replies() {
    let b = json!({
        "app_id": CLIENT_ID, "topic": "moderate", "target_id": "target-1",
        "time": 1_783_674_105, "subscription_id": "subscription-1",
        "values": [
            { "field": "replies", "value": value(json!({})) },
            { "field": "replies", "value": value(json!({ "id": "comment-2", "username": "Another" })) },
            { "field": "mentions", "value": value(json!({ "id": "comment-3" })) },
            { "field": "replies", "value": value(json!({ "id": "comment-4", "username": "rootposter" })) },
        ],
    })
    .to_string();
    let jobs = signed(&b).unwrap();
    let ids: Vec<&Value> = jobs
        .iter()
        .map(|j| &j.data["data"]["commentData"]["commentId"])
        .collect();
    assert_eq!(ids, [&json!("comment-1"), &json!("comment-2")]);
    assert_eq!(jobs[1].data["data"]["commentData"]["fromId"], "another");
    assert_eq!(jobs[1].data["data"]["commentData"]["fromName"], "Another");
}

#[test]
fn accepts_values_as_a_single_object() {
    let b = body(
        json!({ "values": { "field": "replies", "value": value(json!({})) } }),
        json!({}),
    );
    assert_eq!(signed(&b).unwrap().len(), 1);
}

#[test]
fn signature_failures() {
    let b = body(json!({}), json!({}));
    assert_eq!(
        post(&b, Some("sha256=invalid")).unwrap_err(),
        "Invalid webhook signature"
    );
    assert_eq!(
        post(&b, Some("invalid-format")).unwrap_err(),
        "Invalid webhook signature"
    );
    assert_eq!(post(&b, None).unwrap_err(), "Missing webhook signature");
    assert_eq!(
        post("", Some("sha256=x")).unwrap_err(),
        "Empty webhook payload"
    );
    // `lib/webhook.ts` compares decoded bytes: an upper-case hex signature matches.
    let upper = format!("sha256={}", hmac_sha256_hex(SECRET, &b).to_uppercase());
    assert!(post(&b, Some(&upper)).is_ok());
}

#[test]
fn skips_irrelevant_topics_and_falls_back_to_payload_time() {
    assert!(
        signed(&body(json!({ "topic": "insights" }), json!({})))
            .unwrap()
            .is_empty()
    );
    for overrides in [
        json!({ "timestamp": null }),
        json!({ "timestamp": "not-a-date" }),
    ] {
        let jobs = signed(&body(json!({}), overrides)).unwrap();
        assert_eq!(
            jobs[0].data["data"]["commentData"]["createdTime"],
            1_783_674_105
        );
    }
}

#[test]
fn schema_mismatched_or_invalid_json_payloads_are_skipped() {
    let empty_id = body(json!({}), json!({ "id": "" }));
    assert!(signed(&empty_id).unwrap().is_empty());
    let null_text = body(json!({}), json!({ "text": Value::Null }));
    // `null` overrides remove the key in this helper; a JSON null text fails the schema.
    let mut parsed: Value = serde_json::from_str(&null_text).unwrap();
    parsed["values"][0]["value"]["text"] = Value::Null;
    assert!(signed(&parsed.to_string()).unwrap().is_empty());
    assert!(signed("not json").unwrap().is_empty());
}

#[test]
fn app_id_mismatch_fails() {
    assert_eq!(
        signed(&body(json!({ "app_id": "other-app-id" }), json!({}))).unwrap_err(),
        "Webhook app_id does not match configured clientId"
    );
}

#[test]
fn parses_the_anonymized_live_meta_delivery() {
    let raw = r#"{"app_id":"1234567890123456","topic":"moderate","target_id":"17841400000000001","time":1788249976,"subscription_id":"1000000000000001","has_uid_field":false,"values":[{"value":{"id":"17800000000000002","username":"test_commenter","text":"Great post!","media_type":"TEXT_POST","permalink":"https://www.threads.com/@test_commenter/post/ABCdef12345","replied_to":{"id":"17841400000000001"},"root_post":{"id":"17841400000000001","owner_id":"20000000000000003","username":"test_account"},"shortcode":"ABCdef12345","timestamp":"2026-09-01T08:06:09+0000","is_verified":false,"profile_picture_url":"https://scontent.cdninstagram.com/v/t51.0-0/000000000_0000000000000000_0000000000000000000_n.jpg"},"field":"replies"}]}"#;
    let signature = format!("sha256={}", hmac_sha256_hex(SECRET, raw));
    let Ok(Outcome::Jobs(jobs)) = handle_request(
        false,
        None,
        Some(&signature),
        raw.as_bytes(),
        (Some(VERIFY), Some("1234567890123456"), SECRET),
    ) else {
        panic!("expected jobs");
    };
    assert_eq!(
        jobs[0].data,
        json!({ "type": "incomingComment", "data": {
            "integrationType": "threads", "integrationIdentifier": "20000000000000003",
            "commentData": {
                "commentId": "17800000000000002", "postId": "17841400000000001",
                "parentId": "17841400000000001", "fromId": "test_commenter",
                "fromName": "test_commenter",
                "fromAvatarUrl": "https://scontent.cdninstagram.com/v/t51.0-0/000000000_0000000000000000_0000000000000000000_n.jpg",
                "message": "Great post!", "createdTime": 1_788_249_969,
            } } })
    );
}

#[test]
fn multi_byte_body_is_signed_over_its_utf8_bytes() {
    let text = "訂單成立了嗎？已付款 ✅ 謝謝你 🎉 café";
    let b = body(json!({}), json!({ "text": text }));
    assert!(b.contains(text));
    assert_eq!(
        signed(&b).unwrap()[0].data["data"]["commentData"]["message"],
        text
    );
    let escaped: String = b
        .encode_utf16()
        .map(|u| {
            if u > 127 {
                format!("\\u{u:04x}")
            } else {
                char::from(u as u8).to_string()
            }
        })
        .collect();
    assert_ne!(escaped, b);
    let signature = format!("sha256={}", hmac_sha256_hex(SECRET, &escaped));
    assert_eq!(
        post(&b, Some(&signature)).unwrap_err(),
        "Invalid webhook signature"
    );
}

#[test]
fn error_response_status_and_public_message() {
    assert_eq!(
        error_response("Invalid webhook signature"),
        (400, "Invalid Threads webhook request")
    );
    assert_eq!(
        error_response("Unsupported HTTP method: PUT"),
        (405, "Invalid Threads webhook request")
    );
    assert_eq!(
        error_response("boom"),
        (500, "Failed to process Threads webhook")
    );
}

fn err(
    code: Option<Value>,
    status: Option<u16>,
    kind: Option<&str>,
    message: &str,
) -> ThreadsError {
    ThreadsError {
        message: message.into(),
        code,
        kind: kind.map(str::to_owned),
        http_status: status,
        ..ThreadsError::default()
    }
}

#[test]
fn maps_codes_statuses_and_messages_to_categories() {
    let cases = [
        (Some(json!(190)), Category::AuthFailed, 401, "190"),
        (Some(json!(250)), Category::PermissionDenied, 403, "250"),
        (Some(json!(4)), Category::RateLimited, 429, "4"),
        (
            Some(json!(2_207_042)),
            Category::QuotaExceeded,
            429,
            "2207042",
        ),
        (Some(json!(2)), Category::NetworkError, 503, "2"),
        (Some(json!(100)), Category::PayloadInvalid, 400, "100"),
        (None, Category::Unknown, 400, "channel_error"),
    ];
    for (code, category, status, code_text) in cases {
        let mapped = map_error(&err(code, None, None, "x"));
        assert_eq!(
            (
                mapped.category,
                mapped.http_status_code,
                mapped.code.as_deref()
            ),
            (category, status, Some(code_text))
        );
        assert_eq!(mapped.message, "x");
    }
    let category = |e: ThreadsError| categorize(&e);
    assert_eq!(
        category(err(None, None, Some("OAuthException"), "oauth")),
        Category::AuthFailed
    );
    assert_eq!(
        category(err(None, Some(401), None, "unauthorized")),
        Category::AuthFailed
    );
    assert_eq!(
        category(err(None, Some(429), None, "slow down")),
        Category::RateLimited
    );
    assert_eq!(
        category(err(None, Some(503), None, "server exploded")),
        Category::NetworkError
    );
    assert_eq!(
        category(err(
            None,
            None,
            None,
            "request did not finish within 30 seconds"
        )),
        Category::NetworkError
    );
    assert_eq!(
        category(err(None, None, None, "network connection reset")),
        Category::NetworkError
    );
    let http = ThreadsError {
        http_origin: true,
        ..ThreadsError::new("provider failed")
    };
    assert_eq!(categorize(&http), Category::NetworkError);
}

#[test]
fn detects_revoked_token_subcodes_precisely() {
    let revoked = |sub: Option<Value>| {
        is_revoked_token_error(&ThreadsError {
            code: Some(json!(190)),
            sub_code: sub,
            ..ThreadsError::new("revoked")
        })
    };
    assert!(revoked(Some(json!("458"))));
    assert!(!revoked(None));
    assert!(!revoked(Some(json!(999))));
}

#[test]
fn rescue_wraps_http_errors_with_sanitized_details() {
    let error = rescued(
        "me",
        Some(403),
        &json!({ "error": { "message": "denied https://graph.threads.com/v1.0/me?access_token=secret",
            "code": 10, "error_subcode": 1234, "type": "OAuthException" } }),
        "fallback",
    );
    assert_eq!(
        (
            error.code,
            error.sub_code,
            error.kind.as_deref(),
            error.http_status
        ),
        (
            Some(json!(10)),
            Some(json!(1234)),
            Some("OAuthException"),
            Some(403)
        )
    );
    assert!(error.message.contains("[REDACTED]"));
    assert!(!error.message.contains("secret"));
    let plain = rescued(
        "me",
        Some(502),
        &Value::Null,
        "Request failed with status code 502",
    );
    assert_eq!(plain.http_status, Some(502));
    assert!(plain.message.contains("Threads API error at me:"));
}

#[test]
fn sanitizer_redacts_urls_params_and_assignments() {
    let sanitized = sanitize_sensitive_text(
        r#"broken https://%zz?access_token=secret access_token=abc authorization: bearer-token client_secret="ultra-secret""#,
    );
    assert!(sanitized.contains("https://%zz?access_token=[REDACTED]"));
    assert!(sanitized.contains("access_token=[REDACTED]"));
    assert!(sanitized.contains("authorization: [REDACTED]"));
    assert!(sanitized.contains(r#"client_secret="[REDACTED]""#));
    // Changed: TS `not.toContain("secret")` cannot hold (the key `client_secret` stays); checks the values.
    for leaked in ["access_token=abc", "bearer-token", "ultra-secret"] {
        assert!(!sanitized.contains(leaked), "{sanitized}");
    }
    assert_eq!(
        sanitize_sensitive_text("https://graph.threads.com/v1.0/me?refresh_token=secret"),
        "https://graph.threads.com/v1.0/me?refresh_token=[REDACTED]"
    );
    assert_eq!(
        sanitize_sensitive_text("client_secret=top-secret"),
        "client_secret=[REDACTED]"
    );
    let unknown = sanitize_sensitive_text(
        "failed at https://graph.threads.com/v1.0/me?access_token=secret&client_secret=super-secret",
    );
    assert!(
        unknown.contains("[REDACTED]")
            && !unknown.contains("secret&")
            && !unknown.contains("super-secret")
    );
}

fn statuses(list: Vec<Value>) -> impl FnMut() -> std::future::Ready<Result<Value, ThreadsError>> {
    let mut list = list.into_iter();
    move || {
        std::future::ready(Ok(list
            .next()
            .unwrap_or(json!({ "status": "IN_PROGRESS" }))))
    }
}

#[tokio::test]
async fn reply_container_polling() {
    let (t, i) = (Duration::from_millis(50), Duration::from_millis(1));
    assert!(
        wait_for_ready(
            statuses(vec![
                json!({ "status": "IN_PROGRESS" }),
                json!({ "status": "FINISHED" })
            ]),
            t,
            i
        )
        .await
        .is_ok()
    );
    assert!(
        wait_for_ready(statuses(vec![json!({ "status": "PUBLISHED" })]), t, i)
            .await
            .is_ok()
    );
    assert!(
        wait_for_ready(
            statuses(vec![json!({}), json!({ "status": "finished" })]),
            t,
            i
        )
        .await
        .is_ok()
    );
    assert_eq!(
        wait_for_ready(
            statuses(vec![
                json!({ "status": "ERROR", "error_message": "bad media" })
            ]),
            t,
            i
        )
        .await
        .unwrap_err()
        .message,
        "Threads reply creation error: bad media"
    );
    assert_eq!(
        wait_for_ready(statuses(vec![json!({ "status": "EXPIRED" })]), t, i)
            .await
            .unwrap_err()
            .message,
        "Threads reply creation expired: Unknown error"
    );
    let timeout = wait_for_ready(statuses(vec![]), Duration::from_millis(5), i)
        .await
        .unwrap_err();
    assert_eq!(
        timeout.message,
        "Threads reply creation did not finish within 5ms. Retry later."
    );
    assert_eq!(categorize(&timeout), Category::NetworkError);
}
