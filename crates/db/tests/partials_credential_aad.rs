//! Port of `packages/database/__tests__/credential-aad.test.ts` (P3.2b, ported). Stored
//! ciphertext is bound to these exact strings.

use chatbotx_db::repo::partials::credential::{
    credential_aad_for, platform_credential_aad, user_credential_aad,
};

#[test]
fn credential_aad_format_user_aad_keeps_the_legacy_false_suffix() {
    assert_eq!(
        user_credential_aad("123", "stripe"),
        "user:123:stripe:false"
    );
}

#[test]
fn credential_aad_format_platform_aad_keeps_the_legacy_false_suffix() {
    assert_eq!(
        platform_credential_aad("whatsapp"),
        "platform:whatsapp:false"
    );
}

#[test]
fn credential_aad_format_selects_the_aad_from_credential_ownership() {
    assert_eq!(
        credential_aad_for(Some("123"), "stripe"),
        "user:123:stripe:false"
    );
    assert_eq!(
        credential_aad_for(None, "whatsapp"),
        "platform:whatsapp:false"
    );
}
