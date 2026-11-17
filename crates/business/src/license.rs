//! `enterprise/license/{service,schema,public-keys}.ts` and
//! `user/entitlements.ts` (Q.D11 slice 10): the Ed25519 license JWT, the
//! stored renewed token and the enterprise feature gate.
//!
//! The JWT checks follow `jose.jwtVerify` (alg `EdDSA`, `typ: JWT`, issuer,
//! required `sub` / `iat` / `exp`, `nbf`, `exp` with no clock tolerance) and
//! keep its error messages. ponytail: no 60 s status cache; every call reads
//! `LICENSE_KEY` and the stored token (one indexed row, one signature check).

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chatbotx_db::Db;
use ring::signature::{ED25519, UnparsedPublicKey};
use serde_json::Value;

use crate::BusinessError;
use crate::keys::Edition;

/// `LICENSE_ISSUER`.
pub const LICENSE_ISSUER: &str = "https://chatbotx.io/licenses";

/// `LICENSE_PUBLIC_KEYS`: `(kid, SPKI PEM)`.
pub const LICENSE_PUBLIC_KEYS: &[(&str, &str)] = &[
    (
        "chatbotx-license-ed25519-2026-08",
        "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAo+li4bBUyYNnGoOrXm2NLHrUP14S4MTH3lw4Z6C8pl4=\n-----END PUBLIC KEY-----",
    ),
    (
        "chatbotx-license-ed25519-dev",
        "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAO5V5i1yZkhjX6pdurY1+S1WPOb7lGVYzxndIfsBSkvk=\n-----END PUBLIC KEY-----",
    ),
];

/// `LICENSE_FEATURES`.
pub const LICENSE_FEATURES: &[&str] = &[
    "sso",
    "customBranding",
    "customDomain",
    "auditLog",
    "prioritySupport",
];

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// `LICENSE_GRACE_PERIOD_DAYS`.
pub const LICENSE_GRACE_PERIOD_DAYS: i64 = 7;

/// `LicenseState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicenseState {
    Valid,
    Grace,
    Expired,
    Invalid,
    Missing,
}

impl LicenseState {
    pub fn as_str(self) -> &'static str {
        match self {
            LicenseState::Valid => "valid",
            LicenseState::Grace => "grace",
            LicenseState::Expired => "expired",
            LicenseState::Invalid => "invalid",
            LicenseState::Missing => "missing",
        }
    }

    /// `isLicenseEntitled`.
    pub fn is_entitled(self) -> bool {
        matches!(self, LicenseState::Valid | LicenseState::Grace)
    }

    fn rank(self) -> u8 {
        match self {
            LicenseState::Valid => 3,
            LicenseState::Grace => 2,
            LicenseState::Expired => 1,
            LicenseState::Invalid | LicenseState::Missing => 0,
        }
    }
}

/// `LicenseLimits`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LicenseLimits {
    pub max_workspaces: Option<i64>,
    pub max_seats: Option<i64>,
    pub max_channels: Option<i64>,
}

/// `LicenseStatus` (dates as unix ms).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LicenseStatus {
    pub state: LicenseState,
    pub license_id: Option<String>,
    pub customer_name: Option<String>,
    pub tier: Option<String>,
    pub features: Vec<String>,
    pub limits: Option<LicenseLimits>,
    pub issued_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub days_remaining: Option<i64>,
    pub grace_ends_at: Option<i64>,
    pub key_id: Option<String>,
    pub error: Option<String>,
}

impl LicenseStatus {
    fn empty(state: LicenseState, key_id: Option<String>, error: String) -> LicenseStatus {
        LicenseStatus {
            state,
            license_id: None,
            customer_name: None,
            tier: None,
            features: Vec::new(),
            limits: None,
            issued_at: None,
            expires_at: None,
            days_remaining: None,
            grace_ends_at: None,
            key_id,
            error: Some(error),
        }
    }

    /// `hasFeature(feature)`.
    pub fn has_feature(&self, feature: &str) -> bool {
        self.state.is_entitled() && self.features.iter().any(|f| f == feature)
    }

    /// `getLimit(key)`: `None` = not entitled; `Some(None)` = no limit.
    pub fn limits_if_entitled(&self) -> Option<LicenseLimits> {
        self.state
            .is_entitled()
            .then(|| self.limits.unwrap_or_default())
    }
}

fn missing() -> LicenseStatus {
    LicenseStatus::empty(
        LicenseState::Missing,
        None,
        "LICENSE_KEY is not configured".into(),
    )
}

