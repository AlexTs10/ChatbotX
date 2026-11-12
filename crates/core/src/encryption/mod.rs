//! Port of `packages/encryption`: AES-256-GCM blobs (`encryptUtils`), the
//! signed tokens built on them, and the HMAC link signatures.
//!
//! Byte compatibility with the original (so stored ciphertexts decrypt):
//! - key: 32 raw bytes from the 64-hex `ENCRYPTION_KEY` (no derivation);
//! - IV: 12 random bytes; tag: 16 bytes; Web Crypto's `ciphertext‖tag`
//!   output is split into `text` and `tag`;
//! - AAD: the UTF-8 bytes of the `aad` string (absent = empty);
//! - blob JSON: `{"v":1,"kid":…,"iv":hex,"text":hex,"tag":hex,"aad"?:…}`,
//!   lower-case hex;
//! - tokens: base64url (no padding) of the blob JSON.

mod binary;
mod signatures;
mod tokens;

use std::fmt;
use std::sync::Arc;

use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::env::Env;
use crate::{Clock, SystemClock};

pub use binary::{bytes_to_hex, concat_bytes, hex_to_bytes};
pub use signatures::{MeLinkParams, OpenLinkParams, UserHashParams};
pub use tokens::*;

/// The blob format version written by [`Encryption::encrypt_text`].
pub const CURRENT_VERSION: i64 = 1;
const IV_LENGTH_BYTES: usize = 12;
const AUTH_TAG_LENGTH_BYTES: usize = 16;
const KEY_LENGTH_BYTES: usize = 32;
const KEY_HEX_LENGTH: usize = 64;
/// Default `ENCRYPTION_KEY_ID`.
pub const DEFAULT_KEY_ID: &str = "default";

/// Errors of the encryption package. The `Display` text is the original
/// error message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncryptionError {
    /// An `ENCRYPTION_KEY*` variable failed validation.
    InvalidKey(String),
    /// `hexToBytes` on an odd-length string.
    OddHex,
    /// A key that does not decode to 32 bytes.
    KeyLength(usize),
    /// A blob version other than 1.
    UnsupportedVersion(i64),
    /// No key matches the blob's `kid`.
    NoKey(String),
    /// AES-GCM authentication failed (wrong key, AAD, or tampered data).
    Decrypt,
    /// Invalid JSON.
    Json(String),
    /// A value that does not match its schema.
    Schema(String),
    /// A token whose AAD is not the expected token type.
    TypeMismatch,
    /// A token past its `expiresAt`.
    Expired,
    /// `verifyAppointmentCancelPostback` on a payload that is not one.
    InvalidPostback,
    /// The system random source failed.
    Random,
}

impl fmt::Display for EncryptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncryptionError::InvalidKey(message) => f.write_str(message),
            EncryptionError::OddHex => f.write_str("Invalid hex string: odd length"),
            EncryptionError::KeyLength(len) => write!(
                f,
                "Encryption key must decode to {KEY_LENGTH_BYTES} bytes; got {len}."
            ),
            EncryptionError::UnsupportedVersion(v) => write!(
                f,
                "Unsupported encryption version: {v}. Expected {CURRENT_VERSION}."
            ),
            EncryptionError::NoKey(kid) => write!(
                f,
                "Cannot decrypt: no key available for kid=\"{kid}\". Set ENCRYPTION_KEY_PREV to the key that encrypted this blob."
            ),
            EncryptionError::Decrypt => {
                f.write_str("The operation failed for an operation-specific reason")
            }
            EncryptionError::Json(message) | EncryptionError::Schema(message) => {
                f.write_str(message)
            }
            EncryptionError::TypeMismatch => f.write_str("Appointment token type mismatch"),
            EncryptionError::Expired => f.write_str("Appointment token has expired"),
            EncryptionError::InvalidPostback => f.write_str("Invalid appointment cancel postback"),
            EncryptionError::Random => f.write_str("random source failed"),
        }
    }
}

impl std::error::Error for EncryptionError {}

const KEY_LENGTH_MESSAGE: &str = "Encryption key must be a 32-byte hex string (64 hex chars). Generate with: openssl rand -hex 32";
const KEY_HEX_MESSAGE: &str = "Encryption key must be hex-encoded (0-9, a-f).";

