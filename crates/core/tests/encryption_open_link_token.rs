//! Port of `packages/encryption/__tests__/open-link-token.test.ts`.

mod support;

use chatbotx_core::encryption::OpenLinkParams;

fn params() -> OpenLinkParams {
    OpenLinkParams {
        workspace_id: "workspace-1".to_owned(),
        url: "https://zalo.me/g/owfqvp123".to_owned(),
    }
}

/// Port of: packages/encryption/__tests__/open-link-token.test.ts:10 "open link token > signOpenLinkUrl is deterministic"
#[test]
fn open_link_token_signopenlinkurl_is_deterministic() {
    let enc = support::encryption();
    assert_eq!(
        enc.sign_open_link_url(&params()),
        enc.sign_open_link_url(&params())
    );
}

/// Port of: packages/encryption/__tests__/open-link-token.test.ts:17 "open link token > a different destination produces a different signature"
#[test]
fn open_link_token_a_different_destination_produces_a_different_signature() {
    let enc = support::encryption();
    let other = OpenLinkParams {
        url: "https://zalo.me/g/other".to_owned(),
        ..params()
    };
    assert_ne!(
        enc.sign_open_link_url(&other),
        enc.sign_open_link_url(&params())
    );
}

/// Port of: packages/encryption/__tests__/open-link-token.test.ts:23 "open link token > verifyOpenLinkUrl accepts its own signature"
#[test]
fn open_link_token_verifyopenlinkurl_accepts_its_own_signature() {
    let enc = support::encryption();
    let signature = enc.sign_open_link_url(&params());
    assert!(enc.verify_open_link_url(&params(), Some(&signature)));
}

/// Port of: packages/encryption/__tests__/open-link-token.test.ts:27 "open link token > verifyOpenLinkUrl rejects a tampered destination"
#[test]
fn open_link_token_verifyopenlinkurl_rejects_a_tampered_destination() {
    let enc = support::encryption();
    let signature = enc.sign_open_link_url(&params());
    let tampered = OpenLinkParams {
        url: "https://evil.example/".to_owned(),
        ..params()
    };
    assert!(!enc.verify_open_link_url(&tampered, Some(&signature)));
}

/// Port of: packages/encryption/__tests__/open-link-token.test.ts:35 "open link token > verifyOpenLinkUrl rejects a signature minted for another workspace"
#[test]
fn open_link_token_verifyopenlinkurl_rejects_a_signature_minted_for_another_workspace() {
    let enc = support::encryption();
    let signature = enc.sign_open_link_url(&OpenLinkParams {
        workspace_id: "workspace-2".to_owned(),
        ..params()
    });
    assert!(!enc.verify_open_link_url(&params(), Some(&signature)));
}

/// Port of: packages/encryption/__tests__/open-link-token.test.ts:41 "open link token > verifyOpenLinkUrl rejects malformed input without throwing"
#[test]
fn open_link_token_verifyopenlinkurl_rejects_malformed_input_without_throwing() {
    let enc = support::encryption();
    assert!(!enc.verify_open_link_url(&params(), Some("abc")));
    assert!(!enc.verify_open_link_url(&params(), Some("")));
    // `null` and `undefined` are both `None`.
    assert!(!enc.verify_open_link_url(&params(), None));
    assert!(!enc.verify_open_link_url(&params(), None));
}
