//! Public API `Idempotency-Key` (`apps/builder/src/lib/idempotency/*`,
//! `apps/builder/src/middlewares/idempotency.ts`). The records live in a
//! [`CasStore`] (production: `DurableKv`, kv K11) under
//! `api-idempotency:<credentialId>:<procedurePath>:<key>`.

use std::future::Future;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use chatbotx_kv::{CasStore, KvError};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::orpc::OrpcError;

/// `IDEMPOTENCY_KEY_HEADER`.
pub const IDEMPOTENCY_KEY_HEADER: &str = "Idempotency-Key";
/// `IDEMPOTENT_REPLAYED_HEADER`.
pub const IDEMPOTENT_REPLAYED_HEADER: &str = "Idempotent-Replayed";
/// `MAX_IDEMPOTENCY_KEY_LENGTH` (UTF-16 code units, like JS `length`).
pub const MAX_IDEMPOTENCY_KEY_LENGTH: usize = 255;
/// `IDEMPOTENCY_RETENTION_HOURS`.
pub const IDEMPOTENCY_RETENTION_HOURS: u64 = 24;
/// An in-flight claim expires after 5 minutes.
pub const IN_FLIGHT_TTL_MS: u64 = 5 * 60 * 1000;
/// A completed record is kept 24 hours.
pub const COMPLETED_TTL_MS: u64 = IDEMPOTENCY_RETENTION_HOURS * 60 * 60 * 1000;
/// A larger encoded output is not stored (the key is released).
pub const MAX_STORED_OUTPUT_BYTES: usize = 256 * 1024;

/// `IDEMPOTENCY_EXEMPT_READ_PATHS`: POST routes that only read.
pub const IDEMPOTENCY_EXEMPT_READ_PATHS: &[&str] = &[
    // `AI_FILES_SEARCH_TOKEN_PATH`
    "/v1/ai-files/search",
    // `BROADCAST_AUDIENCE_PREVIEW_TOKEN_PATH`
    "/v1/broadcasts/audience/preview",
    "/v1/broadcasts/preview-recurrence",
    "/v1/inbound-webhooks/preview-mapping",
    "/v1/templates/dependencies-preview",
    "/v1/templates/locked-resources",
];

/// `BROADCAST_AUDIENCE_PREVIEW_TOKEN_PATH`.
pub const BROADCAST_AUDIENCE_PREVIEW_TOKEN_PATH: &str = "/v1/broadcasts/audience/preview";

/// `IDEMPOTENCY_EXEMPT_READ_PATHS.has(path)`.
pub fn is_exempt_read_path(path: &str) -> bool {
    IDEMPOTENCY_EXEMPT_READ_PATHS.contains(&path)
}

/// One idempotency scope (`IdempotencyScope`).
#[derive(Debug, Clone, Copy)]
pub struct Scope<'a> {
    pub credential_id: &'a str,
    pub procedure_path: &'a str,
    pub idempotency_key: &'a str,
}

/// `ClaimResult`. A replay also carries the HTTP status of the first answer
/// (`None` for a record written without one).
#[derive(Debug, Clone, PartialEq)]
pub enum Claim {
    Claimed {
        claim_id: String,
    },
    Replay {
        output: Option<Value>,
        status: Option<u16>,
    },
    InFlight,
    FingerprintMismatch,
    /// Store unavailable, or the claim raced an expiry: run unprotected.
    Unprotected,
}

/// `fingerprintInput`: SHA-256 hex of the canonical JSON (sorted keys).
pub fn fingerprint_input(input: &Value) -> String {
    let canonical = serde_json::to_string(&canonicalize(input)).unwrap_or_else(|_| "null".into());
    hex(&Sha256::digest(canonical.as_bytes()))
}

/// `isValidIdempotencyKey`: 1..=255 UTF-16 code units.
pub fn is_valid_idempotency_key(value: &str) -> bool {
    let length = value.encode_utf16().count();
    (1..=MAX_IDEMPOTENCY_KEY_LENGTH).contains(&length)
}

/// `redactOneTimeSecret`: an object output with a non-null `secret` gets
/// `secret: null`.
pub fn redact_one_time_secret(output: Value) -> Value {
    match output {
        Value::Object(mut map) if map.get("secret").is_some_and(|secret| !secret.is_null()) => {
            map.insert("secret".to_string(), Value::Null);
            Value::Object(map)
        }
        other => other,
    }
}

/// `buildStoreKey`.
pub fn store_key(scope: &Scope<'_>) -> String {
    [
        "api-idempotency",
        scope.credential_id,
        scope.procedure_path,
        scope.idempotency_key,
    ]
    .join(":")
}

