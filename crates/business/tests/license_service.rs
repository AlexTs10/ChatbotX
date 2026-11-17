//! `packages/business/__tests__/license.service.test.ts` (Q.D11 slice 10):
//! Ed25519 license tokens signed here with `ring`, checked by
//! `verify_license_token` and `resolve_license` (the stored token is a real
//! `License` row instead of the mocked repository).

mod common;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chatbotx_business::keys::Edition;
use chatbotx_business::license::{
    LICENSE_ISSUER, LicenseLimits, LicenseState, LicenseStatus, resolve_license,
    verify_license_token,
};
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

const KID: &str = "test-license-key";
/// `2026-01-01T00:00:00.000Z`.
const NOW_SECONDS: i64 = 1_767_225_600;
const NOW: i64 = NOW_SECONDS * 1000;
const DAY: i64 = 24 * 60 * 60;

struct Key {
    pair: Ed25519KeyPair,
}

impl Key {
    fn new() -> Key {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        Key {
            pair: Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap(),
        }
    }

    /// `exportSPKI(publicKey)`.
    fn pem(&self) -> String {
        let mut der = vec![
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        ];
        der.extend_from_slice(self.pair.public_key().as_ref());
        format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            STANDARD.encode(der)
        )
    }

    fn sign(&self, header: &Value, claims: &Value) -> String {
        let head = URL_SAFE_NO_PAD.encode(header.to_string());
        let body = URL_SAFE_NO_PAD.encode(claims.to_string());
        let message = format!("{head}.{body}");
        let signature = self.pair.sign(message.as_bytes());
        format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref()))
    }
}

/// `signLicense({ kid, claims, iat, exp, nbf })`.
#[derive(Default)]
struct Sign {
    kid: Option<&'static str>,
    claims: Option<Value>,
    iat: Option<i64>,
    exp: Option<i64>,
    nbf: Option<i64>,
}

fn sign(key: &Key, opts: Sign) -> String {
    let mut claims = json!({
        "customerName": "Acme Corp",
        "tier": "enterprise",
        "features": ["sso", "customBranding"],
        "limits": { "maxWorkspaces": 7, "maxSeats": null, "maxChannels": 12 },
        "licenseId": "lic_test_123",
        "iss": LICENSE_ISSUER,
        "sub": "customer_123",
        "iat": opts.iat.unwrap_or(NOW_SECONDS - 60),
        "exp": opts.exp.unwrap_or(NOW_SECONDS + 31 * DAY),
    });
    if let Some(nbf) = opts.nbf {
        claims["nbf"] = json!(nbf);
    }
    if let Some(Value::Object(extra)) = opts.claims {
        for (k, v) in extra {
            claims[k] = v;
        }
    }
    key.sign(
        &json!({ "alg": "EdDSA", "kid": opts.kid.unwrap_or(KID), "typ": "JWT" }),
        &claims,
    )
}

fn verify(key: &Key, token: &str, tiers: Option<&[&str]>) -> LicenseStatus {
    let pem = key.pem();
    verify_license_token(token, &[(KID, pem.as_str())], tiers, NOW)
}

#[test]
fn returns_valid_status_for_an_ed25519_signed_license() {
    let key = Key::new();
    let status = verify(&key, &sign(&key, Sign::default()), None);
    assert_eq!(
        status,
        LicenseStatus {
            state: LicenseState::Valid,
            license_id: Some("lic_test_123".into()),
            customer_name: Some("Acme Corp".into()),
            tier: Some("enterprise".into()),
            features: vec!["sso".into(), "customBranding".into()],
            limits: Some(LicenseLimits {
                max_workspaces: Some(7),
                max_seats: None,
                max_channels: Some(12),
            }),
            issued_at: Some((NOW_SECONDS - 60) * 1000),
            expires_at: Some((NOW_SECONDS + 31 * DAY) * 1000),
            days_remaining: Some(31),
            grace_ends_at: Some((NOW_SECONDS + 38 * DAY) * 1000),
            key_id: Some(KID.into()),
            error: None,
        }
    );
    assert!(status.has_feature("sso"));
    assert!(!status.has_feature("customDomain"));
}

