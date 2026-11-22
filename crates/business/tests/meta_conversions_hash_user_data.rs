//! Port of `packages/business/src/meta-conversions/__tests__/hash-user-data.test.ts`.

use chatbotx_business::meta_capi_send::{HashableContact, hash_contact_user_data};
use serde_json::{Value, json};

fn hash(contact: HashableContact) -> Value {
    hash_contact_user_data(&contact)
}

fn with(f: impl FnOnce(&mut HashableContact)) -> Value {
    let mut contact = HashableContact {
        id: "contact-1".into(),
        ..HashableContact::default()
    };
    f(&mut contact);
    hash(contact)
}

/// Port of: "hashes email using Meta's official test vector"
#[test]
fn hashes_email_using_metas_test_vector() {
    let r = with(|c| c.email = Some("John_Smith@gmail.com".into()));
    assert_eq!(
        r["em"],
        json!(["62a14e44f765419d10fea99367361a727c12365e2520f32218d505ed9aa0f62f"])
    );
}

/// Port of: "hashes a valid E.164-parseable phone using Meta's official test vector"
#[test]
fn hashes_a_valid_phone_using_metas_test_vector() {
    let r = with(|c| c.phone_number = Some("+16505551212".into()));
    assert_eq!(
        r["ph"],
        json!(["e323ec626319ca94ee8bff2e4c87cf613be6ea19919ed1364124e16807ab3176"])
    );
}

/// Port of: "hashes first name using Meta's official test vector"
#[test]
fn hashes_first_name_using_metas_test_vector() {
    let r = with(|c| c.first_name = Some("Mary".into()));
    assert_eq!(
        r["fn"],
        json!(["6915771be1c5aa0c886870b6951b03d7eafc121fea0e80a5ea83beb7c449f4ec"])
    );
}

/// Port of: "strips interior whitespace (not just trims) for multi-word first names"
#[test]
fn strips_interior_whitespace_for_first_names() {
    let r = with(|c| c.first_name = Some("John Smith".into()));
    assert_eq!(
        r["fn"],
        json!(["45358dd8587f2de278ec7411a560bae78720c13aaa369d0cd95b67715adb445c"])
    );
}

/// Port of: "strips interior whitespace and punctuation for multi-word last names"
#[test]
fn strips_interior_whitespace_for_last_names() {
    let r = with(|c| c.last_name = Some("Mary Jane".into()));
    assert_eq!(
        r["ln"],
        json!(["f08f448a5e7a9dc3619bb7c129f6a7d5fc6af002cea17ad71dfdc1c68f4d4e0e"])
    );
}

/// Port of: "strips punctuation (hyphen) for hyphenated last names"
#[test]
fn strips_hyphens_for_last_names() {
    let r = with(|c| c.last_name = Some("Smith-Jones".into()));
    assert_eq!(
        r["ln"],
        json!(["6a77f47e3dec33252edbebb457d65e90501b643806c50cf8d6c3e2bd9cb74681"])
    );
}

/// Port of: "hashes the opaque contact id, unnormalized, as external_id"
#[test]
fn hashes_the_opaque_contact_id_as_external_id() {
    let r = hash(HashableContact {
        id: "e2e-contact-42".into(),
        ..HashableContact::default()
    });
    assert_eq!(
        r["external_id"],
        json!(["74c1082af713785f2f4ada1c8e966a8d46a96563fb52363a27ca03cb0b37bbb1"])
    );
}

/// Port of: "omits ph for a phone that cannot be parsed as a valid number"
/// (+ "omits ph for an unparseable phone string without throwing")
#[test]
fn omits_ph_for_an_invalid_phone() {
    for phone in ["12345", "not-a-phone"] {
        let r = with(|c| c.phone_number = Some(phone.into()));
        assert!(r.get("ph").is_none(), "{phone}");
    }
}

/// Port of: "omits em for a malformed email address"
#[test]
fn omits_em_for_a_malformed_email() {
    let r = with(|c| c.email = Some("not-an-email".into()));
    assert!(r.get("em").is_none());
}

/// Port of: "only includes present fields, plus external_id always"
#[test]
fn only_includes_present_fields() {
    let r = with(|c| c.email = Some("john_smith@gmail.com".into()));
    let mut keys: Vec<&String> = r.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["em", "external_id"]);
}

fn full() -> HashableContact {
    HashableContact {
        id: "contact-1".into(),
        email: Some("john_smith@gmail.com".into()),
        first_name: Some("Mary".into()),
        last_name: Some("Jane".into()),
        phone_number: Some("+16505551212".into()),
    }
}

/// Port of: "every present field is an array with exactly one hash string"
#[test]
fn every_field_is_one_hash() {
    let r = hash(full());
    for value in r.as_object().unwrap().values() {
        let list = value.as_array().unwrap();
        assert_eq!(list.len(), 1);
        let h = list[0].as_str().unwrap();
        assert!(
            h.len() == 64
                && h.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }
}

/// Port of: "never leaks plaintext PII into the hashed output"
#[test]
fn never_leaks_plaintext_pii() {
    let text = hash(full()).to_string();
    for leaked in [
        "john_smith@gmail.com",
        "Mary",
        "Jane",
        "+16505551212",
        "6505551212",
    ] {
        assert!(!text.contains(leaked), "{leaked}");
    }
}