fn invalid(error: impl Into<String>, key_id: Option<String>) -> LicenseStatus {
    LicenseStatus::empty(LicenseState::Invalid, key_id, error.into())
}

/// The 32-byte key of an Ed25519 SPKI PEM (`importSPKI`).
fn spki_ed25519(pem: &str) -> Option<Vec<u8>> {
    const PREFIX: [u8; 12] = [
        0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
    ];
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .concat();
    let der = STANDARD.decode(body.trim()).ok()?;
    (der.len() == 44 && der[..12] == PREFIX).then(|| der[12..].to_vec())
}

fn int_claim(claims: &Value, name: &str) -> Option<i64> {
    claims
        .get(name)
        .and_then(Value::as_f64)
        .filter(|n| n.fract() == 0.0)
        .map(|n| n as i64)
}

/// `licensePayloadSchema.parse(payload)`: the zod-style issue messages.
fn parse_payload(claims: &Value) -> Result<Payload, String> {
    let mut issues = Vec::new();
    let string = |issues: &mut Vec<String>, name: &str| -> Option<String> {
        match claims.get(name) {
            Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
            Some(Value::String(_)) => {
                issues.push(format!(
                    "{name}: Too small: expected string to have >=1 characters"
                ));
                None
            }
            _ => {
                issues.push(format!("{name}: Invalid input: expected string"));
                None
            }
        }
    };
    let positive =
        |issues: &mut Vec<String>, path: &str, value: Option<&Value>, nullable: bool| match value {
            Some(Value::Null) if nullable => Some(None),
            Some(v) if v.as_f64().is_some_and(|n| n.fract() == 0.0) => {
                let n = v.as_f64().unwrap_or_default() as i64;
                if n > 0 {
                    Some(Some(n))
                } else {
                    issues.push(format!("{path}: Too small: expected number to be >0"));
                    None
                }
            }
            _ => {
                issues.push(format!("{path}: Invalid input: expected int"));
                None
            }
        };
    let sub = string(&mut issues, "sub");
    let iss = string(&mut issues, "iss");
    let iat = positive(&mut issues, "iat", claims.get("iat"), false);
    let exp = positive(&mut issues, "exp", claims.get("exp"), false);
    if claims.get("nbf").is_some() {
        positive(&mut issues, "nbf", claims.get("nbf"), false);
    }
    let customer_name = string(&mut issues, "customerName");
    let tier = match claims.get("tier").and_then(Value::as_str) {
        Some(t @ ("enterprise" | "cloud")) => Some(t.to_owned()),
        _ => {
            issues
                .push("tier: Invalid option: expected one of \"enterprise\"|\"cloud\"".to_owned());
            None
        }
    };
    let features = match claims.get("features") {
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for (i, item) in items.iter().enumerate() {
                match item.as_str().filter(|f| LICENSE_FEATURES.contains(f)) {
                    Some(f) => out.push(f.to_owned()),
                    None => issues.push(format!("features.{i}: Invalid option")),
                }
            }
            Some(out)
        }
        _ => {
            issues.push("features: Invalid input: expected array".to_owned());
            None
        }
    };
    let limits = match claims.get("limits") {
        Some(limits @ Value::Object(_)) => {
            let get = |issues: &mut Vec<String>, name: &str| {
                positive(issues, &format!("limits.{name}"), limits.get(name), true)
            };
            let w = get(&mut issues, "maxWorkspaces");
            let s = get(&mut issues, "maxSeats");
            let c = get(&mut issues, "maxChannels");
            Some((w, s, c))
        }
        _ => {
            issues.push("limits: Invalid input: expected object".to_owned());
            None
        }
    };
    let license_id = string(&mut issues, "licenseId");
    if !issues.is_empty() {
        return Err(issues.join("; "));
    }
    let (w, s, c) = limits.unwrap_or_default();
    Ok(Payload {
        _sub: sub.unwrap_or_default(),
        _iss: iss.unwrap_or_default(),
        iat: iat.flatten().unwrap_or_default(),
        exp: exp.flatten().unwrap_or_default(),
        customer_name: customer_name.unwrap_or_default(),
        tier: tier.unwrap_or_default(),
        features: features.unwrap_or_default(),
        limits: LicenseLimits {
            max_workspaces: w.flatten(),
            max_seats: s.flatten(),
            max_channels: c.flatten(),
        },
        license_id: license_id.unwrap_or_default(),
    })
}

