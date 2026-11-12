//! HS256 realtime tokens (spec 2.1; `packages/realtime-protocol/src/auth.ts`).
//!
//! Same format as `jose` `SignJWT`: header `{"alg":"HS256"}`, payload =
//! the purpose claims, `purpose`, `iat`, `aud`, `exp` (in that key order),
//! base64url segments without padding. Verify: HS256 only, signature,
//! `exp`/`nbf` with 5 s clock tolerance, `aud` exact, `purpose` exact, then
//! the claim schema of the purpose.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chatbotx_kv::Clock;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::Sha256;

/// `REALTIME_TOKEN_TTL_SECONDS`.
pub const TOKEN_TTL_SECS: u64 = 60;
/// `CLOCK_TOLERANCE_SECONDS`.
pub const CLOCK_TOLERANCE_SECS: u64 = 5;
/// `REALTIME_MEMBER_REVOKED_TTL_SECONDS` = token TTL + tolerance.
pub const MEMBER_REVOKED_TTL_SECS: u64 = TOKEN_TTL_SECS + CLOCK_TOLERANCE_SECS;

/// `REALTIME_TOKEN_PURPOSE`.
pub mod purpose {
    pub const MEMBER_CONNECT: &str = "member-connect";
    pub const GUEST_CONNECT: &str = "guest-connect";
    /// Kept for the generic sign/verify cases (decision D3: no HTTP hop).
    pub const PRESENCE_REPORT: &str = "presence-report";
}

/// `RealtimeAudienceKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudienceKind {
    Workspace,
    Guest,
}

/// `aud = "<kind>:<id>"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Audience<'a> {
    pub kind: AudienceKind,
    pub id: &'a str,
}

impl<'a> Audience<'a> {
    pub const fn workspace(id: &'a str) -> Audience<'a> {
        Audience {
            kind: AudienceKind::Workspace,
            id,
        }
    }

    pub const fn guest(id: &'a str) -> Audience<'a> {
        Audience {
            kind: AudienceKind::Guest,
            id,
        }
    }
}

/// `realtimeChatScopes`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ChatScope {
    All,
    Assigned,
    None,
}

/// The input of `signMemberConnectToken`. `issued_at_ms` is read by the
/// mint before the membership lookup (spec 2.1); it must be ≥ 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberConnect {
    pub workspace_id: String,
    pub user_id: String,
    pub chat_scope: ChatScope,
    pub team_ids: Vec<String>,
    pub issued_at_ms: i64,
}

/// `RealtimeMemberClaims`.
#[derive(Clone, Debug, PartialEq)]
pub struct MemberClaims {
    pub user_id: String,
    pub chat_scope: ChatScope,
    pub team_ids: Vec<String>,
    /// JWT `iat` (seconds).
    pub iat: f64,
    /// Millisecond mint time; a revoke marker `> iat_ms` invalidates it.
    pub iat_ms: u64,
}

/// `RealtimeGuestClaims`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestClaims {
    pub guest_conversation_id: String,
    pub workspace_id: String,
}

/// The presence-report claims (`bodyHash`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresenceReportClaims {
    pub body_hash: String,
}

/// A token that cannot be signed or does not verify.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenError {
    pub reason: String,
}

impl TokenError {
    pub fn new(reason: impl Into<String>) -> TokenError {
        TokenError {
            reason: reason.into(),
        }
    }
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "realtime token: {}", self.reason)
    }
}

impl std::error::Error for TokenError {}

/// `signMemberConnectToken`: `iat = floor(issued_at_ms / 1000)`,
/// `exp = iat + 60`. Rejects a negative `issued_at_ms`.
pub fn sign_member_connect_token(
    member: &MemberConnect,
    secret: &str,
) -> Result<String, TokenError> {
    let issued_at_ms = u64::try_from(member.issued_at_ms)
        .map_err(|_| TokenError::new("issuedAtMs must be a non-negative integer"))?;
    let iat = issued_at_ms / 1000;
    let iat = i64::try_from(iat).map_err(|_| TokenError::new("issuedAtMs is too large"))?;
    Ok(sign_claims(
        &[
            ("userId", json!(member.user_id)),
            ("chatScope", json!(member.chat_scope)),
            ("teamIds", json!(member.team_ids)),
            ("iatMs", json!(issued_at_ms)),
        ],
        Audience::workspace(&member.workspace_id),
        purpose::MEMBER_CONNECT,
        secret,
        iat,
    ))
}