/// `claimIdempotencyKey`.
pub async fn claim_idempotency_key(
    store: &dyn CasStore,
    scope: &Scope<'_>,
    fingerprint: &str,
) -> Claim {
    let key = store_key(scope);
    let claim_id = new_claim_id();
    let record = json!({
        "state": "inFlight",
        "claimId": claim_id,
        "fingerprint": fingerprint,
    });
    match store.set_if_absent(&key, &record, IN_FLIGHT_TTL_MS).await {
        Ok(true) => return Claim::Claimed { claim_id },
        Ok(false) => {}
        Err(error) => return store_unavailable(&error),
    }
    let existing = match store.get_json(&key).await {
        Ok(Some(existing)) => existing,
        Ok(None) => return Claim::Unprotected,
        Err(error) => return store_unavailable(&error),
    };
    if existing.get("fingerprint").and_then(Value::as_str) != Some(fingerprint) {
        return Claim::FingerprintMismatch;
    }
    if existing.get("state").and_then(Value::as_str) == Some("inFlight") {
        return Claim::InFlight;
    }
    let output = existing
        .get("output")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .and_then(|text| serde_json::from_str(text).ok());
    let status = existing
        .get("status")
        .and_then(Value::as_u64)
        .and_then(|status| u16::try_from(status).ok());
    Claim::Replay { output, status }
}

/// `completeIdempotencyKey`; `status` is the HTTP status of the answer.
pub async fn complete_idempotency_key(
    store: &dyn CasStore,
    scope: &Scope<'_>,
    fingerprint: &str,
    claim_id: &str,
    output: Option<&Value>,
    status: Option<u16>,
) {
    let key = store_key(scope);
    let mut record = json!({
        "state": "completed",
        "claimId": claim_id,
        "fingerprint": fingerprint,
    });
    if let Some(output) = output {
        let encoded = redact_one_time_secret(output.clone()).to_string();
        if encoded.len() > MAX_STORED_OUTPUT_BYTES {
            tracing::warn!("Idempotency output exceeded storage limit");
            release_idempotency_key(store, scope, claim_id).await;
            return;
        }
        record["output"] = Value::String(encoded);
    }
    if let Some(status) = status {
        record["status"] = Value::from(status);
    }
    let expected = in_flight(claim_id);
    match store
        .compare_and_swap(&key, Some(&expected), &record, COMPLETED_TTL_MS)
        .await
    {
        Ok(true) => {}
        Ok(false) => tracing::warn!("Idempotency claim expired before completion"),
        Err(error) => {
            store_unavailable(&error);
        }
    }
}

/// `releaseIdempotencyKey`.
pub async fn release_idempotency_key(store: &dyn CasStore, scope: &Scope<'_>, claim_id: &str) {
    let key = store_key(scope);
    if let Err(error) = store.compare_and_delete(&key, &in_flight(claim_id)).await {
        store_unavailable(&error);
    }
}

/// What `apiIdempotencyMiddleware` reads of one call.
#[derive(Debug, Clone, Copy)]
pub struct MiddlewareCall<'a> {
    pub headers: &'a HeaderMap,
    /// `procedure["~orpc"].route.method`.
    pub method: &'a str,
    /// `procedure["~orpc"].route.path`.
    pub route_path: Option<&'a str>,
    /// `path.join(".")` (the route id).
    pub procedure_path: &'a str,
    /// `context.apiCredentialId`.
    pub credential_id: &'a str,
    /// The raw procedure input.
    pub input: &'a Value,
}

