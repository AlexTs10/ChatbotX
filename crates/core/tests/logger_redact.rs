//! Port of `packages/logger/__tests__/redact.test.ts`. JSON-like values are
//! `serde_json::Value`; lengths are UTF-16 units, as in JavaScript.

use chatbotx_core::logger::{
    DEFAULT_MAX_LOG_CHARS, LoggedError, Thrown, cap_text, redact_secrets, scrub_secrets_in_string,
    to_log_safe_error,
};
use serde_json::{Value, json};

const TOKEN: &str = "EAABsecret1234567890";

fn graph_url() -> String {
    format!("https://graph.facebook.com/v25.0/123?fields=name&access_token={TOKEN}")
}

/// Port of: packages/logger/__tests__/redact.test.ts:14 "redactSecrets > redacts top-level sensitive keys and keeps ordinary ones"
#[test]
fn redactsecrets_redacts_top_level_sensitive_keys_and_keeps_ordinary_ones() {
    let result = redact_secrets(&json!({
        "access_token": "abc",
        "apiKey": "xyz",
        "api_key": "xyz",
        "verify_token": "vt",
        "password": "hunter2",
        "signature": "sig",
        "userId": "keep-me",
    }));

    assert_eq!(result["access_token"], "[redacted]");
    assert_eq!(result["apiKey"], "[redacted]");
    assert_eq!(result["api_key"], "[redacted]");
    assert_eq!(result["verify_token"], "[redacted]");
    assert_eq!(result["password"], "[redacted]");
    assert_eq!(result["signature"], "[redacted]");
    assert_eq!(result["userId"], "keep-me");
}

/// Port of: packages/logger/__tests__/redact.test.ts:34 "redactSecrets > does not redact ordinary keys such as tokenCount (exact-match guard)"
#[test]
fn redactsecrets_does_not_redact_ordinary_keys_such_as_tokencount_exact_match_guard() {
    let result = redact_secrets(&json!({ "tokenCount": 42 }));

    assert_eq!(result["tokenCount"], 42);
}

/// Port of: packages/logger/__tests__/redact.test.ts:40 "redactSecrets > redacts sensitive keys nested in objects and arrays"
#[test]
fn redactsecrets_redacts_sensitive_keys_nested_in_objects_and_arrays() {
    let result = redact_secrets(&json!({
        "user": { "name": "alice", "token": "secret-token" },
        "entries": [{ "client_secret": "s1", "value": 1 }],
    }));

    assert_eq!(result["user"]["token"], "[redacted]");
    assert_eq!(result["user"]["name"], "alice");
    assert_eq!(result["entries"][0]["client_secret"], "[redacted]");
    assert_eq!(result["entries"][0]["value"], 1);
}

/// Port of: packages/logger/__tests__/redact.test.ts:55 "redactSecrets > redacts the SDP value nested in a WhatsApp calling session under calls[] (the `sdp` key, not the whole session)"
#[test]
fn redactsecrets_redacts_the_sdp_value_nested_in_a_whatsapp_calling_session_under_calls_the_sdp_key_not_the_whole_session()
 {
    let sentinel = "v=0 SENTINEL-SDP-OFFER-DO-NOT-LOG";
    let result = redact_secrets(&json!({
        "entry": [{
            "changes": [{
                "field": "calls",
                "value": {
                    "calls": [{
                        "id": "wacid.ABC",
                        "event": "connect",
                        "session": { "sdp_type": "offer", "sdp": sentinel },
                    }],
                },
            }],
        }],
    }));

    let call = &result["entry"][0]["changes"][0]["value"]["calls"][0];
    assert_eq!(call["session"]["sdp"], "[redacted]");
    assert_eq!(call["session"]["sdp_type"], "offer");
    assert_eq!(call["id"], "wacid.ABC");
    assert!(!result.to_string().contains(sentinel));
}

/// Port of: packages/logger/__tests__/redact.test.ts:99 "redactSecrets > passes primitives through untouched"
#[test]
fn redactsecrets_passes_primitives_through_untouched() {
    assert_eq!(redact_secrets(&json!(42)), json!(42));
    assert_eq!(redact_secrets(&json!("hello")), json!("hello"));
    assert_eq!(redact_secrets(&Value::Null), Value::Null);
    assert_eq!(redact_secrets(&json!(true)), json!(true));
}

/// Port of: packages/logger/__tests__/redact.test.ts:106 "redactSecrets > does not mutate the input"
#[test]
fn redactsecrets_does_not_mutate_the_input() {
    let input = json!({ "token": "secret", "nested": { "password": "p" } });
    redact_secrets(&input);

    assert_eq!(input["token"], "secret");
    assert_eq!(input["nested"]["password"], "p");
}

/// Port of: packages/logger/__tests__/redact.test.ts:114 "redactSecrets > drops subtrees deeper than the depth limit without throwing"
#[test]
fn redactsecrets_drops_subtrees_deeper_than_the_depth_limit_without_throwing() {
    let mut nested = json!({ "token": "deep-secret" });
    for _ in 0..200 {
        nested = json!({ "child": nested });
    }

    let result = std::panic::catch_unwind(|| redact_secrets(&nested));
    assert!(result.is_ok());

    let serialized = result.unwrap().to_string();
    assert!(!serialized.contains("deep-secret"));
    assert!(serialized.contains("[omitted: too deeply nested]"));
}