/// `verifyMemberConnectToken`.
pub fn verify_member_connect_token(
    token: &str,
    workspace_id: &str,
    secret: &str,
    clock: &dyn Clock,
) -> Result<MemberClaims, TokenError> {
    let payload = verify_realtime_token(
        token,
        Audience::workspace(workspace_id),
        purpose::MEMBER_CONNECT,
        secret,
        clock,
    )?;
    let user_id = non_empty_string(&payload, "userId")?;
    let chat_scope: ChatScope = payload
        .get("chatScope")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or_else(|| TokenError::new("chatScope: expected all, assigned or none"))?;
    let team_ids = match payload.get("teamIds") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::String(id) if !id.is_empty() => Ok(id.clone()),
                _ => Err(TokenError::new("teamIds: expected non-empty strings")),
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(TokenError::new("teamIds: expected an array")),
    };
    let iat = payload
        .get("iat")
        .and_then(Value::as_f64)
        .ok_or_else(|| TokenError::new("iat: expected a number"))?;
    let iat_ms = payload
        .get("iatMs")
        .and_then(non_negative_integer)
        .ok_or_else(|| TokenError::new("iatMs: expected a non-negative integer"))?;
    Ok(MemberClaims {
        user_id,
        chat_scope,
        team_ids,
        iat,
        iat_ms,
    })
}

/// `signGuestConnectToken` (`iat` = now).
pub fn sign_guest_connect_token(
    guest_conversation_id: &str,
    workspace_id: &str,
    secret: &str,
    clock: &dyn Clock,
) -> String {
    sign_claims(
        &[
            ("guestConversationId", json!(guest_conversation_id)),
            ("workspaceId", json!(workspace_id)),
        ],
        Audience::guest(guest_conversation_id),
        purpose::GUEST_CONNECT,
        secret,
        now_secs(clock),
    )
}

/// `verifyGuestConnectToken`.
pub fn verify_guest_connect_token(
    token: &str,
    guest_conversation_id: &str,
    secret: &str,
    clock: &dyn Clock,
) -> Result<GuestClaims, TokenError> {
    let payload = verify_realtime_token(
        token,
        Audience::guest(guest_conversation_id),
        purpose::GUEST_CONNECT,
        secret,
        clock,
    )?;
    Ok(GuestClaims {
        guest_conversation_id: non_empty_string(&payload, "guestConversationId")?,
        workspace_id: non_empty_string(&payload, "workspaceId")?,
    })
}

/// `signPresenceReportToken` (`iat` = now).
pub fn sign_presence_report_token(
    workspace_id: &str,
    body_hash: &str,
    secret: &str,
    clock: &dyn Clock,
) -> String {
    sign_claims(
        &[("bodyHash", json!(body_hash))],
        Audience::workspace(workspace_id),
        purpose::PRESENCE_REPORT,
        secret,
        now_secs(clock),
    )
}

/// `verifyPresenceReportToken`.
pub fn verify_presence_report_token(
    token: &str,
    workspace_id: &str,
    secret: &str,
    clock: &dyn Clock,
) -> Result<PresenceReportClaims, TokenError> {
    let payload = verify_realtime_token(
        token,
        Audience::workspace(workspace_id),
        purpose::PRESENCE_REPORT,
        secret,
        clock,
    )?;
    Ok(PresenceReportClaims {
        body_hash: non_empty_string(&payload, "bodyHash")?,
    })
}

/// `verifyRealtimeToken`: signature, time claims, audience and purpose;
/// returns the whole payload.
pub fn verify_realtime_token(
    token: &str,
    audience: Audience<'_>,
    purpose: &str,
    secret: &str,
    clock: &dyn Clock,
) -> Result<Map<String, Value>, TokenError> {
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(TokenError::new("expected three segments"));
    };
    let header: Map<String, Value> = decode_segment(header)?;
    if header.get("alg").and_then(Value::as_str) != Some(ALGORITHM) {
        return Err(TokenError::new("unexpected algorithm"));
    }
    if header.contains_key("crit")
        || header
            .get("b64")
            .is_some_and(|b64| b64 != &Value::Bool(true))
    {
        return Err(TokenError::new("unsupported header parameter"));
    }
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| TokenError::new("signature is not base64url"))?;
    let signing_input = token
        .rsplit_once('.')
        .map_or("", |(signing_input, _)| signing_input);
    mac(secret, signing_input)
        .verify_slice(&signature)
        .map_err(|_| TokenError::new("signature verification failed"))?;
    let claims: Map<String, Value> = decode_segment(payload)?;

    let now = now_secs(clock) as f64;
    let tolerance = CLOCK_TOLERANCE_SECS as f64;
    if let Some(iat) = claims.get("iat") {
        if !iat.is_number() {
            return Err(TokenError::new("\"iat\" claim must be a number"));
        }
    }
    if let Some(nbf) = claims.get("nbf") {
        let nbf = nbf
            .as_f64()
            .ok_or_else(|| TokenError::new("\"nbf\" claim must be a number"))?;
        if nbf > now + tolerance {
            return Err(TokenError::new("\"nbf\" claim timestamp check failed"));
        }
    }
    if let Some(exp) = claims.get("exp") {
        let exp = exp
            .as_f64()
            .ok_or_else(|| TokenError::new("\"exp\" claim must be a number"))?;
        if exp <= now - tolerance {
            return Err(TokenError::new("\"exp\" claim timestamp check failed"));
        }
    }
    let expected_audience = format_audience(audience);
    let audience_matches = match claims.get("aud") {
        Some(Value::String(aud)) => *aud == expected_audience,
        Some(Value::Array(auds)) => auds
            .iter()
            .any(|aud| aud.as_str() == Some(expected_audience.as_str())),
        _ => false,
    };
    if !audience_matches {
        return Err(TokenError::new("unexpected \"aud\" claim value"));
    }
    if claims.get("purpose").and_then(Value::as_str) != Some(purpose) {
        return Err(TokenError::new("Unexpected realtime token purpose"));
    }
    Ok(claims)
}

