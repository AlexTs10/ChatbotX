//! Port of `packages/encryption/__tests__/media-token.test.ts`.

mod support;

use chatbotx_core::encryption::{MEDIA_TOKEN_TTL_MS, MediaTokenClaims, MediaTokenKind};
use regex::Regex;

fn claims(kind: MediaTokenKind, ref_id: &str) -> MediaTokenClaims {
    MediaTokenClaims {
        workspace_id: "workspace-1".to_owned(),
        kind,
        ref_id: ref_id.to_owned(),
        message_created_at: None,
    }
}

/// Port of: packages/encryption/__tests__/media-token.test.ts:8 "media token > round-trips media claims with an expiry"
#[test]
fn media_token_round_trips_media_claims_with_an_expiry() {
    let enc = support::encryption();
    let token = enc
        .sign_media_token(
            &claims(MediaTokenKind::Attachment, "attachment-1"),
            MEDIA_TOKEN_TTL_MS,
        )
        .unwrap();

    assert!(Regex::new(r"^[A-Za-z0-9\-_]+$").unwrap().is_match(&token));
    let payload = enc.verify_media_token(&token).unwrap();
    assert_eq!(payload.claims.workspace_id, "workspace-1");
    assert_eq!(payload.claims.kind, MediaTokenKind::Attachment);
    assert_eq!(payload.claims.ref_id, "attachment-1");
    // `expiresAt: expect.any(Number)`: an i64 by type.
    assert!(payload.expires_at > 0);
}

/// Port of: packages/encryption/__tests__/media-token.test.ts:24 "media token > round-trips the optional messageCreatedAt shard hint"
#[test]
fn media_token_round_trips_the_optional_messagecreatedat_shard_hint() {
    let enc = support::encryption();
    let message_created_at = support::iso_ms("2026-01-03T12:00:00Z");
    let token = enc
        .sign_media_token(
            &MediaTokenClaims {
                message_created_at: Some(message_created_at),
                ..claims(MediaTokenKind::Attachment, "attachment-1")
            },
            MEDIA_TOKEN_TTL_MS,
        )
        .unwrap();

    let payload = enc.verify_media_token(&token).unwrap();
    assert_eq!(payload.claims.ref_id, "attachment-1");
    assert_eq!(payload.claims.message_created_at, Some(message_created_at));
}

/// Port of: packages/encryption/__tests__/media-token.test.ts:39 "media token > verifies tokens minted without the messageCreatedAt hint"
#[test]
fn media_token_verifies_tokens_minted_without_the_messagecreatedat_hint() {
    let enc = support::encryption();
    let token = enc
        .sign_media_token(
            &claims(MediaTokenKind::Avatar, "contact-inbox-1"),
            MEDIA_TOKEN_TTL_MS,
        )
        .unwrap();

    let payload = enc.verify_media_token(&token).unwrap();
    assert_eq!(payload.claims.message_created_at, None);
}

/// Port of: packages/encryption/__tests__/media-token.test.ts:50 "media token > rejects tampered tokens"
#[test]
fn media_token_rejects_tampered_tokens() {
    let enc = support::encryption();
    let token = enc
        .sign_media_token(
            &claims(MediaTokenKind::Avatar, "contact-inbox-1"),
            MEDIA_TOKEN_TTL_MS,
        )
        .unwrap();
    let last = token.chars().last().unwrap();
    let tampered = format!(
        "{}{}",
        &token[..token.len() - 1],
        if last == 'A' { 'B' } else { 'A' }
    );

    assert!(enc.verify_media_token(&tampered).is_err());
}

/// Port of: packages/encryption/__tests__/media-token.test.ts:62 "media token > rejects expired tokens"
#[test]
fn media_token_rejects_expired_tokens() {
    let enc = support::encryption();
    let token = enc
        .sign_media_token(&claims(MediaTokenKind::Attachment, "attachment-1"), -1)
        .unwrap();

    let err = enc.verify_media_token(&token).unwrap_err();
    assert!(err.to_string().contains("expired"), "{err}");
}