#[test]
fn surfaces_signed_claims_for_an_expired_license_and_the_grace_window() {
    let key = Key::new();
    let expired = verify(
        &key,
        &sign(
            &key,
            Sign {
                exp: Some(NOW_SECONDS - 9 * DAY),
                ..Sign::default()
            },
        ),
        None,
    );
    assert_eq!(expired.state, LicenseState::Expired);
    assert_eq!(expired.license_id.as_deref(), Some("lic_test_123"));
    assert_eq!(expired.days_remaining, Some(-9));
    assert!(expired.error.unwrap().contains("exp"));

    let grace = verify(
        &key,
        &sign(
            &key,
            Sign {
                exp: Some(NOW_SECONDS - 3 * DAY),
                ..Sign::default()
            },
        ),
        None,
    );
    assert_eq!(grace.state, LicenseState::Grace);
    assert_eq!(grace.days_remaining, Some(-3));
    assert_eq!(grace.grace_ends_at, Some((NOW_SECONDS + 4 * DAY) * 1000));
    assert!(grace.state.is_entitled());

    // "treats exactly 7 days past expiry as expired (boundary)".
    let boundary = verify(
        &key,
        &sign(
            &key,
            Sign {
                exp: Some(NOW_SECONDS - 7 * DAY),
                ..Sign::default()
            },
        ),
        None,
    );
    assert_eq!(boundary.state, LicenseState::Expired);
    assert_eq!(boundary.grace_ends_at, Some(NOW));
    assert_eq!(boundary.limits_if_entitled(), None);
}

#[test]
fn rejects_tampered_wrong_key_unknown_kid_and_non_eddsa_tokens() {
    let key = Key::new();
    let token = sign(&key, Sign::default());
    let parts: Vec<&str> = token.split('.').collect();
    let mut claims: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
    claims["licenseId"] = json!("lic_tampered");
    let tampered = format!(
        "{}.{}.{}",
        parts[0],
        URL_SAFE_NO_PAD.encode(claims.to_string()),
        parts[2]
    );
    let status = verify(&key, &tampered, None);
    assert_eq!(
        (status.state, status.license_id.clone()),
        (LicenseState::Invalid, None)
    );
    assert!(status.error.is_some());

    let other = Key::new();
    let status = verify(&key, &sign(&other, Sign::default()), None);
    assert_eq!(
        (status.state, status.key_id.as_deref()),
        (LicenseState::Invalid, Some(KID))
    );

    let status = verify(
        &key,
        &sign(
            &key,
            Sign {
                kid: Some("unknown-key"),
                ..Sign::default()
            },
        ),
        None,
    );
    assert_eq!(status.key_id.as_deref(), Some("unknown-key"));
    assert_eq!(
        status.error.as_deref(),
        Some("Unknown license key id: unknown-key")
    );

    let hs = key.sign(
        &json!({ "alg": "HS256", "kid": KID, "typ": "JWT" }),
        &json!({}),
    );
    let status = verify(&key, &hs, None);
    assert_eq!(status.state, LicenseState::Invalid);
    assert_eq!(
        status.error.as_deref(),
        Some("License token must use EdDSA")
    );
}

#[test]
fn rejects_an_invalid_payload_shape_and_a_license_not_active_yet() {
    let key = Key::new();
    let bad = sign(
        &key,
        Sign {
            claims: Some(
                json!({ "limits": { "maxWorkspaces": -1, "maxSeats": null, "maxChannels": 12 } }),
            ),
            ..Sign::default()
        },
    );
    let status = verify(&key, &bad, None);
    assert_eq!(status.state, LicenseState::Invalid);
    assert!(status.error.unwrap().contains("Too small"));

    let early = verify(
        &key,
        &sign(
            &key,
            Sign {
                nbf: Some(NOW_SECONDS + DAY),
                ..Sign::default()
            },
        ),
        None,
    );
    assert_eq!(
        (early.state, early.key_id.as_deref()),
        (LicenseState::Invalid, Some(KID))
    );
    assert!(early.error.unwrap().contains("nbf"));
}

#[test]
fn checks_the_tier_against_the_allowed_tiers() {
    let key = Key::new();
    let tier = |t: &str, exp: Option<i64>| {
        sign(
            &key,
            Sign {
                claims: Some(json!({ "tier": t })),
                exp,
                ..Sign::default()
            },
        )
    };
    let status = verify(&key, &tier("cloud", None), Some(&["enterprise"]));
    assert_eq!(status.state, LicenseState::Invalid);
    assert_eq!(
        status.error.as_deref(),
        Some("License tier \"cloud\" cannot be used with this edition")
    );
    assert_eq!(
        verify(&key, &tier("enterprise", None), Some(&["cloud"])).state,
        LicenseState::Invalid
    );
    assert_eq!(
        verify(&key, &tier("enterprise", None), Some(&["enterprise"])).state,
        LicenseState::Valid
    );
    assert_eq!(
        verify(&key, &tier("cloud", None), None).state,
        LicenseState::Valid
    );
    // Expired and the wrong tier: invalid, not expired.
    let lapsed = tier("cloud", Some(NOW_SECONDS - 2 * DAY));
    assert_eq!(
        verify(&key, &lapsed, Some(&["enterprise"])).state,
        LicenseState::Invalid
    );
    assert_eq!(
        verify(&key, &tier("enterprise-plus", None), None).state,
        LicenseState::Invalid
    );
}