fn validate_key_hex(name: &str, value: &str) -> Result<[u8; KEY_LENGTH_BYTES], EncryptionError> {
    if crate::js::utf16_len(value) != KEY_HEX_LENGTH {
        return Err(EncryptionError::InvalidKey(format!(
            "{name}: {KEY_LENGTH_MESSAGE}"
        )));
    }
    if !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(EncryptionError::InvalidKey(format!(
            "{name}: {KEY_HEX_MESSAGE}"
        )));
    }
    let bytes = hex_to_bytes(value)?;
    let mut key = [0u8; KEY_LENGTH_BYTES];
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// The validated encryption keys (`packages/encryption/src/keys.ts`).
#[derive(Clone, PartialEq, Eq)]
pub struct Keys {
    key: [u8; KEY_LENGTH_BYTES],
    key_hex: String,
    key_id: String,
    prev: Option<[u8; KEY_LENGTH_BYTES]>,
}

impl fmt::Debug for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Keys")
            .field("key_id", &self.key_id)
            .field("has_prev", &self.prev.is_some())
            .finish_non_exhaustive()
    }
}

impl Keys {
    /// One active key with id `default` and no previous key.
    pub fn new(key_hex: &str) -> Result<Keys, EncryptionError> {
        Keys::with_rotation(key_hex, DEFAULT_KEY_ID, None)
    }

    /// The full rotation set: active key, its id, and the previous key.
    pub fn with_rotation(
        key_hex: &str,
        key_id: &str,
        prev_hex: Option<&str>,
    ) -> Result<Keys, EncryptionError> {
        let key = validate_key_hex("ENCRYPTION_KEY", key_hex)?;
        if key_id.is_empty() {
            return Err(EncryptionError::InvalidKey(
                "ENCRYPTION_KEY_ID: Too small: expected string to have >=1 characters".to_owned(),
            ));
        }
        let prev = prev_hex
            .map(|hex| validate_key_hex("ENCRYPTION_KEY_PREV", hex))
            .transpose()?;
        Ok(Keys {
            key,
            key_hex: key_hex.to_owned(),
            key_id: key_id.to_owned(),
            prev,
        })
    }

    /// `keys()`: reads `ENCRYPTION_KEY`, `ENCRYPTION_KEY_ID` (default
    /// `default`) and `ENCRYPTION_KEY_PREV`, and validates them.
    pub fn from_env(env: &dyn Env) -> Result<Keys, EncryptionError> {
        let Some(key) = env.var("ENCRYPTION_KEY") else {
            return Err(EncryptionError::InvalidKey(
                "ENCRYPTION_KEY: Invalid input: expected string, received undefined".to_owned(),
            ));
        };
        let key_id = env
            .var("ENCRYPTION_KEY_ID")
            .unwrap_or_else(|| DEFAULT_KEY_ID.to_owned());
        let prev = env.var("ENCRYPTION_KEY_PREV");
        Keys::with_rotation(&key, &key_id, prev.as_deref())
    }

    /// The active key id written as `kid`.
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// The active key as hex (the `env.ENCRYPTION_KEY` the HMACs use).
    pub fn key_hex(&self) -> &str {
        &self.key_hex
    }

    pub(crate) fn key_bytes(&self) -> &[u8; KEY_LENGTH_BYTES] {
        &self.key
    }

    /// `resolveKeyHex`: which key decrypts a blob with this `kid`.
    fn resolve(&self, kid: Option<&str>) -> Result<&[u8; KEY_LENGTH_BYTES], EncryptionError> {
        if kid == Some(self.key_id.as_str()) {
            return Ok(&self.key);
        }
        let kid_truthy = kid.is_some_and(|k| !k.is_empty());
        if !kid_truthy && self.prev.is_none() {
            return Ok(&self.key);
        }
        if let Some(prev) = &self.prev {
            return Ok(prev);
        }
        Err(EncryptionError::NoKey(kid.unwrap_or("(none)").to_owned()))
    }
}

/// One encrypted value (`EncryptedData`). Field order is the original
/// object literal order, so the JSON is byte-identical.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedData {
    pub v: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kid: Option<String>,
    pub iv: String,
    pub text: String,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aad: Option<String>,
}

