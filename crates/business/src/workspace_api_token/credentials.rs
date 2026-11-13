//! `workspace-api-token/credentials.ts`: bearer tokens and their hashes.

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
pub fn hash_token(_token: &str) -> String {
    todo!("P3.6-D2")
}

/// `generateApiChannelToken`: `cbx_api_` + base64url (no padding) of 32
/// random bytes.
pub fn generate_api_channel_token() -> ApiChannelCredentials {
    todo!("P3.6-D2")
}

/// `generateSigningSecret`: hex of 32 random bytes.
pub fn generate_signing_secret() -> String {
    todo!("P3.6-D2")
}
