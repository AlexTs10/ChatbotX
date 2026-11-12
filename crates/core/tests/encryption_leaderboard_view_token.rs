//! Port of `packages/encryption/__tests__/leaderboard-view-token.test.ts`.

mod support;

use chatbotx_core::encryption::{
    ContactInboxClaims, LEADERBOARD_VIEW_TOKEN_TTL_MS, MINIGAME_PLAY_TOKEN_TTL_MS,
};
use regex::Regex;

fn claims() -> ContactInboxClaims {
    ContactInboxClaims {
        workspace_id: "workspace-1".to_owned(),
        contact_id: "contact-1".to_owned(),
        contact_inbox_id: "contact-inbox-1".to_owned(),
    }
}

/// Port of: packages/encryption/__tests__/leaderboard-view-token.test.ts:12 "leaderboard view token > round-trips a view token"
#[test]
fn leaderboard_view_token_round_trips_a_view_token() {
    let enc = support::encryption();
    let token = enc
        .sign_leaderboard_view_token(&claims(), LEADERBOARD_VIEW_TOKEN_TTL_MS)
        .unwrap();

    assert!(Regex::new(r"^[A-Za-z0-9\-_]+$").unwrap().is_match(&token));
    assert_eq!(
        enc.verify_leaderboard_view_token(&token).unwrap().claims,
        claims()
    );
}

/// Port of: packages/encryption/__tests__/leaderboard-view-token.test.ts:27 "leaderboard view token > rejects expired tokens"
#[test]
fn leaderboard_view_token_rejects_expired_tokens() {
    let enc = support::encryption();
    let token = enc.sign_leaderboard_view_token(&claims(), -1).unwrap();

    let err = enc.verify_leaderboard_view_token(&token).unwrap_err();
    assert!(err.to_string().contains("expired"), "{err}");
}

/// Port of: packages/encryption/__tests__/leaderboard-view-token.test.ts:40 "leaderboard view token > rejects malformed tokens"
#[test]
fn leaderboard_view_token_rejects_malformed_tokens() {
    let enc = support::encryption();
    assert!(enc.verify_leaderboard_view_token("not-a-token").is_err());
}

/// Port of: packages/encryption/__tests__/leaderboard-view-token.test.ts:44 "leaderboard view token > rejects a minigame play token"
#[test]
fn leaderboard_view_token_rejects_a_minigame_play_token() {
    let enc = support::encryption();
    let token = enc
        .sign_minigame_play_token(&claims(), MINIGAME_PLAY_TOKEN_TTL_MS)
        .unwrap();
    assert!(enc.verify_leaderboard_view_token(&token).is_err());
}
