//! Port of `integrations/facebook-ads/__tests__/audience-users.test.ts`
//! (Q.P6-U4): the phone normalisation and the audience payloads of the
//! `facebookCustomAudience` step. The bulk-sync and auth-URL cases belong to
//! the Facebook Ads connection, not to this step.

use chatbotx_business::ads::retarget::{
    AudienceContact, hashed_payload, normalize_contact_phone, page_uid_payload,
};
use serde_json::json;
use sha2::{Digest, Sha256};

fn sha256(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn phone(raw: &str, country: Option<&str>, fallback: Option<&str>) -> Option<String> {
    normalize_contact_phone(raw, country, fallback)
}

#[test]
fn uses_the_contact_country_to_produce_an_e164_number_without_plus() {
    assert_eq!(
        phone("0912 345 678", Some("VN"), None).as_deref(),
        Some("84912345678")
    );
    assert_eq!(
        phone("(212) 555-0175", Some("US"), None).as_deref(),
        Some("12125550175")
    );
}

#[test]
fn keeps_already_international_numbers() {
    assert_eq!(
        phone("+84912345678", None, None).as_deref(),
        Some("84912345678")
    );
}

#[test]
fn uses_the_workspace_fallback_country_when_the_contact_has_none() {
    assert_eq!(
        phone("0912345678", None, Some("VN")).as_deref(),
        Some("84912345678")
    );
    assert_eq!(
        phone("0912345678", Some("Vietnam"), Some("VN")).as_deref(),
        Some("84912345678")
    );
}

#[test]
fn returns_none_for_a_local_format_number_with_no_usable_region() {
    assert_eq!(phone("09-99", None, None), None);
    assert_eq!(phone("0912345678", Some("Vietnam"), None), None);
}

#[test]
fn keeps_digits_that_already_carry_a_calling_code() {
    assert_eq!(
        phone("84 912 345 678", None, None).as_deref(),
        Some("84912345678")
    );
}

#[test]
fn returns_none_when_no_digits_remain() {
    assert_eq!(phone("n/a", None, None), None);
}

#[test]
fn page_uid_payload_mirrors_the_legacy_schema_with_the_raw_key_flag() {
    assert_eq!(
        page_uid_payload("psid-1", "page-1"),
        json!({ "schema": ["PAGEUID"], "is_raw": true, "page_ids": ["page-1"], "data": [["psid-1"]] })
    );
}

#[test]
fn hashes_normalized_phone_email_and_names() {
    let contact = AudienceContact {
        email: Some(" Person@Example.COM ".into()),
        phone_number: Some("0912345678".into()),
        first_name: Some(" An ".into()),
        last_name: Some("Nguyen".into()),
        country: Some("VN".into()),
    };
    assert_eq!(
        hashed_payload(&contact, None),
        Some(json!({
            "schema": ["PHONE", "EMAIL", "FN", "LN"],
            "data": [[sha256("84912345678"), sha256("person@example.com"), sha256("an"), sha256("nguyen")]],
        }))
    );
}

#[test]
fn leaves_missing_optional_fields_blank() {
    let contact = AudienceContact {
        email: Some("a@b.co".into()),
        ..AudienceContact::default()
    };
    assert_eq!(
        hashed_payload(&contact, None).unwrap()["data"],
        json!([["", sha256("a@b.co"), "", ""]])
    );
}

#[test]
fn returns_none_when_neither_phone_nor_email_is_present() {
    let contact = AudienceContact {
        first_name: Some("An".into()),
        last_name: Some("Nguyen".into()),
        ..AudienceContact::default()
    };
    assert_eq!(hashed_payload(&contact, None), None);
}