struct Payload {
    _sub: String,
    _iss: String,
    iat: i64,
    exp: i64,
    customer_name: String,
    tier: String,
    features: Vec<String>,
    limits: LicenseLimits,
    license_id: String,
}

/// `statusFromPayload`.
fn status_from_payload(
    expired: bool,
    claims: &Value,
    key_id: String,
    now_ms: i64,
    allowed_tiers: Option<&[&str]>,
    error: Option<String>,
) -> LicenseStatus {
    let payload = match parse_payload(claims) {
        Ok(p) => p,
        Err(message) => return invalid(message, Some(key_id)),
    };
    if let Some(tiers) = allowed_tiers
        && !tiers.contains(&payload.tier.as_str())
    {
        return invalid(
            format!(
                "License tier \"{}\" cannot be used with this edition",
                payload.tier
            ),
            Some(key_id),
        );
    }
    let grace_ends_at = payload.exp * 1000 + LICENSE_GRACE_PERIOD_DAYS * DAY_MS;
    let state = match (expired, now_ms < grace_ends_at) {
        (false, _) => LicenseState::Valid,
        (true, true) => LicenseState::Grace,
        (true, false) => LicenseState::Expired,
    };
    let remaining = payload.exp * 1000 - now_ms;
    LicenseStatus {
        state,
        license_id: Some(payload.license_id),
        customer_name: Some(payload.customer_name),
        tier: Some(payload.tier),
        features: payload.features,
        limits: Some(payload.limits),
        issued_at: Some(payload.iat * 1000),
        expires_at: Some(payload.exp * 1000),
        // `Math.ceil(ms / DAY)`.
        days_remaining: Some(
            remaining.div_euclid(DAY_MS) + i64::from(remaining.rem_euclid(DAY_MS) != 0),
        ),
        grace_ends_at: Some(grace_ends_at),
        key_id: Some(key_id),
        error,
    }
}

fn decode_json(part: &str) -> Option<Value> {
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).ok()?).ok()
}

/// `verifyLicenseToken(token, keyMap, { allowedTiers, currentDate })`.
pub fn verify_license_token(
    token: &str,
    keys: &[(&str, &str)],
    allowed_tiers: Option<&[&str]>,
    now_ms: i64,
) -> LicenseStatus {
    let parts: Vec<&str> = token.split('.').collect();
    let Some(header) = parts
        .first()
        .and_then(|h| decode_json(h))
        .filter(Value::is_object)
    else {
        return invalid("Invalid Token or Protected Header formatting", None);
    };
    let key_id = header.get("kid").and_then(Value::as_str).map(str::to_owned);
    if header.get("alg").and_then(Value::as_str) != Some("EdDSA") {
        return invalid("License token must use EdDSA", key_id);
    }
    let Some(kid) = key_id else {
        return invalid("License token is missing a key id", None);
    };
    let Some((_, pem)) = keys.iter().find(|(k, _)| *k == kid) else {
        return invalid(format!("Unknown license key id: {kid}"), Some(kid));
    };
    let Some(public_key) = spki_ed25519(pem) else {
        return invalid("Invalid SPKI key", Some(kid));
    };
    if parts.len() != 3 {
        return invalid("Invalid Compact JWS", Some(kid));
    }
    let Ok(signature) = URL_SAFE_NO_PAD.decode(parts[2]) else {
        return invalid("Invalid Compact JWS", Some(kid));
    };
    let message = format!("{}.{}", parts[0], parts[1]);
    if UnparsedPublicKey::new(&ED25519, &public_key)
        .verify(message.as_bytes(), &signature)
        .is_err()
    {
        return invalid("signature verification failed", Some(kid));
    }
    let Some(claims) = decode_json(parts[1]).filter(Value::is_object) else {
        return invalid("JWT Claims Set must be a top-level JSON object", Some(kid));
    };
    let typ = header.get("typ").and_then(Value::as_str);
    if !typ
        .is_some_and(|t| t.eq_ignore_ascii_case("jwt") || t.eq_ignore_ascii_case("application/jwt"))
    {
        return invalid("unexpected \"typ\" JWT header value", Some(kid));
    }
    for claim in ["iss", "sub", "iat", "exp"] {
        if claims.get(claim).is_none() {
            return invalid(format!("missing required \"{claim}\" claim"), Some(kid));
        }
    }
    if claims.get("iss").and_then(Value::as_str) != Some(LICENSE_ISSUER) {
        return invalid("unexpected \"iss\" claim value", Some(kid));
    }
    let now = now_ms.div_euclid(1000);
    for name in ["iat", "nbf", "exp"] {
        if claims.get(name).is_some() && !claims[name].is_number() {
            return invalid(format!("\"{name}\" claim must be a number"), Some(kid));
        }
    }
    if let Some(nbf) = int_claim(&claims, "nbf")
        && nbf > now
    {
        return invalid("\"nbf\" claim timestamp check failed", Some(kid));
    }
    let expired = int_claim(&claims, "exp").is_some_and(|exp| exp <= now);
    let error = expired.then(|| "\"exp\" claim timestamp check failed".to_owned());
    status_from_payload(expired, &claims, kid, now_ms, allowed_tiers, error)
}

