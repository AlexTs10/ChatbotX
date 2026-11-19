//! Port of `packages/business/__tests__/google-ads-hash-user-data.test.ts`.

use chatbotx_business::google_ads_conversion::{
    hash_matching_identifiers, normalize_email, normalize_phone,
};
use chatbotx_core::utils::crypto::sha256_hex as sha;
use serde_json::json;

/// Port of: "normalizeEmail (Google rules) > %s -> %s"
#[test]
fn normalize_email_follows_google_rules() {
    for (raw, expected) in [
        (
            "Cloudy.SanFrancisco+Shopping@Gmail.com",
            "cloudysanfrancisco@gmail.com",
        ),
        ("  a.b.c@googlemail.com  ", "abc@googlemail.com"),
        ("A.B+tag@Example.COM", "a.b+tag@example.com"),
        ("first last@example.com", "firstlast@example.com"),
    ] {
        assert_eq!(normalize_email(raw).as_deref(), Some(expected), "{raw}");
    }
}

/// Port of: "normalizeEmail (Google rules) > rejects %j"
#[test]
fn normalize_email_rejects_malformed_addresses() {
    for raw in [
        "",
        "no-at-sign",
        "a@@example.com",
        "a@b",
        "@example.com",
        "+tag@gmail.com",
        "...@gmail.com",
    ] {
        assert_eq!(normalize_email(raw), None, "{raw:?}");
    }
}

/// Port of: "normalizePhone (E.164 with the +) > does not pull a number out of surrounding prose"
#[test]
fn normalize_phone_does_not_pull_a_number_out_of_surrounding_prose() {
    assert_eq!(normalize_phone("Call my assistant at +14155552671"), None);
}

/// Port of: "normalizePhone (E.164 with the +) > keeps the leading + and removes formatting"
#[test]
fn normalize_phone_keeps_the_leading_plus_and_removes_formatting() {
    assert_eq!(
        normalize_phone(" +1 (415) 555-2671 ").as_deref(),
        Some("+14155552671")
    );
    assert_eq!(
        normalize_phone("+84 90 123 4567").as_deref(),
        Some("+84901234567")
    );
}

/// Port of: "normalizePhone (E.164 with the +) > a WhatsApp wa_id (international digits, no +) is read as international"
#[test]
fn normalize_phone_reads_a_whatsapp_wa_id_as_international() {
    assert_eq!(
        normalize_phone("84901234567").as_deref(),
        Some("+84901234567")
    );
    assert_eq!(
        normalize_phone(" 14155552671 ").as_deref(),
        Some("+14155552671")
    );
}

/// Port of: "normalizePhone (E.164 with the +) > rejects %s"
#[test]
fn normalize_phone_rejects_what_it_cannot_validate() {
    for raw in [
        "0901234567",
        "12345678",
        "8490123456789012",
        "+14155552671 ext. 12",
        "+1 123",
        "call me",
        "",
    ] {
        assert_eq!(normalize_phone(raw), None, "{raw:?}");
    }
}

/// Port of: "hashMatchingIdentifiers > is the lowercase hex SHA-256 of the normalised value"
#[test]
fn hash_matching_identifiers_is_the_hex_sha256_of_the_normalised_value() {
    let hashed = hash_matching_identifiers(
        Some("Cloudy.SanFrancisco+Shopping@Gmail.com"),
        Some("+1 415 555 2671"),
    )
    .expect("hashed");
    assert_eq!(
        hashed,
        json!({
            "emailAddress": sha("cloudysanfrancisco@gmail.com"),
            "phoneNumber": sha("+14155552671"),
        })
    );
    let digest = hashed["emailAddress"].as_str().expect("hex");
    assert!(
        digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
}

/// Port of: "hashMatchingIdentifiers > omits an identifier that is missing or cannot be normalised"
#[test]
fn hash_matching_identifiers_omits_what_cannot_be_normalised() {
    // `{}` in the original is `None` here (nothing to send).
    assert_eq!(hash_matching_identifiers(None, None), None);
    assert_eq!(
        hash_matching_identifiers(Some("nope"), Some("0901234567")),
        None
    );
    assert_eq!(
        hash_matching_identifiers(None, Some("+14155552671")),
        Some(json!({ "phoneNumber": sha("+14155552671") }))
    );
}

/// Port of: "hashMatchingIdentifiers > the same contact value always hashes to the same digest"
#[test]
fn hash_matching_identifiers_is_stable_for_the_same_contact_value() {
    assert_eq!(
        hash_matching_identifiers(Some("A@b.co"), None),
        hash_matching_identifiers(Some(" a@B.co "), None)
    );
}
