//! Port of `integrations/google-ads/__tests__/sanitize.test.ts` (the message
//! part of `sanitizeGoogleAdsError`; the HTTP code and reason are fields of
//! `GoogleError` in Rust).

use chatbotx_business::google_ads_api::sanitize;

const CLICK_ID: &str = "Cj0KCQiA1234567890abcdef";

/// Port of: "redacts a click id Google echoes back inside an error detail"
#[test]
fn redacts_a_click_id_google_echoes_back() {
    let message = sanitize(
        &format!("Invalid gclid {CLICK_ID} unparseable {CLICK_ID}"),
        &[CLICK_ID],
    );
    assert!(!message.contains(CLICK_ID));
}

/// Port of: "redacts bearer and Google access tokens"
#[test]
fn redacts_bearer_and_google_access_tokens() {
    let message = sanitize("Authorization: Bearer ya29.A0AbCdEf-gh_ij failed", &[]);
    assert!(!message.contains("ya29"));
    assert!(!message.contains("A0AbCdEf"));
}

/// Port of: "redacts key=value credentials and private keys"
#[test]
fn redacts_key_value_credentials_and_private_keys() {
    let message = sanitize(
        "gclid=ABCDEF123456 {\"developer-token\":\"dev-secret\"} -----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----",
        &[],
    );
    assert!(!message.contains("ABCDEF123456"));
    assert!(!message.contains("dev-secret"));
    assert!(!message.contains("BEGIN PRIVATE KEY"));
}

/// Port of: "never throws on a non-error input and truncates long messages"
#[test]
fn truncates_long_messages() {
    assert_eq!(sanitize(&"x".repeat(5000), &[]).chars().count(), 1000);
}

/// Port of: "customer-matching data > redacts a SHA-256 digest, an e-mail address and an E.164 phone"
#[test]
fn redacts_a_digest_an_email_and_a_phone() {
    let digest = format!("{}{}", "a".repeat(32), "0123456789abcdef".repeat(2));
    let message = sanitize(
        &format!("bad identifier {digest} for Customer.Name+tag@Example.co.uk or +84901234567"),
        &[],
    );
    for leaked in [
        digest.as_str(),
        "Customer.Name",
        "Example.co.uk",
        "+84901234567",
    ] {
        assert!(!message.contains(leaked), "{leaked}");
    }
}

/// Port of: "customer-matching data > keeps ids that are not matching data readable"
#[test]
fn keeps_ids_that_are_not_matching_data_readable() {
    let transaction_id = format!("gads-v2-123-e-{}", "b".repeat(32));
    let message = sanitize(
        &format!("{transaction_id} at 2026-10-08T14:30:00+0700 field userData.userIdentifiers[0]"),
        &[],
    );
    assert!(message.contains(&transaction_id));
    assert!(message.contains("+0700"));
    assert!(message.contains("userData.userIdentifiers[0]"));
}

/// Port of: "scans a very long run without an @ in linear time"
#[test]
fn scans_a_long_run_in_linear_time() {
    let started = std::time::Instant::now();
    let message = sanitize(&"a".repeat(200_000), &[]);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert_eq!(message.len(), 1000);
}

/// Port of: "still redacts an address after a long run of local-part characters"
#[test]
fn redacts_an_address_after_a_long_run() {
    let message = sanitize(&format!("{} x.y+z@mail.example.com", "a".repeat(50)), &[]);
    assert!(!message.contains("mail.example.com"));
}

/// Port of: "does not redact a 64-character run inside a longer token"
#[test]
fn keeps_a_64_character_run_inside_a_longer_token() {
    let longer = "c".repeat(70);
    assert_eq!(sanitize(&longer, &[]), longer);
}
