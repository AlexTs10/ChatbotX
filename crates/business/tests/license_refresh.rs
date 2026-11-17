//! `license.refresh.test.ts` and `license.startup.test.ts` (Q.D11 slice 10):
//! a one-shot local vendor instead of msw, a real `License` row instead of the
//! mocked repository, and `startup_check` instead of a `process.exit` spy.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chatbotx_business::keys::Edition;
use chatbotx_business::license::{
    LICENSE_ISSUER, LicenseState, RefreshConfig, RefreshOutcome, StartupCheck, refresh_license,
    resolve_license, startup_check, verify_license_token,
};
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

const KID: &str = "test-license-key";
const LICENSE_ID: &str = "lic_test_123";
const NOW_SECONDS: i64 = 1_767_225_600;
const NOW: i64 = NOW_SECONDS * 1000;
const DAY: i64 = 86_400;

struct Key {
    pair: Ed25519KeyPair,
    pem: String,
}

impl Key {
    fn new() -> Key {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let mut der = vec![
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        ];
        der.extend_from_slice(pair.public_key().as_ref());
        let pem = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            STANDARD.encode(der)
        );
        Key { pair, pem }
    }

    /// `signLicense({ issuedAt, expiresAt, licenseId, tier })`.
    fn sign(&self, iat: i64, exp: i64, license_id: &str, tier: &str) -> String {
        let claims = json!({
            "customerName": "Acme Corp", "tier": tier, "features": ["sso"],
            "limits": { "maxWorkspaces": null, "maxSeats": null, "maxChannels": null },
            "licenseId": license_id, "iss": LICENSE_ISSUER, "sub": "customer_123",
            "iat": iat, "exp": exp,
        });
        let head =
            URL_SAFE_NO_PAD.encode(json!({ "alg": "EdDSA", "kid": KID, "typ": "JWT" }).to_string());
        let message = format!("{head}.{}", URL_SAFE_NO_PAD.encode(claims.to_string()));
        let signature = URL_SAFE_NO_PAD.encode(self.pair.sign(message.as_bytes()).as_ref());
        format!("{message}.{signature}")
    }

    fn license(&self, iat: i64) -> String {
        self.sign(iat, NOW_SECONDS + 31 * DAY, LICENSE_ID, "enterprise")
    }
}

/// Answers one request with `status` and `body`; returns the request body.
fn vendor(status: u16, body: &'static str) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://{}/api/licenses/refresh",
        listener.local_addr().unwrap()
    );
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 8192];
        let text = loop {
            let n = stream.read(&mut buf).unwrap();
            request.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&request).to_string();
            if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if rest.len() >= length {
                    break rest.to_string();
                }
            }
        };
        let reply = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(reply.as_bytes()).unwrap();
        text
    });
    (url, handle)
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn config<'a>(token: &'a str, url: &'a str, keys: &'a [(&'a str, &'a str)]) -> RefreshConfig<'a> {
    RefreshConfig {
        edition: Edition::Enterprise,
        token: Some(token),
        enabled: true,
        url,
        keys,
    }
}

/// "persists a renewed token and makes it visible without a restart".
#[tokio::test]
async fn persists_a_renewed_token_and_resolves_it() {
    let db = common::fresh_db().await;
    let key = Key::new();
    let env = key.license(NOW_SECONDS - 60);
    let renewed = key.license(NOW_SECONDS);
    let body = leak(json!({ "token": renewed }).to_string());
    let (url, handle) = vendor(200, body);
    let keys = [(KID, key.pem.as_str())];
    let out = refresh_license(&db, config(&env, &url, &keys), NOW).await;
    let expires_at = (NOW_SECONDS + 31 * DAY) * 1000;
    assert_eq!(out, RefreshOutcome::Refreshed { expires_at });
    let request: Value = serde_json::from_str(&handle.join().unwrap()).unwrap();
    assert_eq!(
        request,
        json!({ "token": env, "licenseId": LICENSE_ID, "edition": "enterprise" })
    );
    let status = resolve_license(&db, Some(&env), Edition::Enterprise, &keys, NOW).await;
    assert_eq!(
        (status.state, status.issued_at),
        (LicenseState::Valid, Some(NOW))
    );
}

/// "returns unchanged when the vendor returns the same token" / "rejects a
/// token issued for another license".
#[tokio::test]
async fn the_same_token_is_unchanged_and_another_license_is_rejected() {
    let db = common::fresh_db().await;
    let key = Key::new();
    let env = key.license(NOW_SECONDS - 60);
    let keys = [(KID, key.pem.as_str())];
    let (url, _h) = vendor(200, leak(json!({ "token": env }).to_string()));
    let out = refresh_license(&db, config(&env, &url, &keys), NOW).await;
    assert_eq!(
        out,
        RefreshOutcome::Unchanged {
            expires_at: (NOW_SECONDS + 31 * DAY) * 1000
        }
    );

    let other = key.sign(
        NOW_SECONDS,
        NOW_SECONDS + 40 * DAY,
        "lic_other",
        "enterprise",
    );
    let (url, _h) = vendor(200, leak(json!({ "token": other }).to_string()));
    let out = refresh_license(&db, config(&env, &url, &keys), NOW).await;
    assert_eq!(out, RefreshOutcome::Failed("rejectedToken".into()));
    let stored = r#"SELECT COUNT(*) FROM "License" WHERE ?1 = 1"#;
    assert_eq!(common::scalar_i64(&db, stored, 1).await, Some(0));
}