/// `extractBearerToken`: `"Bearer <t>"` -> `t`, else `None`.
pub fn extract_bearer_token(authorization_header: Option<&str>) -> Option<&str> {
    let header = authorization_header.filter(|header| !header.is_empty())?;
    let mut parts = header.split(' ');
    let scheme = parts.next();
    let token = parts.next().filter(|token| !token.is_empty());
    match (scheme, token) {
        (Some(BEARER_SCHEME), Some(token)) => Some(token),
        _ => None,
    }
}

// ---- signing helpers ----

const ALGORITHM: &str = "HS256";
const BEARER_SCHEME: &str = "Bearer";

type HmacSha256 = Hmac<Sha256>;

fn mac(secret: &str, signing_input: &str) -> HmacSha256 {
    // HMAC accepts a key of any length.
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(secret.as_bytes())
        .unwrap_or_else(|_| unreachable!("HMAC takes keys of any length"));
    mac.update(signing_input.as_bytes());
    mac
}

fn format_audience(audience: Audience<'_>) -> String {
    let kind = match audience.kind {
        AudienceKind::Workspace => "workspace",
        AudienceKind::Guest => "guest",
    };
    format!("{kind}:{}", audience.id)
}

/// `Math.floor(Date.now() / 1000)`.
fn now_secs(clock: &dyn Clock) -> i64 {
    clock.now_ms().div_euclid(1000)
}

/// `new SignJWT({...claims, purpose}).setIssuedAt(iat).setAudience(..)
/// .setExpirationTime(iat + 60)`: the payload keys in that order.
fn sign_claims(
    claims: &[(&str, Value)],
    audience: Audience<'_>,
    purpose: &str,
    secret: &str,
    iat: i64,
) -> String {
    let mut payload = String::from("{");
    let mut push = |key: &str, value: &Value| {
        if payload.len() > 1 {
            payload.push(',');
        }
        payload.push_str(&crate::protocol::to_json(key));
        payload.push(':');
        payload.push_str(&crate::protocol::to_json(value));
    };
    for (key, value) in claims {
        push(key, value);
    }
    push("purpose", &json!(purpose));
    push("iat", &json!(iat));
    push("aud", &json!(format_audience(audience)));
    push("exp", &json!(iat.saturating_add(TOKEN_TTL_SECS as i64)));
    payload.push('}');

    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256"}"#);
    let signing_input = format!("{header}.{}", URL_SAFE_NO_PAD.encode(payload));
    let signature = URL_SAFE_NO_PAD.encode(mac(secret, &signing_input).finalize().into_bytes());
    format!("{signing_input}.{signature}")
}

fn decode_segment(segment: &str) -> Result<Map<String, Value>, TokenError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(segment)
        .map_err(|_| TokenError::new("segment is not base64url"))?;
    serde_json::from_slice(&bytes).map_err(|_| TokenError::new("segment is not a JSON object"))
}

fn non_empty_string(claims: &Map<String, Value>, key: &str) -> Result<String, TokenError> {
    match claims.get(key) {
        Some(Value::String(value)) if !value.is_empty() => Ok(value.clone()),
        _ => Err(TokenError::new(format!(
            "{key}: expected a non-empty string"
        ))),
    }
}

