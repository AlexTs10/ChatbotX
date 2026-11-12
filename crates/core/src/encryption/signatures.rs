//! HMAC-SHA256 signatures keyed with the raw `ENCRYPTION_KEY` bytes:
//! `user-hash.ts`, `link-signature.ts` (`signMeLink`) and
//! `open-link-token.ts` (`signOpenLinkUrl`).

use ring::hmac;

use super::{Encryption, bytes_to_hex};

/// `UserHashParams`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserHashParams {
    pub source_id: String,
    pub contact_inbox_id: String,
}

/// `MeLinkParams`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeLinkParams {
    pub workspace_id: String,
    pub source_id: String,
    pub integration_id: String,
    pub form_id: String,
}

/// `OpenLinkParams`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenLinkParams {
    pub workspace_id: String,
    pub url: String,
}

/// `Buffer.from(hash, "hex")`: pairs of hex digits until the first pair
/// that is not one.
fn node_hex_decode(hex: &str) -> Vec<u8> {
    let bytes = hex.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let (Some(hi), Some(lo)) = (
            char::from(pair[0]).to_digit(16),
            char::from(pair[1]).to_digit(16),
        ) else {
            break;
        };
        out.push((hi * 16 + lo) as u8);
    }
    out
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Encryption {
    fn hmac_hex(&self, message: &str) -> String {
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.keys().key_bytes());
        bytes_to_hex(hmac::sign(&key, message.as_bytes()).as_ref())
    }

    fn verify_hmac_hex(&self, message: &str, hash: Option<&str>) -> bool {
        let Some(hash) = hash.filter(|h| !h.is_empty()) else {
            return false;
        };
        let expected = node_hex_decode(&self.hmac_hex(message));
        let actual = node_hex_decode(hash);
        constant_time_eq(&actual, &expected)
    }

    /// `signUserHash`: `HMAC(key, "<sourceId>:<contactInboxId>")` as hex.
    pub fn sign_user_hash(&self, params: &UserHashParams) -> String {
        self.hmac_hex(&format!("{}:{}", params.source_id, params.contact_inbox_id))
    }

    /// `signMeLink`.
    pub fn sign_me_link(&self, params: &MeLinkParams) -> String {
        self.hmac_hex(&me_link_message(params))
    }

    /// `verifyMeLink`: constant-time; `false` for a missing or malformed hash.
    pub fn verify_me_link(&self, params: &MeLinkParams, hash: Option<&str>) -> bool {
        self.verify_hmac_hex(&me_link_message(params), hash)
    }

    /// `signOpenLinkUrl`: deterministic, bound to the workspace.
    pub fn sign_open_link_url(&self, params: &OpenLinkParams) -> String {
        self.hmac_hex(&format!("{}:{}", params.workspace_id, params.url))
    }

    /// `verifyOpenLinkUrl`.
    pub fn verify_open_link_url(&self, params: &OpenLinkParams, signature: Option<&str>) -> bool {
        self.verify_hmac_hex(
            &format!("{}:{}", params.workspace_id, params.url),
            signature,
        )
    }
}

fn me_link_message(params: &MeLinkParams) -> String {
    format!(
        "{}:{}:{}:{}",
        params.workspace_id, params.source_id, params.integration_id, params.form_id
    )
}
