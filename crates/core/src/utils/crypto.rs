//! `packages/utils/src/crypto.ts`.

use ring::{digest, hmac};

use crate::encryption::bytes_to_hex;

/// `sha256Hex`: UTF-8 → SHA-256 → lower-case hex (Meta CAPI hashing).
pub fn sha256_hex(payload: &str) -> String {
    bytes_to_hex(digest::digest(&digest::SHA256, payload.as_bytes()).as_ref())
}

/// `hmacSha256Hex`: HMAC-SHA256 keyed with the UTF-8 secret, as hex.
pub fn hmac_sha256_hex(secret: &str, payload: &str) -> String {
    hmac_hex(secret, payload.as_bytes())
}

fn hmac_hex(secret: &str, payload: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    bytes_to_hex(hmac::sign(&key, payload).as_ref())
}

/// `timingSafeStringEqual`: compares every UTF-16 unit, false on a length
/// mismatch.
pub fn timing_safe_string_equal(a: &str, b: &str) -> bool {
    let a: Vec<u16> = a.encode_utf16().collect();
    let b: Vec<u16> = b.encode_utf16().collect();
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(&b)
        .fold(0u32, |acc, (x, y)| acc + u32::from(x != y))
        == 0
}

/// `DEFAULT_HMAC_SIGNATURE_PREFIX` (`X-Hub-Signature-256`).
pub const DEFAULT_HMAC_SIGNATURE_PREFIX: &str = "sha256=";

/// `VerifyHmacSha256SignatureInput`.
#[derive(Debug, Clone, Copy)]
pub struct VerifyHmacSha256SignatureInput<'a> {
    pub raw_body: &'a [u8],
    pub secret: &'a str,
    pub signature_header: Option<&'a str>,
    /// Defaults to `"sha256="`.
    pub prefix: Option<&'a str>,
}

/// `verifyHmacSha256Signature`: `<prefix><hex>` over the exact raw bytes.
pub fn verify_hmac_sha256_signature(input: VerifyHmacSha256SignatureInput<'_>) -> bool {
    let prefix = input.prefix.unwrap_or(DEFAULT_HMAC_SIGNATURE_PREFIX);
    let Some(header) = input.signature_header else {
        return false;
    };
    let Some(provided) = header.strip_prefix(prefix) else {
        return false;
    };
    let provided = provided.to_lowercase();
    let expected = hmac_hex(input.secret, input.raw_body);
    crate::js::utf16_len(&provided) == expected.len()
        && timing_safe_string_equal(&provided, &expected)
}
