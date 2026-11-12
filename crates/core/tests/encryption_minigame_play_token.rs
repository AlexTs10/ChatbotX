//! Port of `packages/encryption/__tests__/minigame-play-token.test.ts`.

mod support;

use chatbotx_core::encryption::{ContactInboxClaims, MINIGAME_PLAY_TOKEN_TTL_MS};
use regex::Regex;

fn claims() -> ContactInboxClaims {
    ContactInboxClaims {
        workspace_id: "workspace-1".to_owned(),
        contact_id: "contact-1".to_owned(),
        contact_inbox_id: "contact-inbox-1".to_owned(),
    }
}

/// Port of: packages/encryption/__tests__/minigame-play-token.test.ts:11 "minigame play token > round-trips a play token"
#[test]
fn minigame_play_token_round_trips_a_play_token() {
    let enc = support::encryption();
    let token = enc
        .sign_minigame_play_token(&claims(), MINIGAME_PLAY_TOKEN_TTL_MS)
        .unwrap();

    assert!(Regex::new(r"^[A-Za-z0-9\-_]+$").unwrap().is_match(&token));
    assert_eq!(
        enc.verify_minigame_play_token(&token).unwrap().claims,
        claims()
    );
}

/// Port of: packages/encryption/__tests__/minigame-play-token.test.ts:26 "minigame play token > rejects expired tokens"
#[test]
fn minigame_play_token_rejects_expired_tokens() {
    let enc = support::encryption();
    let token = enc.sign_minigame_play_token(&claims(), -1).unwrap();

    let err = enc.verify_minigame_play_token(&token).unwrap_err();
    assert!(err.to_string().contains("expired"), "{err}");
}

/// Port of: packages/encryption/__tests__/minigame-play-token.test.ts:39 "minigame play token > rejects malformed tokens"
#[test]
fn minigame_play_token_rejects_malformed_tokens() {
    let enc = support::encryption();
    assert!(enc.verify_minigame_play_token("not-a-token").is_err());
}
