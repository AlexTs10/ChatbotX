//! `workspace-api-token/credentials.ts`: bearer tokens and their hashes.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::{SecureRandom as _, SystemRandom};
use sha2::{Digest as _, Sha256};

const API_TOKEN_PREFIX: &str = "cbx_api_";
const TOKEN_BYTES: usize = 32;
const SIGNING_SECRET_BYTES: usize = 32;
const TOKEN_PREFIX_DISPLAY_LENGTH: usize = 12;

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0_u8; N];
    // `SystemRandom` fails only when the OS RNG is unavailable.
    SystemRandom::new()
        .fill(&mut bytes)
        .unwrap_or_else(|_| panic!("the system random number generator is unavailable"));
    bytes
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `ApiChannelCredentials`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiChannelCredentials {
    pub token: String,
    /// Lowercase SHA-256 hex of `token`.
    pub token_hash: String,
    /// The first 12 characters of `token`.
    pub token_prefix: String,
}

/// `hashToken`: SHA-256 hex of the UTF-8 token.
pub fn hash_token(token: &str) -> String {
    to_hex(&Sha256::digest(token.as_bytes()))
}

/// `generateApiChannelToken`: `cbx_api_` + base64url (no padding) of 32
/// random bytes.
pub fn generate_api_channel_token() -> ApiChannelCredentials {
    let token = format!(
        "{API_TOKEN_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(random_bytes::<TOKEN_BYTES>())
    );
    ApiChannelCredentials {
        token_hash: hash_token(&token),
        token_prefix: token.chars().take(TOKEN_PREFIX_DISPLAY_LENGTH).collect(),
        token,
    }
}

/// `generateSigningSecret`: hex of 32 random bytes.
pub fn generate_signing_secret() -> String {
    to_hex(&random_bytes::<SIGNING_SECRET_BYTES>())
}
