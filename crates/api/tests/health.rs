//! `GET /api/health` must match the Next.js route in
//! `apps/builder/src/app/api/health/route.ts`.

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;

/// `YYYY-MM-DDTHH:MM:SS.mmmZ`, the format of JS `Date.prototype.toISOString`.
fn is_js_iso_string(s: &str) -> bool {
    let pattern = b"dddd-dd-ddTdd:dd:dd.dddZ";
    s.len() == pattern.len()
        && s.bytes().zip(pattern).all(|(c, &p)| match p {
            b'd' => c.is_ascii_digit(),
            _ => c == p,
        })
}

#[tokio::test]
async fn health_returns_the_original_json_shape() {
    let response = chatbotx_api::router()
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router is infallible");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.as_bytes()),
        Some(&b"application/json"[..])
    );

    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    let raw = std::str::from_utf8(&bytes).expect("utf-8 body");
    // Same key order as JSON.stringify of the original object literal.
    assert!(
        raw.starts_with(r#"{"status":"healthy","timestamp":""#)
            && raw.ends_with(r#"","service":"builder"}"#),
        "unexpected body: {raw}"
    );

    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    let object = json.as_object().expect("json object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["service", "status", "timestamp"]);
    assert_eq!(object["status"], "healthy");
    assert_eq!(object["service"], "builder");
    let timestamp = object["timestamp"].as_str().expect("timestamp string");
    assert!(is_js_iso_string(timestamp), "bad timestamp: {timestamp}");
}

#[test]
fn iso_pattern_helper_rejects_bad_input() {
    assert!(is_js_iso_string("2024-02-29T12:34:56.789Z"));
    assert!(!is_js_iso_string("2024-02-29T12:34:56Z"));
    assert!(!is_js_iso_string("2024-02-29 12:34:56.789Z"));
    assert!(!is_js_iso_string(""));
}
