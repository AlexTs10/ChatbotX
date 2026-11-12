//! Port of `packages/encryption/__tests__/encryption.test.ts`.
//! zod schemas become serde structs (`decryptObject::<T>`).

mod support;

use chatbotx_core::encryption::EncryptedData;
use regex::Regex;
use serde::Deserialize;

fn flip_first_hex_nibble(hex: &str) -> String {
    let head = hex.chars().next().unwrap_or('0').to_ascii_lowercase();
    let flipped = match head {
        '0' => 'f',
        '1' => 'e',
        '2' => 'd',
        '3' => 'c',
        '4' => 'b',
        '5' => 'a',
        '6' => '9',
        '7' => '8',
        '8' => '7',
        '9' => '6',
        'a' => '5',
        'b' => '4',
        'c' => '3',
        'd' => '2',
        'e' => '1',
        'f' => '0',
        _ => '0',
    };
    format!("{flipped}{}", &hex[1..])
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:11 "encryptUtils > text round-trip > decrypts what was encrypted"
#[test]
fn encryptutils_text_round_trip_decrypts_what_was_encrypted() {
    let enc = support::encryption();
    let plaintext = "hunter2 — special chars: 你好 🚀";
    let blob = enc.encrypt_text(plaintext, None).unwrap();
    assert_eq!(enc.decrypt_text(&blob, None).unwrap(), plaintext);
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:17 "encryptUtils > text round-trip > produces a versioned blob with iv, text, tag"
#[test]
fn encryptutils_text_round_trip_produces_a_versioned_blob_with_iv_text_tag() {
    let enc = support::encryption();
    let blob = enc.encrypt_text("abc", None).unwrap();
    assert_eq!(blob.v, 1);
    assert!(Regex::new("^[0-9a-f]{24}$").unwrap().is_match(&blob.iv));
    assert!(Regex::new("^[0-9a-f]{32}$").unwrap().is_match(&blob.tag));
    assert!(!blob.text.is_empty());
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:25 "encryptUtils > text round-trip > two encryptions of same plaintext yield different ciphertexts"
#[test]
fn encryptutils_text_round_trip_two_encryptions_of_same_plaintext_yield_different_ciphertexts() {
    let enc = support::encryption();
    let a = enc.encrypt_text("same", None).unwrap();
    let b = enc.encrypt_text("same", None).unwrap();
    assert_ne!(a.iv, b.iv);
    assert_ne!(a.text, b.text);
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:34 "encryptUtils > tamper detection > flipping a byte in ciphertext throws"
#[test]
fn encryptutils_tamper_detection_flipping_a_byte_in_ciphertext_throws() {
    let enc = support::encryption();
    let blob = enc.encrypt_text("attack at dawn", None).unwrap();
    let flipped = EncryptedData {
        text: flip_first_hex_nibble(&blob.text),
        ..blob
    };
    assert!(enc.decrypt_text(&flipped, None).is_err());
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:43 "encryptUtils > tamper detection > flipping a byte in the auth tag throws"
#[test]
fn encryptutils_tamper_detection_flipping_a_byte_in_the_auth_tag_throws() {
    let enc = support::encryption();
    let blob = enc.encrypt_text("attack at dawn", None).unwrap();
    let flipped = EncryptedData {
        tag: flip_first_hex_nibble(&blob.tag),
        ..blob
    };
    assert!(enc.decrypt_text(&flipped, None).is_err());
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:52 "encryptUtils > tamper detection > flipping a byte in the IV throws"
#[test]
fn encryptutils_tamper_detection_flipping_a_byte_in_the_iv_throws() {
    let enc = support::encryption();
    let blob = enc.encrypt_text("attack at dawn", None).unwrap();
    let flipped = EncryptedData {
        iv: flip_first_hex_nibble(&blob.iv),
        ..blob
    };
    assert!(enc.decrypt_text(&flipped, None).is_err());
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:63 "encryptUtils > version handling > unknown version is rejected"
#[test]
fn encryptutils_version_handling_unknown_version_is_rejected() {
    let enc = support::encryption();
    let blob = enc.encrypt_text("hi", None).unwrap();
    let bogus = EncryptedData { v: 999, ..blob };
    let err = enc.decrypt_text(&bogus, None).unwrap_err();
    assert!(
        Regex::new("Unsupported encryption version")
            .unwrap()
            .is_match(&err.to_string())
    );
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:73 "encryptUtils > aad binding > encrypts and decrypts with matching aad"
#[test]
fn encryptutils_aad_binding_encrypts_and_decrypts_with_matching_aad() {
    let enc = support::encryption();
    let blob = enc.encrypt_text("secret", Some("org:1:whatsapp")).unwrap();
    assert_eq!(
        enc.decrypt_text(&blob, Some("org:1:whatsapp")).unwrap(),
        "secret"
    );
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:80 "encryptUtils > aad binding > decrypting with wrong aad throws"
#[test]
fn encryptutils_aad_binding_decrypting_with_wrong_aad_throws() {
    let enc = support::encryption();
    let blob = enc.encrypt_text("secret", Some("org:1:whatsapp")).unwrap();
    assert!(enc.decrypt_text(&blob, Some("org:2:whatsapp")).is_err());
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:87 "encryptUtils > aad binding > decrypting without aad falls back to the aad stored on the blob"
#[test]
fn encryptutils_aad_binding_decrypting_without_aad_falls_back_to_the_aad_stored_on_the_blob() {
    let enc = support::encryption();
    let blob = enc.encrypt_text("secret", Some("org:1:whatsapp")).unwrap();
    assert_eq!(enc.decrypt_text(&blob, None).unwrap(), "secret");
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:92 "encryptUtils > aad binding > decrypting with aad throws when no aad was used at encryption"
#[test]
fn encryptutils_aad_binding_decrypting_with_aad_throws_when_no_aad_was_used_at_encryption() {
    let enc = support::encryption();
    let blob = enc.encrypt_text("secret", None).unwrap();
    assert!(enc.decrypt_text(&blob, Some("org:1:whatsapp")).is_err());
}

#[derive(Debug, PartialEq, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientCredentials {
    client_id: String,
    client_secret: String,
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:99 "encryptUtils > aad binding > encryptObject/decryptObject round-trip with matching aad"
#[test]
fn encryptutils_aad_binding_encryptobject_decryptobject_round_trip_with_matching_aad() {
    let enc = support::encryption();
    let original = ClientCredentials {
        client_id: "app_123".to_owned(),
        client_secret: "s3cr3t".to_owned(),
    };
    let blob = enc
        .encrypt_object(&original, Some("org:1:messenger"))
        .unwrap();
    let recovered: ClientCredentials = enc.decrypt_object(&blob, Some("org:1:messenger")).unwrap();
    assert_eq!(recovered, original);
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:111 "encryptUtils > aad binding > decryptObject with wrong aad throws"
#[test]
fn encryptutils_aad_binding_decryptobject_with_wrong_aad_throws() {
    let enc = support::encryption();
    let blob = enc
        .encrypt_object(
            &ClientCredentials {
                client_id: "app_123".to_owned(),
                client_secret: "s3cr3t".to_owned(),
            },
            Some("org:1:messenger"),
        )
        .unwrap();
    let result: Result<ClientCredentials, _> = enc.decrypt_object(&blob, Some("org:9:messenger"));
    assert!(result.is_err());
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Secrets {
    api_key: String,
    verify_token: String,
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:132 "encryptUtils > object round-trip > decryptObject parses against schema"
#[test]
fn encryptutils_object_round_trip_decryptobject_parses_against_schema() {
    let enc = support::encryption();
    let original = serde_json::json!({ "apiKey": "k_live_123", "verifyToken": "vt-xyz" });
    let blob = enc.encrypt_object(&original, None).unwrap();
    let recovered: Secrets = enc.decrypt_object(&blob, None).unwrap();
    assert_eq!(
        recovered,
        Secrets {
            api_key: "k_live_123".to_owned(),
            verify_token: "vt-xyz".to_owned(),
        }
    );
}

/// Port of: packages/encryption/__tests__/encryption.test.ts:139 "encryptUtils > object round-trip > decryptObject throws when the decrypted value does not match schema"
#[test]
fn encryptutils_object_round_trip_decryptobject_throws_when_the_decrypted_value_does_not_match_schema()
 {
    let enc = support::encryption();
    let blob = enc
        .encrypt_object(&serde_json::json!({ "apiKey": 42 }), None)
        .unwrap();
    let result: Result<Secrets, _> = enc.decrypt_object(&blob, None);
    assert!(result.is_err());
}
