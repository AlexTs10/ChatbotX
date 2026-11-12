//! Port of `packages/encryption/__tests__/link-signature.test.ts`. The
//! expected value is computed with `ring::hmac` (stands in for `node:crypto`).

mod support;

use chatbotx_core::encryption::{MeLinkParams, hex_to_bytes};
use ring::hmac;

fn params() -> MeLinkParams {
    MeLinkParams {
        workspace_id: "workspace-1".to_owned(),
        source_id: "source-1".to_owned(),
        integration_id: "integration-1".to_owned(),
        form_id: "system-field-1".to_owned(),
    }
}

/// Port of: packages/encryption/__tests__/link-signature.test.ts:14 "me link signature > signMeLink is deterministic"
#[test]
fn me_link_signature_signmelink_is_deterministic() {
    let enc = support::encryption();
    let key = hmac::Key::new(
        hmac::HMAC_SHA256,
        &hex_to_bytes(support::TEST_ENCRYPTION_KEY).unwrap(),
    );
    let expected: String = hmac::sign(&key, b"workspace-1:source-1:integration-1:system-field-1")
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    assert_eq!(enc.sign_me_link(&params()), expected);
    assert_eq!(enc.sign_me_link(&params()), enc.sign_me_link(&params()));
}

/// Port of: packages/encryption/__tests__/link-signature.test.ts:26 "me link signature > verifyMeLink rejects tampered and short hashes"
#[test]
fn me_link_signature_verifymelink_rejects_tampered_and_short_hashes() {
    let enc = support::encryption();
    let hash = enc.sign_me_link(&params());

    assert!(enc.verify_me_link(&params(), Some(&hash)));
    let other = MeLinkParams {
        source_id: "other-source".to_owned(),
        ..params()
    };
    assert!(!enc.verify_me_link(&other, Some(&hash)));
    assert!(!enc.verify_me_link(&params(), Some("abc")));
    assert!(!enc.verify_me_link(&params(), None));
}