async fn resolve(key: &Key, env: Option<&str>, stored: Option<String>) -> LicenseStatus {
    let db = common::fresh_db().await;
    if let Some(token) = stored {
        db.write(move |tx| {
            chatbotx_db::repo::license::upsert(tx, "lic_test_123", &token, NOW)
                .map_err(|e| chatbotx_db::DbError::other(e.message()))
        })
        .await
        .unwrap();
    }
    let pem = key.pem();
    resolve_license(&db, env, Edition::Community, &[(KID, pem.as_str())], NOW).await
}

#[tokio::test]
async fn resolves_license_key_with_its_feature_and_limit_accessors() {
    let key = Key::new();
    let env = sign(
        &key,
        Sign {
            exp: Some(NOW_SECONDS + 400 * DAY),
            ..Sign::default()
        },
    );
    let status = resolve(&key, Some(&env), None).await;
    assert_eq!(status.state, LicenseState::Valid);
    assert!(status.has_feature("customBranding"));
    let limits = status.limits_if_entitled().unwrap();
    assert_eq!((limits.max_workspaces, limits.max_seats), (Some(7), None));

    // "keeps enterprise entitlements while inside the grace window".
    let grace = sign(
        &key,
        Sign {
            exp: Some(NOW_SECONDS - 3 * DAY),
            ..Sign::default()
        },
    );
    let status = resolve(&key, Some(&grace), None).await;
    assert_eq!(status.limits_if_entitled().unwrap().max_workspaces, Some(7));
    // "drops entitlements once the grace window has elapsed".
    let lapsed = sign(
        &key,
        Sign {
            exp: Some(NOW_SECONDS - 9 * DAY),
            ..Sign::default()
        },
    );
    let status = resolve(&key, Some(&lapsed), None).await;
    assert!(!status.has_feature("sso"));
    assert_eq!(status.limits_if_entitled(), None);

    // "reports missing when LICENSE_KEY is unset".
    let status = resolve(&key, None, None).await;
    assert_eq!(status.state, LicenseState::Missing);
    assert_eq!(
        status.error.as_deref(),
        Some("LICENSE_KEY is not configured")
    );
}

#[tokio::test]
async fn picks_the_newer_of_license_key_and_the_stored_token() {
    let key = Key::new();
    // "prefers a valid stored token over an expired LICENSE_KEY".
    let env = sign(
        &key,
        Sign {
            exp: Some(NOW_SECONDS - 2 * DAY),
            iat: Some(NOW_SECONDS - 30),
            ..Sign::default()
        },
    );
    let stored = sign(
        &key,
        Sign {
            exp: Some(NOW_SECONDS + 31 * DAY),
            iat: Some(NOW_SECONDS - 60),
            ..Sign::default()
        },
    );
    let status = resolve(&key, Some(&env), Some(stored)).await;
    assert_eq!(
        (status.state, status.expires_at),
        (LicenseState::Valid, Some((NOW_SECONDS + 31 * DAY) * 1000))
    );

    // "ignores a stored token for a different license".
    let other = sign(
        &key,
        Sign {
            claims: Some(json!({ "licenseId": "lic_other" })),
            exp: Some(NOW_SECONDS + 31 * DAY),
            ..Sign::default()
        },
    );
    let env = sign(
        &key,
        Sign {
            exp: Some(NOW_SECONDS - 2 * DAY),
            ..Sign::default()
        },
    );
    assert_eq!(
        resolve(&key, Some(&env), Some(other)).await.state,
        LicenseState::Grace
    );

    // "prefers a grace license over a lapsed one for the same licenseId".
    let env = sign(
        &key,
        Sign {
            exp: Some(NOW_SECONDS - 9 * DAY),
            iat: Some(NOW_SECONDS - 60),
            ..Sign::default()
        },
    );
    let stored = sign(
        &key,
        Sign {
            exp: Some(NOW_SECONDS - 3 * DAY),
            iat: Some(NOW_SECONDS - 30),
            ..Sign::default()
        },
    );
    assert_eq!(
        resolve(&key, Some(&env), Some(stored)).await.state,
        LicenseState::Grace
    );
}