/// `apiIdempotencyMiddleware` around `next` (the handler). A handler answer
/// that is not 2xx counts as a thrown error (the claim is released). `None`
/// from `next` = the route is not handled (released, passed through).
pub async fn api_idempotency_middleware<F, Fut>(
    store: &dyn CasStore,
    call: MiddlewareCall<'_>,
    next: F,
) -> Option<Response>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Option<Response>>,
{
    let Some(raw_key) = call.headers.get(IDEMPOTENCY_KEY_HEADER) else {
        return next().await;
    };
    let method = call.method.to_ascii_uppercase();
    if method == "GET" || method == "HEAD" {
        return next().await;
    }
    if call.route_path.is_some_and(is_exempt_read_path) {
        return next().await;
    }
    let raw_key = String::from_utf8_lossy(raw_key.as_bytes());
    let idempotency_key = raw_key.trim();
    if !is_valid_idempotency_key(idempotency_key) {
        return Some(idempotency_error("idempotencyKeyInvalid"));
    }
    let scope = Scope {
        credential_id: call.credential_id,
        procedure_path: call.procedure_path,
        idempotency_key,
    };
    let fingerprint = fingerprint_input(call.input);
    let claim_id = match claim_idempotency_key(store, &scope, &fingerprint).await {
        Claim::Claimed { claim_id } => claim_id,
        Claim::Replay { output, status } => return Some(replay(output, status)),
        Claim::Unprotected => return next().await,
        Claim::FingerprintMismatch => return Some(idempotency_error("idempotencyKeyReused")),
        Claim::InFlight => return Some(idempotency_error("idempotencyKeyConflict")),
    };
    let Some(response) = next().await else {
        release_idempotency_key(store, &scope, &claim_id).await;
        return None;
    };
    if !response.status().is_success() {
        release_idempotency_key(store, &scope, &claim_id).await;
        return Some(response);
    }
    let (parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(error) => {
            release_idempotency_key(store, &scope, &claim_id).await;
            return Some(crate::handlers::internal_error(&error.to_string()).into_response());
        }
    };
    let output = if bytes.is_empty() {
        None
    } else if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
        Some(value)
    } else {
        // Not a JSON answer: it cannot be replayed as one.
        tracing::warn!("Idempotency output could not be stored");
        release_idempotency_key(store, &scope, &claim_id).await;
        return Some(Response::from_parts(parts, Body::from(bytes)));
    };
    complete_idempotency_key(
        store,
        &scope,
        &fingerprint,
        &claim_id,
        output.as_ref(),
        Some(parts.status.as_u16()),
    )
    .await;
    Some(Response::from_parts(parts, Body::from(bytes)))
}

/// The procedure input of a non-GET public call (oRPC compact input): the
/// path params merged with a JSON object body; a non-object JSON body alone;
/// a body that is not JSON as `__raw__:<sha256>`.
pub fn procedure_input(params: &[(&'static str, String)], body: &[u8]) -> Value {
    let body_value =
        if body.is_empty() {
            None
        } else {
            Some(serde_json::from_slice::<Value>(body).unwrap_or_else(|_| {
                Value::String(format!("__raw__:{}", hex(&Sha256::digest(body))))
            }))
        };
    if params.is_empty() {
        return body_value.unwrap_or(Value::Null);
    }
    let mut map = match body_value {
        Some(Value::Object(map)) => map,
        Some(other) => {
            let mut map = Map::new();
            map.insert("__body__".to_string(), other);
            map
        }
        None => Map::new(),
    };
    for (name, value) in params {
        map.insert((*name).to_string(), Value::String(value.clone()));
    }
    Value::Object(map)
}

/// `canonicalize`: object keys sorted (by UTF-16 code units, like JS `<`).
fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.encode_utf16().cmp(right.encode_utf16()));
            let mut out = Map::new();
            for (key, entry) in entries {
                out.insert(key.clone(), canonicalize(entry));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// `crypto.randomUUID()` (version 4).
fn new_claim_id() -> String {
    let mut bytes = [0_u8; 16];
    if SystemRandom::new().fill(&mut bytes).is_err() {
        // No OS randomness: a time-based id still separates claims.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        bytes = nanos.to_le_bytes();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The CAS expectation of an own in-flight claim.
fn in_flight(claim_id: &str) -> Map<String, Value> {
    let mut expected = Map::new();
    expected.insert("state".to_string(), Value::from("inFlight"));
    expected.insert("claimId".to_string(), Value::from(claim_id));
    expected
}

/// `logStoreUnavailable`; the call then runs unprotected.
fn store_unavailable(error: &KvError) -> Claim {
    tracing::warn!(%error, "Idempotency store unavailable, proceeding unprotected");
    Claim::Unprotected
}

/// `possibleIdempotencyErrors[code]` (declared on every write route: `defined`).
fn idempotency_error(code: &str) -> Response {
    let (status, message) = match code {
        "idempotencyKeyInvalid" => (422, "Idempotency-Key must be 1-255 characters"),
        "idempotencyKeyReused" => (
            422,
            "This Idempotency-Key was already used with a different request",
        ),
        _ => (
            409,
            "A request with this Idempotency-Key is still in progress",
        ),
    };
    let mut error = OrpcError::new(code, Some(status), Some(message));
    error.defined = true;
    error.into_response()
}

/// The replayed answer: the stored output with the first status.
fn replay(output: Option<Value>, status: Option<u16>) -> Response {
    let status = status.unwrap_or(200);
    let mut response = match output {
        Some(output) => crate::handlers::json(status, output),
        None => {
            let mut response = Response::new(Body::empty());
            *response.status_mut() =
                axum::http::StatusCode::from_u16(status).unwrap_or(axum::http::StatusCode::OK);
            response
        }
    };
    response
        .headers_mut()
        .insert(IDEMPOTENT_REPLAYED_HEADER, HeaderValue::from_static("true"));
    response
}
