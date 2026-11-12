//! Port of `packages/utils/__tests__/crypto.test.ts`. `node:crypto`
//! `createHmac` in the test becomes `ring::hmac`.

use chatbotx_core::utils::crypto::{
    VerifyHmacSha256SignatureInput, hmac_sha256_hex, timing_safe_string_equal,
    verify_hmac_sha256_signature,
};
use regex::Regex;
use ring::hmac;

const SECRET: &str = "app-secret";

fn sign(raw_body: &[u8], secret: &str) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let digest: String = hmac::sign(&key, raw_body)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("sha256={digest}")
}

fn verify(raw_body: &[u8], header: Option<&str>, prefix: Option<&str>) -> bool {
    verify_hmac_sha256_signature(VerifyHmacSha256SignatureInput {
        raw_body,
        secret: SECRET,
        signature_header: header,
        prefix,
    })
}

/// Port of: packages/utils/__tests__/crypto.test.ts:12 "hmacSha256Hex > produces a stable lowercase hex digest for a given secret + payload"
#[test]
fn hmacsha256hex_produces_a_stable_lowercase_hex_digest_for_a_given_secret_payload() {
    let first = hmac_sha256_hex("secret", "payload");
    let second = hmac_sha256_hex("secret", "payload");

    assert_eq!(first, second);
    assert!(Regex::new("^[0-9a-f]{64}$").unwrap().is_match(&first));
}

/// Port of: packages/utils/__tests__/crypto.test.ts:20 "hmacSha256Hex > changes when the secret changes"
#[test]
fn hmacsha256hex_changes_when_the_secret_changes() {
    assert_ne!(
        hmac_sha256_hex("secret-a", "payload"),
        hmac_sha256_hex("secret-b", "payload")
    );
}

/// Port of: packages/utils/__tests__/crypto.test.ts:27 "hmacSha256Hex > changes when the payload changes"
#[test]
fn hmacsha256hex_changes_when_the_payload_changes() {
    assert_ne!(
        hmac_sha256_hex("secret", "payload-a"),
        hmac_sha256_hex("secret", "payload-b")
    );
}

/// Port of: packages/utils/__tests__/crypto.test.ts:36 "timingSafeStringEqual > returns true for identical strings"
#[test]
fn timingsafestringequal_returns_true_for_identical_strings() {
    assert!(timing_safe_string_equal("abc123", "abc123"));
}

/// Port of: packages/utils/__tests__/crypto.test.ts:40 "timingSafeStringEqual > returns false for different strings of equal length"
#[test]
fn timingsafestringequal_returns_false_for_different_strings_of_equal_length() {
    assert!(!timing_safe_string_equal("abc123", "abc124"));
}

/// Port of: packages/utils/__tests__/crypto.test.ts:44 "timingSafeStringEqual > returns false for strings of different length"
#[test]
fn timingsafestringequal_returns_false_for_strings_of_different_length() {
    assert!(!timing_safe_string_equal("abc", "abcd"));
}

/// Port of: packages/utils/__tests__/crypto.test.ts:55 "verifyHmacSha256Signature > accepts a signature computed over the exact raw bytes"
#[test]
fn verifyhmacsha256signature_accepts_a_signature_computed_over_the_exact_raw_bytes() {
    let raw_body = br#"{"hello":"world"}"#;
    assert!(verify(raw_body, Some(&sign(raw_body, SECRET)), None));
}

/// Port of: packages/utils/__tests__/crypto.test.ts:67 "verifyHmacSha256Signature > accepts a valid signature over non-ASCII (Vietnamese) bytes"
#[test]
fn verifyhmacsha256signature_accepts_a_valid_signature_over_non_ascii_vietnamese_bytes() {
    let raw_body = r#"{"text":"Xin chào, tôi cần hỗ trợ đặt hàng."}"#.as_bytes();
    assert!(verify(raw_body, Some(&sign(raw_body, SECRET)), None));
}

/// Port of: packages/utils/__tests__/crypto.test.ts:81 "verifyHmacSha256Signature > rejects when the body does not match the signature"
#[test]
fn verifyhmacsha256signature_rejects_when_the_body_does_not_match_the_signature() {
    let header = sign(br#"{"hello":"world"}"#, SECRET);
    assert!(!verify(br#"{"hello":"tampered"}"#, Some(&header), None));
}

/// Port of: packages/utils/__tests__/crypto.test.ts:93 "verifyHmacSha256Signature > rejects a missing signature header"
#[test]
fn verifyhmacsha256signature_rejects_a_missing_signature_header() {
    assert!(!verify(b"{}", None, None));
}

/// Port of: packages/utils/__tests__/crypto.test.ts:103 "verifyHmacSha256Signature > rejects a signature header with the wrong prefix"
#[test]
fn verifyhmacsha256signature_rejects_a_signature_header_with_the_wrong_prefix() {
    let digest = sign(b"{}", SECRET).replace("sha256=", "");
    assert!(!verify(b"{}", Some(&format!("sha1={digest}")), None));
}

/// Port of: packages/utils/__tests__/crypto.test.ts:115 "verifyHmacSha256Signature > rejects a malformed (non-hex) signature hash"
#[test]
fn verifyhmacsha256signature_rejects_a_malformed_non_hex_signature_hash() {
    assert!(!verify(b"{}", Some("sha256=zz"), None));
}

/// Port of: packages/utils/__tests__/crypto.test.ts:125 "verifyHmacSha256Signature > rejects a truncated (wrong-length) signature hash"
#[test]
fn verifyhmacsha256signature_rejects_a_truncated_wrong_length_signature_hash() {
    let digest = sign(b"{}", SECRET).replace("sha256=", "");
    assert!(!verify(
        b"{}",
        Some(&format!("sha256={}", &digest[..10])),
        None
    ));
}

/// Port of: packages/utils/__tests__/crypto.test.ts:137 "verifyHmacSha256Signature > supports a custom prefix"
#[test]
fn verifyhmacsha256signature_supports_a_custom_prefix() {
    let digest = sign(b"{}", SECRET).replace("sha256=", "");
    assert!(verify(
        b"{}",
        Some(&format!("custom={digest}")),
        Some("custom=")
    ));
}
