//! Port of `apps/builder/__tests__/integration-api-token-hash.test.ts`
//! (`workspace-api-token/credentials.ts`).

use chatbotx_business::workspace_api_token::credentials::{generate_api_channel_token, hash_token};

const TOKEN_PREFIX: &str = "cbx_api_";
const SHA256_HEX_LENGTH: usize = 64;
// 8-char prefix + 43 chars of unpadded base64url for 32 random bytes.
const TOKEN_LENGTH: usize = 51;

fn is_lowercase_hex(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_base64url(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[test]
fn integration_api_token_hashing_pins_the_digest_algorithm_to_plain_sha_256_hex() {
    // Known-answer fixture: sha256("ws-1.fixture").
    assert_eq!(
        hash_token("ws-1.fixture"),
        "c398e4c2a5927596abf8442ec42853ef435b49bc5ce25aab22dcd7f8e1447ea9"
    );
}

#[test]
fn integration_api_token_hashing_stored_token_hash_matches_what_the_auth_middleware_computes_for_the_token()
 {
    let credentials = generate_api_channel_token();
    let verification_hash = hash_token(&credentials.token);
    assert_eq!(verification_hash, credentials.token_hash);
}

#[test]
fn integration_api_token_hashing_token_hash_is_lowercase_sha_256_hex() {
    let credentials = generate_api_channel_token();
    assert!(is_lowercase_hex(&credentials.token_hash));
    assert_eq!(credentials.token_hash.len(), SHA256_HEX_LENGTH);
}

#[test]
fn integration_api_token_hashing_token_starts_with_the_display_prefix_and_stays_in_the_base64url_alphabet()
 {
    let credentials = generate_api_channel_token();
    assert!(credentials.token.starts_with(TOKEN_PREFIX));
    assert!(is_base64url(&credentials.token[TOKEN_PREFIX.len()..]));
    assert_eq!(credentials.token.len(), TOKEN_LENGTH);
}

#[test]
fn integration_api_token_hashing_token_prefix_carries_distinguishing_characters_beyond_the_static_literal()
 {
    let credentials = generate_api_channel_token();
    assert!(credentials.token_prefix.len() > TOKEN_PREFIX.len());
    assert!(credentials.token.starts_with(&credentials.token_prefix));
}

#[test]
fn integration_api_token_hashing_generates_a_unique_token_and_hash_per_call() {
    let first = generate_api_channel_token();
    let second = generate_api_channel_token();
    assert_ne!(first.token, second.token);
    assert_ne!(first.token_hash, second.token_hash);
}
