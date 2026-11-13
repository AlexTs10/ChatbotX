//! Port of `packages/business/__tests__/contact-phone.test.ts` (P3.6-D2).
//! Pure functions; one Rust test per `test` / `test.each`.

use chatbotx_business::contact::phone::{
    normalize_identity_phone_to_e164, normalize_phone_to_e164, resolve_default_region,
};

#[test]
fn resolve_default_region_uses_the_workspace_country_when_it_is_known() {
    assert_eq!(resolve_default_region(Some("VN")).as_deref(), Some("VN"));
}

#[test]
fn resolve_default_region_has_no_region_so_numbers_must_carry_a_plus() {
    for country in [None, Some(""), Some("unknown")] {
        assert_eq!(resolve_default_region(country), None, "{country:?}");
    }
}

#[test]
fn normalize_phone_to_e164_keeps_an_international_number_and_strips_separators() {
    assert_eq!(
        normalize_phone_to_e164("+84 901-234-567", None).as_deref(),
        Some("+84901234567")
    );
}

#[test]
fn normalize_phone_to_e164_reads_a_local_number_against_the_workspace_country() {
    assert_eq!(
        normalize_phone_to_e164("0901234567", Some("VN")).as_deref(),
        Some("+84901234567")
    );
}

#[test]
fn normalize_phone_to_e164_an_explicit_country_code_beats_the_workspace_country() {
    assert_eq!(
        normalize_phone_to_e164("+15551234567", Some("VN")).as_deref(),
        Some("+15551234567")
    );
}

#[test]
fn normalize_phone_to_e164_rejects_a_local_number_when_no_country_is_known() {
    assert_eq!(normalize_phone_to_e164("0901234567", None), None);
}

#[test]
fn normalize_phone_to_e164_rejects_junk() {
    for raw in ["", "abc", "+"] {
        assert_eq!(normalize_phone_to_e164(raw, Some("VN")), None, "{raw:?}");
    }
}

#[test]
fn identity_keeps_reading_a_valid_local_number_in_the_workspace_country() {
    assert_eq!(
        normalize_identity_phone_to_e164("0901234567", Some("VN")).as_deref(),
        Some("+84901234567")
    );
}

#[test]
fn identity_reads_a_bare_digits_wa_id_as_international_not_as_a_national_number() {
    assert_eq!(
        normalize_identity_phone_to_e164("447911123456", Some("VN")).as_deref(),
        Some("+447911123456")
    );
    assert_eq!(
        normalize_identity_phone_to_e164("84901234567", Some("US")).as_deref(),
        Some("+84901234567")
    );
}

#[test]
fn identity_a_same_country_wa_id_wins_over_a_bogus_national_reading() {
    for raw in ["4915112345678", "4917612345678", "491701234567"] {
        assert_eq!(
            normalize_identity_phone_to_e164(raw, Some("DE")),
            Some(format!("+{raw}"))
        );
    }
}

#[test]
fn identity_a_local_number_that_merely_looks_like_a_calling_code_keeps_its_national_reading() {
    for (country, local, expected) in [
        ("VN", "846366595", "+84846366595"),
        ("BR", "55991234567", "+5555991234567"),
        ("BR", "5532123456", "+555532123456"),
        ("IN", "9118772020", "+919118772020"),
    ] {
        assert_eq!(
            normalize_identity_phone_to_e164(local, Some(country)).as_deref(),
            Some(expected),
            "{country} {local}"
        );
    }
}

#[test]
fn identity_a_same_country_mobile_wa_id_is_international() {
    for (country, wa_id, expected) in [
        ("VN", "84901234567", "+84901234567"),
        ("BR", "5511987654321", "+5511987654321"),
        ("IN", "919876543210", "+919876543210"),
        ("US", "14155552671", "+14155552671"),
    ] {
        assert_eq!(
            normalize_identity_phone_to_e164(wa_id, Some(country)).as_deref(),
            Some(expected),
            "{country} {wa_id}"
        );
    }
}

#[test]
fn identity_a_local_number_still_reads_as_national_when_its_international_reading_is_another_country()
 {
    assert_eq!(
        normalize_identity_phone_to_e164("2025550123", Some("US")).as_deref(),
        Some("+12025550123")
    );
}

#[test]
fn identity_works_without_a_workspace_country() {
    assert_eq!(
        normalize_identity_phone_to_e164("84901234567", None).as_deref(),
        Some("+84901234567")
    );
    assert_eq!(
        normalize_identity_phone_to_e164("84901234567", Some("unknown")).as_deref(),
        Some("+84901234567")
    );
}

#[test]
fn identity_accepts_a_number_that_already_carries_a_plus() {
    assert_eq!(
        normalize_identity_phone_to_e164("+84 901 234 567", Some("US")).as_deref(),
        Some("+84901234567")
    );
}

#[test]
fn identity_still_rejects_junk() {
    assert_eq!(normalize_identity_phone_to_e164("abc", Some("VN")), None);
    assert_eq!(normalize_identity_phone_to_e164("", Some("VN")), None);
}