impl EncryptedData {
    /// `encryptedDataSchema.parse`: `v` is 1, `iv` is 24 chars, `tag` 32,
    /// `text` not empty.
    pub fn validate(&self) -> Result<(), EncryptionError> {
        if self.v != CURRENT_VERSION {
            return Err(EncryptionError::Schema(
                "Invalid input: expected 1".to_owned(),
            ));
        }
        if crate::js::utf16_len(&self.iv) != IV_LENGTH_BYTES * 2 {
            return Err(EncryptionError::Schema(
                "iv: Too small/big: expected string to have =24 characters".to_owned(),
            ));
        }
        if self.text.is_empty() {
            return Err(EncryptionError::Schema(
                "text: Too small: expected string to have >=1 characters".to_owned(),
            ));
        }
        if crate::js::utf16_len(&self.tag) != AUTH_TAG_LENGTH_BYTES * 2 {
            return Err(EncryptionError::Schema(
                "tag: Too small/big: expected string to have =32 characters".to_owned(),
            ));
        }
        Ok(())
    }
}

/// `encryptUtils` plus the token and signature helpers, bound to one key set
/// and a clock (`Date.now()` for token expiry).
#[derive(Clone)]
pub struct Encryption {
    keys: Keys,
    clock: Arc<dyn Clock>,
    random: Arc<SystemRandom>,
}

impl fmt::Debug for Encryption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Encryption")
            .field("keys", &self.keys)
            .finish_non_exhaustive()
    }
}

impl Encryption {
    /// Encryption with the system clock.
    pub fn new(keys: Keys) -> Encryption {
        Encryption::with_clock(keys, Arc::new(SystemClock))
    }

    /// Encryption with an injected clock (tests).
    pub fn with_clock(keys: Keys, clock: Arc<dyn Clock>) -> Encryption {
        Encryption {
            keys,
            clock,
            random: Arc::new(SystemRandom::new()),
        }
    }

    /// The key set.
    pub fn keys(&self) -> &Keys {
        &self.keys
    }

