//! Port of `packages/encryption/__tests__/user-hash.test.ts`. The expected
//! value is computed with `ring::hmac` (stands in for `node:crypto`).

mod support;

use chatbotx_core::encryption::{UserHashParams, hex_to_bytes};
use ring::hmac;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Port of: packages/encryption/__tests__/user-hash.test.ts:7 "user hash signature > signUserHash signs source and contact inbox ids deterministically"
#[test]
fn user_hash_signature_signuserhash_signs_source_and_contact_inbox_ids_deterministically() {
    let enc = support::encryption();
    let key = hmac::Key::new(
        hmac::HMAC_SHA256,
        &hex_to_bytes(support::TEST_ENCRYPTION_KEY).unwrap(),
    );
    let expected = hex(hmac::sign(&key, b"source-1:contact-inbox-1").as_ref());
    let params = UserHashParams {
        source_id: "source-1".to_owned(),
        contact_inbox_id: "contact-inbox-1".to_owned(),
    };

    assert_eq!(enc.sign_user_hash(&params), expected);
    assert_eq!(enc.sign_user_hash(&params), enc.sign_user_hash(&params));
}