/// Port of: packages/logger/__tests__/redact.test.ts:132 "capText > leaves text at or under the limit untouched"
#[test]
fn captext_leaves_text_at_or_under_the_limit_untouched() {
    assert_eq!(cap_text("hello", 10), "hello");
    assert_eq!(cap_text(&"a".repeat(10), 10), "a".repeat(10));
}

/// Port of: packages/logger/__tests__/redact.test.ts:137 "capText > truncates and appends a marker when over the limit"
#[test]
fn captext_truncates_and_appends_a_marker_when_over_the_limit() {
    let result = cap_text(&"a".repeat(20), 10);

    assert!(result.starts_with(&"a".repeat(10)));
    assert!(result.contains("…[truncated 10 chars]"));
}

/// Port of: packages/logger/__tests__/redact.test.ts:144 "capText > defaults to DEFAULT_MAX_LOG_CHARS"
#[test]
fn captext_defaults_to_default_max_log_chars() {
    let result = cap_text(
        &"a".repeat(DEFAULT_MAX_LOG_CHARS + 100),
        DEFAULT_MAX_LOG_CHARS,
    );

    assert!(result.contains("…[truncated 100 chars]"));
}

/// Port of: packages/logger/__tests__/redact.test.ts:150 "capText > does not split a trailing surrogate pair"
#[test]
fn captext_does_not_split_a_trailing_surrogate_pair() {
    // "😀" is a surrogate pair; its high half sits on the cap boundary.
    let text = format!("{}😀{}", "a".repeat(9), "a".repeat(5));
    let result = cap_text(&text, 10);
    let retained = &result[..result.find("…[truncated").unwrap()];
    let last_unit = retained.encode_utf16().last().unwrap();

    assert!(!(0xd800..=0xdbff).contains(&last_unit));
    // Rust strings cannot hold a lone surrogate: the retained part is whole
    // characters, so the pair is either kept or dropped entirely.
    assert_eq!(retained, "a".repeat(9));
}

/// Port of: packages/logger/__tests__/redact.test.ts:162 "scrubSecretsInString > redacts a token embedded in a URL query string"
#[test]
fn scrubsecretsinstring_redacts_a_token_embedded_in_a_url_query_string() {
    let result = scrub_secrets_in_string(&graph_url());

    assert!(!result.contains(TOKEN));
    assert!(result.contains("access_token=[redacted]"));
    assert!(result.contains("fields=name"));
}

/// Port of: packages/logger/__tests__/redact.test.ts:170 "scrubSecretsInString > redacts a bare Gemini key query parameter"
#[test]
fn scrubsecretsinstring_redacts_a_bare_gemini_key_query_parameter() {
    let key = "AIzaSensitiveValue";
    let result = scrub_secrets_in_string(&format!(
        "https://generativelanguage.googleapis.com/v1beta/models?key={key}"
    ));

    assert!(!result.contains(key));
    assert!(result.contains("key=[redacted]"));
}

/// Port of: packages/logger/__tests__/redact.test.ts:180 "scrubSecretsInString > leaves a string without secrets untouched"
#[test]
fn scrubsecretsinstring_leaves_a_string_without_secrets_untouched() {
    assert_eq!(scrub_secrets_in_string("just a message"), "just a message");
}

/// Port of: packages/logger/__tests__/redact.test.ts:186 "redactSecrets URL-value scrubbing > scrubs a token inside a nested string value, not just by key"
#[test]
fn redactsecrets_url_value_scrubbing_scrubs_a_token_inside_a_nested_string_value_not_just_by_key() {
    let result = redact_secrets(&json!({ "requestUrl": graph_url() }));

    assert!(!result.to_string().contains(TOKEN));
    assert!(
        result["requestUrl"]
            .as_str()
            .unwrap()
            .contains("access_token=[redacted]")
    );
}

/// Port of: packages/logger/__tests__/redact.test.ts:198 "toLogSafeError > scrubs a token from an error message and stack, dropping nested fields"
#[test]
fn tologsafeerror_scrubs_a_token_from_an_error_message_and_stack_dropping_nested_fields() {
    let error = LoggedError::new(format!("Request to {} failed", graph_url()))
        .with_field("request", json!({ "url": graph_url() }))
        .with_field("response", json!({ "status": 400 }));

    let result = to_log_safe_error(&Thrown::Error(error));
    let serialized = serde_json::to_value(&result).unwrap();

    assert!(!serialized.to_string().contains(TOKEN));
    assert_eq!(result.name, "Error");
    assert!(result.message.contains("access_token=[redacted]"));
    assert!(serialized.get("request").is_none());
    assert!(serialized.get("response").is_none());
}

/// Port of: packages/logger/__tests__/redact.test.ts:217 "toLogSafeError > handles a non-Error thrown value"
#[test]
fn tologsafeerror_handles_a_non_error_thrown_value() {
    let result = to_log_safe_error(&Thrown::Value(json!("boom")));
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        json!({ "name": "NonError", "message": "boom" })
    );
}