    pub(crate) fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    fn cipher(key: &[u8; KEY_LENGTH_BYTES]) -> LessSafeKey {
        // A 32-byte key is always valid for AES-256-GCM.
        LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).expect("32-byte AES key"))
    }

    /// `encryptUtils.encryptText`.
    pub fn encrypt_text(
        &self,
        text: &str,
        aad: Option<&str>,
    ) -> Result<EncryptedData, EncryptionError> {
        let mut iv = [0u8; IV_LENGTH_BYTES];
        self.random
            .fill(&mut iv)
            .map_err(|_| EncryptionError::Random)?;
        self.encrypt_text_with_iv(text, aad, iv)
    }

    fn encrypt_text_with_iv(
        &self,
        text: &str,
        aad: Option<&str>,
        iv: [u8; NONCE_LEN],
    ) -> Result<EncryptedData, EncryptionError> {
        let key = Encryption::cipher(self.keys.key_bytes());
        let mut in_out = text.as_bytes().to_vec();
        let tag = key
            .seal_in_place_separate_tag(
                Nonce::assume_unique_for_key(iv),
                Aad::from(aad.unwrap_or("").as_bytes()),
                &mut in_out,
            )
            .map_err(|_| EncryptionError::Decrypt)?;
        Ok(EncryptedData {
            v: CURRENT_VERSION,
            kid: Some(self.keys.key_id().to_owned()),
            iv: bytes_to_hex(&iv),
            text: bytes_to_hex(&in_out),
            tag: bytes_to_hex(tag.as_ref()),
            aad: aad.map(str::to_owned),
        })
    }

    /// `encryptUtils.decryptText`: `aad` defaults to the blob's own `aad`.
    pub fn decrypt_text(
        &self,
        data: &EncryptedData,
        aad: Option<&str>,
    ) -> Result<String, EncryptionError> {
        if data.v != CURRENT_VERSION {
            return Err(EncryptionError::UnsupportedVersion(data.v));
        }
        let key_bytes = self.keys.resolve(data.kid.as_deref())?;
        let iv = hex_to_bytes(&data.iv)?;
        let mut combined = concat_bytes(&[&hex_to_bytes(&data.text)?, &hex_to_bytes(&data.tag)?]);
        let resolved_aad = aad.or(data.aad.as_deref());
        let nonce: [u8; NONCE_LEN] = iv
            .as_slice()
            .try_into()
            .map_err(|_| EncryptionError::Decrypt)?;
        let key = Encryption::cipher(key_bytes);
        let plain = key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(resolved_aad.unwrap_or("").as_bytes()),
                &mut combined,
            )
            .map_err(|_| EncryptionError::Decrypt)?;
        // `TextDecoder` replaces invalid UTF-8 with U+FFFD.
        Ok(String::from_utf8_lossy(plain).into_owned())
    }

    /// `encryptUtils.encryptObject`: `JSON.stringify(value)` encrypted.
    pub fn encrypt_object<T: Serialize + ?Sized>(
        &self,
        value: &T,
        aad: Option<&str>,
    ) -> Result<EncryptedData, EncryptionError> {
        let json =
            serde_json::to_string(value).map_err(|e| EncryptionError::Json(e.to_string()))?;
        self.encrypt_text(&json, aad)
    }

    /// `encryptUtils.decryptObject`: decrypts, `JSON.parse`s, and checks the
    /// shape (serde stands in for the zod schema).
    pub fn decrypt_object<T: DeserializeOwned>(
        &self,
        data: &EncryptedData,
        aad: Option<&str>,
    ) -> Result<T, EncryptionError> {
        let text = self.decrypt_text(data, aad)?;
        let parsed: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| EncryptionError::Json(e.to_string()))?;
        serde_json::from_value(parsed).map_err(|e| EncryptionError::Schema(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestClock;

    const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    /// Known-answer vector made with Web Crypto (Node 24) for key `KEY`,
    /// IV `000102030405060708090a0b`, text `hello`, AAD `org:1:whatsapp`.
    #[test]
    fn encrypts_byte_identical_to_web_crypto() {
        let enc = Encryption::with_clock(Keys::new(KEY).unwrap(), Arc::new(TestClock::new(0)));
        let iv: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
        let blob = enc
            .encrypt_text_with_iv("hello", Some("org:1:whatsapp"), iv)
            .unwrap();
        assert_eq!(blob.iv, "000102030405060708090a0b");
        assert_eq!(blob.text, "e961f67169");
        assert_eq!(blob.tag, "a40c222f38f76dc392e01e010804cc2a");
        assert_eq!(enc.decrypt_text(&blob, None).unwrap(), "hello");
        assert_eq!(
            serde_json::to_string(&blob).unwrap(),
            "{\"v\":1,\"kid\":\"default\",\"iv\":\"000102030405060708090a0b\",\"text\":\"e961f67169\",\"tag\":\"a40c222f38f76dc392e01e010804cc2a\",\"aad\":\"org:1:whatsapp\"}"
        );
    }

    /// A token minted by the TypeScript `signAppointmentToken` encoding of
    /// the blob above (base64url of its JSON) decodes and decrypts.
    #[test]
    fn decodes_a_token_written_by_the_original() {
        let enc = Encryption::with_clock(Keys::new(KEY).unwrap(), Arc::new(TestClock::new(0)));
        let token = "eyJ2IjoxLCJraWQiOiJkZWZhdWx0IiwiaXYiOiIwMDAxMDIwMzA0MDUwNjA3MDgwOTBhMGIiLCJ0ZXh0IjoiZTk2MWY2NzE2OSIsInRhZyI6ImE0MGMyMjJmMzhmNzZkYzM5MmUwMWUwMTA4MDRjYzJhIiwiYWFkIjoib3JnOjE6d2hhdHNhcHAifQ";
        let blob = tokens::decode_token_blob(token).unwrap();
        assert_eq!(enc.decrypt_text(&blob, None).unwrap(), "hello");
        assert_eq!(tokens::encode_token_blob(&blob).unwrap(), token);
    }

    #[test]
    fn rotation_uses_prev_for_unknown_kid() {
        let old = Keys::new(KEY).unwrap();
        let old_enc = Encryption::new(old);
        let blob = old_enc.encrypt_text("x", None).unwrap();
        let new_key = "f".repeat(64);
        let rotated = Keys::with_rotation(&new_key, "k2", Some(KEY)).unwrap();
        let enc = Encryption::new(rotated);
        assert_eq!(enc.decrypt_text(&blob, None).unwrap(), "x");
        let no_prev = Encryption::new(Keys::with_rotation(&new_key, "k2", None).unwrap());
        assert_eq!(
            no_prev.decrypt_text(&blob, None).unwrap_err().to_string(),
            "Cannot decrypt: no key available for kid=\"default\". Set ENCRYPTION_KEY_PREV to the key that encrypted this blob."
        );
    }

    #[test]
    fn legacy_blob_without_kid_uses_active_key() {
        let enc = Encryption::new(Keys::new(KEY).unwrap());
        let mut blob = enc.encrypt_text("legacy", None).unwrap();
        blob.kid = None;
        assert_eq!(enc.decrypt_text(&blob, None).unwrap(), "legacy");
    }
}