/// `z.number().int().nonnegative()` as `u64`.
fn non_negative_integer(value: &Value) -> Option<u64> {
    if let Some(n) = value.as_u64() {
        return Some(n);
    }
    let n = value.as_f64()?;
    (n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n <= u64::MAX as f64).then_some(n as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chatbotx_kv::TestClock;

    const SECRET: &str = "compat-secret-compat-secret-compat-secret";
    const IAT_MS: i64 = 1_790_000_000_123;
    /// Minted by the original `jose` `SignJWT` (the TS `signMemberConnectToken` shape).
    const JOSE_MEMBER: &str = "eyJhbGciOiJIUzI1NiJ9.eyJ1c2VySWQiOiJ1c2VyLTEiLCJjaGF0U2NvcGUiOiJhc3NpZ25lZCIsInRlYW1JZHMiOlsidGVhbS0xIl0sImlhdE1zIjoxNzkwMDAwMDAwMTIzLCJwdXJwb3NlIjoibWVtYmVyLWNvbm5lY3QiLCJpYXQiOjE3OTAwMDAwMDAsImF1ZCI6IndvcmtzcGFjZTp3cy0xIiwiZXhwIjoxNzkwMDAwMDYwfQ.huh81xRv4ZBs4H-L4tLFUOUt0P12v8ipXCgK_4sXIOk";
    /// Minted by `jose` (the TS `signGuestConnectToken` shape).
    const JOSE_GUEST: &str = "eyJhbGciOiJIUzI1NiJ9.eyJndWVzdENvbnZlcnNhdGlvbklkIjoid3MtMTpndWVzdC0xIiwid29ya3NwYWNlSWQiOiJ3cy0xIiwicHVycG9zZSI6Imd1ZXN0LWNvbm5lY3QiLCJpYXQiOjE3OTAwMDAwMDAsImF1ZCI6Imd1ZXN0OndzLTE6Z3Vlc3QtMSIsImV4cCI6MTc5MDAwMDA2MH0.FIZxL52ztziJ3h8mKdKPq8e2EOqKULCELwmTnSjvLaE";

    #[test]
    fn verifies_tokens_minted_by_the_typescript_issuer() {
        let clock = TestClock::new(IAT_MS);
        let member = verify_member_connect_token(JOSE_MEMBER, "ws-1", SECRET, &clock).unwrap();
        assert_eq!(member.user_id, "user-1");
        assert_eq!(member.chat_scope, ChatScope::Assigned);
        assert_eq!(member.team_ids, vec!["team-1".to_owned()]);
        assert_eq!(member.iat_ms, IAT_MS as u64);
        let guest = verify_guest_connect_token(JOSE_GUEST, "ws-1:guest-1", SECRET, &clock).unwrap();
        assert_eq!(guest.workspace_id, "ws-1");
    }

    #[test]
    fn mints_the_same_bytes_as_the_typescript_issuer() {
        let member = sign_member_connect_token(
            &MemberConnect {
                workspace_id: "ws-1".to_owned(),
                user_id: "user-1".to_owned(),
                chat_scope: ChatScope::Assigned,
                team_ids: vec!["team-1".to_owned()],
                issued_at_ms: IAT_MS,
            },
            SECRET,
        )
        .unwrap();
        assert_eq!(member, JOSE_MEMBER);
        let clock = TestClock::new(IAT_MS);
        assert_eq!(
            sign_guest_connect_token("ws-1:guest-1", "ws-1", SECRET, &clock),
            JOSE_GUEST
        );
    }

    #[test]
    fn rejects_a_tampered_signature_header_or_algorithm() {
        let clock = TestClock::new(IAT_MS);
        let mut tampered = JOSE_MEMBER.to_owned();
        tampered.pop();
        tampered.push('A');
        assert!(verify_member_connect_token(&tampered, "ws-1", SECRET, &clock).is_err());
        let none_alg = format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#),
            JOSE_MEMBER.split_once('.').unwrap().1
        );
        assert!(verify_member_connect_token(&none_alg, "ws-1", SECRET, &clock).is_err());
        assert!(verify_member_connect_token("a.b", "ws-1", SECRET, &clock).is_err());
        // exp + 5 s tolerance: still valid at exp + 4 s, not at exp + 5 s.
        assert!(
            verify_member_connect_token(
                JOSE_MEMBER,
                "ws-1",
                SECRET,
                &TestClock::new(1_790_000_064_999)
            )
            .is_ok()
        );
        assert!(
            verify_member_connect_token(
                JOSE_MEMBER,
                "ws-1",
                SECRET,
                &TestClock::new(1_790_000_065_000)
            )
            .is_err()
        );
    }
}
