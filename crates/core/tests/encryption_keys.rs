//! Port of `packages/encryption/__tests__/keys.test.ts`. `process.env`
//! becomes an injected [`Env`](chatbotx_core::env::Env) map.

use chatbotx_core::encryption::Keys;

const VALID_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Port of: packages/encryption/__tests__/keys.test.ts:22 "keys env validation > accepts a valid 32-byte hex key"
#[test]
fn keys_env_validation_accepts_a_valid_32_byte_hex_key() {
    assert!(Keys::from_env(&[("ENCRYPTION_KEY", VALID_KEY)]).is_ok());
}

/// Port of: packages/encryption/__tests__/keys.test.ts:27 "keys env validation > rejects a missing key"
#[test]
fn keys_env_validation_rejects_a_missing_key() {
    assert!(Keys::from_env(&chatbotx_core::env::EmptyEnv).is_err());
}

/// Port of: packages/encryption/__tests__/keys.test.ts:31 "keys env validation > rejects a too-short key"
#[test]
fn keys_env_validation_rejects_a_too_short_key() {
    assert!(Keys::from_env(&[("ENCRYPTION_KEY", "0123abcd")]).is_err());
}

/// Port of: packages/encryption/__tests__/keys.test.ts:36 "keys env validation > rejects a non-hex key of the right length"
#[test]
fn keys_env_validation_rejects_a_non_hex_key_of_the_right_length() {
    let key = "z".repeat(64);
    assert!(Keys::from_env(&[("ENCRYPTION_KEY", key.as_str())]).is_err());
}