/// "keeps the stored token when the vendor returns 402" / "returns badBody"
/// / "returns requestFailed when the vendor is unreachable".
#[tokio::test]
async fn vendor_failures_are_reported_and_store_nothing() {
    let db = common::fresh_db().await;
    let key = Key::new();
    let env = key.license(NOW_SECONDS - 60);
    let keys = [(KID, key.pem.as_str())];
    let (url, _h) = vendor(402, "");
    let out = refresh_license(&db, config(&env, &url, &keys), NOW).await;
    assert_eq!(out, RefreshOutcome::Failed("http_402".into()));
    let (url, _h) = vendor(200, "not json");
    let out = refresh_license(&db, config(&env, &url, &keys), NOW).await;
    assert_eq!(out, RefreshOutcome::Failed("badBody".into()));
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", closed.local_addr().unwrap());
    drop(closed);
    let out = refresh_license(&db, config(&env, &url, &keys), NOW).await;
    assert_eq!(out, RefreshOutcome::Failed("requestFailed".into()));
    let stored = r#"SELECT COUNT(*) FROM "License" WHERE ?1 = 1"#;
    assert_eq!(common::scalar_i64(&db, stored, 1).await, Some(0));
}

/// "skips the vendor request when refresh is disabled" / "on community
/// edition".
#[tokio::test]
async fn refresh_is_skipped_when_disabled_or_on_community() {
    let db = common::fresh_db().await;
    let key = Key::new();
    let env = key.license(NOW_SECONDS - 60);
    let keys = [(KID, key.pem.as_str())];
    let url = "http://127.0.0.1:9/";
    let off = RefreshConfig {
        enabled: false,
        ..config(&env, url, &keys)
    };
    assert_eq!(
        refresh_license(&db, off, NOW).await,
        RefreshOutcome::Skipped("disabled".into())
    );
    let community = RefreshConfig {
        edition: Edition::Community,
        ..config(&env, url, &keys)
    };
    assert_eq!(
        refresh_license(&db, community, NOW).await,
        RefreshOutcome::Skipped("community".into())
    );
}

#[test]
fn startup_refuses_a_missing_invalid_or_wrong_tier_license() {
    let key = Key::new();
    let keys = [(KID, key.pem.as_str())];
    let check = |edition: Edition, token: &str| {
        let tiers = chatbotx_business::license::allowed_tiers(edition);
        startup_check(edition, &verify_license_token(token, &keys, tiers, NOW))
    };
    let valid = key.license(NOW_SECONDS - 60);
    assert_eq!(check(Edition::Enterprise, &valid), StartupCheck::Verified);
    assert!(matches!(
        check(Edition::Enterprise, "not-a-token"),
        StartupCheck::Refuse {
            state: "invalid",
            ..
        }
    ));
    let other = Key::new();
    assert!(matches!(
        check(Edition::Enterprise, &other.license(NOW_SECONDS)),
        StartupCheck::Refuse { .. }
    ));
    let grace = key.sign(
        NOW_SECONDS - 10 * DAY,
        NOW_SECONDS - 3 * DAY,
        LICENSE_ID,
        "enterprise",
    );
    assert_eq!(check(Edition::Enterprise, &grace), StartupCheck::Grace);
    let lapsed = key.sign(
        NOW_SECONDS - 20 * DAY,
        NOW_SECONDS - 9 * DAY,
        LICENSE_ID,
        "enterprise",
    );
    assert_eq!(check(Edition::Enterprise, &lapsed), StartupCheck::Lapsed);
    assert_eq!(
        check(Edition::Community, "not-a-token"),
        StartupCheck::Skipped
    );
    let cloud = key.sign(NOW_SECONDS - 60, NOW_SECONDS + DAY, LICENSE_ID, "cloud");
    assert_eq!(check(Edition::Cloud, &cloud), StartupCheck::Verified);
    assert!(matches!(
        check(Edition::Enterprise, &cloud),
        StartupCheck::Refuse { .. }
    ));
    assert!(matches!(
        check(Edition::Cloud, &valid),
        StartupCheck::Refuse { .. }
    ));
}

/// "exits 1 when enterprise / cloud edition has no LICENSE_KEY".
#[tokio::test]
async fn startup_refuses_a_missing_license_key() {
    let db = common::fresh_db().await;
    for edition in [Edition::Enterprise, Edition::Cloud] {
        let status = resolve_license(&db, None, edition, &[], NOW).await;
        assert!(matches!(
            startup_check(edition, &status),
            StartupCheck::Refuse {
                state: "missing",
                ..
            }
        ));
    }
}