/// `resolveAllowedTiers()`.
pub fn allowed_tiers(edition: Edition) -> Option<&'static [&'static str]> {
    match edition {
        Edition::Cloud => Some(&["cloud"]),
        Edition::Enterprise => Some(&["enterprise"]),
        Edition::Community => None,
    }
}

/// `isNewerLicense(candidate, current)`.
pub fn is_newer_license(candidate: &LicenseStatus, current: &LicenseStatus) -> bool {
    let (a, b) = (candidate.state.rank(), current.state.rank());
    if a != b {
        return a > b;
    }
    // `Date.parse(null ?? "")` is NaN: every comparison with it is false.
    let gt = |x: Option<i64>, y: Option<i64>| matches!((x, y), (Some(x), Some(y)) if x > y);
    let ne = |x: Option<i64>, y: Option<i64>| match (x, y) {
        (Some(x), Some(y)) => x != y,
        _ => true,
    };
    if ne(candidate.issued_at, current.issued_at) {
        return gt(candidate.issued_at, current.issued_at);
    }
    gt(candidate.expires_at, current.expires_at)
}

/// `resolveLicense()`: `LICENSE_KEY` (`env_token`), or the stored renewal of
/// the same license when it is newer.
pub async fn resolve_license(
    db: &Db,
    env_token: Option<&str>,
    edition: Edition,
    keys: &[(&str, &str)],
    now_ms: i64,
) -> LicenseStatus {
    let Some(env_token) = env_token.map(str::trim).filter(|t| !t.is_empty()) else {
        return missing();
    };
    let tiers = allowed_tiers(edition);
    let env = verify_license_token(env_token, keys, tiers, now_ms);
    let Some(license_id) = env.license_id.clone() else {
        return env;
    };
    let stored = db
        .read(move |c| {
            chatbotx_db::repo::license::find_token_by_license_id(c, &license_id)
                .map_err(|e| chatbotx_db::DbError::other(e.message()))
        })
        .await;
    let stored = match stored {
        Ok(Some(token)) => token,
        Ok(None) => return env,
        Err(err) => {
            tracing::warn!(%err, "license: could not read the stored license token");
            return env;
        }
    };
    let candidate = verify_license_token(&stored, keys, tiers, now_ms);
    if candidate.state == LicenseState::Invalid || candidate.license_id != env.license_id {
        tracing::warn!(
            err = candidate.error.as_deref().unwrap_or(""),
            state = candidate.state.as_str(),
            "license: ignoring an unusable stored license token"
        );
        return env;
    }
    if is_newer_license(&candidate, &env) {
        candidate
    } else {
        env
    }
}

/// `getLicenseStatus()` of the process: `LICENSE_KEY` and the built-in keys.
pub async fn license_status(db: &Db, edition: Edition) -> LicenseStatus {
    let token = std::env::var("LICENSE_KEY").ok();
    resolve_license(
        db,
        token.as_deref(),
        edition,
        LICENSE_PUBLIC_KEYS,
        chatbotx_core::js::now_ms(),
    )
    .await
}

/// `hasEnterpriseFeatures()`: cloud or enterprise with an entitled license.
pub async fn has_enterprise_features(db: &Db, edition: Edition) -> bool {
    !edition.is_community() && license_status(db, edition).await.state.is_entitled()
}

/// `enterpriseFeatureRequiredException()`.
pub fn enterprise_feature_required() -> BusinessError {
    BusinessError::exception(
        "This feature requires an enterprise license",
        "enterpriseFeatureRequired",
        403,
    )
}

/// `assertEnterpriseFeatures()`.
pub async fn assert_enterprise_features(db: &Db, edition: Edition) -> crate::Result<()> {
    if has_enterprise_features(db, edition).await {
        Ok(())
    } else {
        Err(enterprise_feature_required())
    }
}
