//! Port of `packages/logger/__tests__/err-serializer.test.ts`. A JS `Error`
//! (with `Object.assign`ed fields) becomes a `LoggedError`; pino's log
//! arguments become `LogArg`s.

use chatbotx_core::logger::{LogArg, LoggedError, Thrown, redact_err_serializer, scrub_log_args};
use serde_json::{Map, json};

const TOKEN: &str = "EAABsecret1234567890";

fn refresh_url() -> String {
    format!(
        "https://graph.instagram.com/refresh_access_token?grant_type=ig_refresh_token&access_token={TOKEN}"
    )
}

/// Port of: packages/logger/__tests__/err-serializer.test.ts:8 "redactErrSerializer > scrubs a token embedded in an error message and stack"
#[test]
fn redacterrserializer_scrubs_a_token_embedded_in_an_error_message_and_stack() {
    let error = LoggedError::new(format!("Request to {} failed", refresh_url()));

    let serialized = redact_err_serializer(&Thrown::Error(error)).to_string();

    assert!(!serialized.contains(TOKEN));
    assert!(serialized.contains("access_token=[redacted]"));
}

/// Port of: packages/logger/__tests__/err-serializer.test.ts:17 "redactErrSerializer > scrubs a token on a nested originError captured by an HTTP client"
#[test]
fn redacterrserializer_scrubs_a_token_on_a_nested_originerror_captured_by_an_http_client() {
    let origin = LoggedError::new(format!("GET {}", refresh_url()))
        .with_field("request", json!({ "url": refresh_url() }));
    let error = LoggedError::new("auth refresh failed").with_error_field("originError", origin);

    let serialized = redact_err_serializer(&Thrown::Error(error)).to_string();

    assert!(!serialized.contains(TOKEN));
}

/// Port of: packages/logger/__tests__/err-serializer.test.ts:30 "redactErrSerializer > scrubs a Facebook fb_exchange_token"
#[test]
fn redacterrserializer_scrubs_a_facebook_fb_exchange_token() {
    let url = format!(
        "https://graph.facebook.com/oauth/access_token?grant_type=fb_exchange_token&fb_exchange_token={TOKEN}"
    );
    let serialized = redact_err_serializer(&Thrown::Error(LoggedError::new(format!(
        "refresh via {url}"
    ))))
    .to_string();

    assert!(!serialized.contains(TOKEN));
}

/// Port of: packages/logger/__tests__/err-serializer.test.ts:39 "redactErrSerializer > passes an already-sanitized plain object through, still scrubbing"
#[test]
fn redacterrserializer_passes_an_already_sanitized_plain_object_through_still_scrubbing() {
    let result = redact_err_serializer(&Thrown::Value(json!({
        "name": "Error",
        "message": format!("boom {}", refresh_url()),
    })));

    assert!(!result.to_string().contains(TOKEN));
}

/// Port of: packages/logger/__tests__/err-serializer.test.ts:50 "scrubLogArgs > scrubs a token in an explicit message string"
#[test]
fn scrublogargs_scrubs_a_token_in_an_explicit_message_string() {
    let result = scrub_log_args(&[
        LogArg::Object {
            err: Some(LoggedError::new("x")),
            fields: Map::new(),
        },
        LogArg::Message(format!("hit {}", refresh_url())),
    ]);

    assert!(matches!(
        &result[0],
        LogArg::Object { err: Some(_), fields } if fields.is_empty()
    ));
    let LogArg::Message(msg) = &result[1] else {
        panic!("expected a message, got {:?}", result[1]);
    };
    assert!(!msg.contains(TOKEN));
    assert!(msg.contains("access_token=[redacted]"));
}

/// Port of: packages/logger/__tests__/err-serializer.test.ts:61 "scrubLogArgs > appends a scrubbed message when a bare Error is logged with no message"
#[test]
fn scrublogargs_appends_a_scrubbed_message_when_a_bare_error_is_logged_with_no_message() {
    let error = LoggedError::new(format!("GET {}", refresh_url()));
    let result = scrub_log_args(&[LogArg::Error(error.clone())]);

    assert_eq!(result[0], LogArg::Error(error));
    let LogArg::Message(msg) = &result[1] else {
        panic!("expected a message string, got {:?}", result.get(1));
    };
    assert!(!msg.contains(TOKEN));
}

/// Port of: packages/logger/__tests__/err-serializer.test.ts:72 "scrubLogArgs > appends a scrubbed message for an { err } object with no message"
#[test]
fn scrublogargs_appends_a_scrubbed_message_for_an_err_object_with_no_message() {
    let result = scrub_log_args(&[LogArg::Object {
        err: Some(LoggedError::new(format!("GET {}", refresh_url()))),
        fields: Map::new(),
    }]);

    let LogArg::Message(msg) = &result[1] else {
        panic!("expected a message string, got {:?}", result.get(1));
    };
    assert!(!msg.contains(TOKEN));
}

/// Port of: packages/logger/__tests__/err-serializer.test.ts:78 "scrubLogArgs > leaves a clean call untouched"
#[test]
fn scrublogargs_leaves_a_clean_call_untouched() {
    let mut fields = Map::new();
    fields.insert("userId".to_owned(), json!("u-1"));
    let args = vec![
        LogArg::Object { err: None, fields },
        LogArg::Message("did a thing".to_owned()),
    ];

    assert_eq!(scrub_log_args(&args), args);
}
