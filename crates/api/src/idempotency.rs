//! Public API `Idempotency-Key` (`apps/builder/src/lib/idempotency/*`,
//! `apps/builder/src/middlewares/idempotency.ts`). The records live in a
//! [`CasStore`] (production: `DurableKv`, kv K11) under
//! `api-idempotency:<credentialId>:<procedurePath>:<key>`.

use std::future::Future;

use axum::http::HeaderMap;
use axum::response::Response;
use chatbotx_kv::CasStore;
use serde_json::Value;

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
    let _ = path;
    todo!("P3.6-D2")
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
    let _ = input;
    todo!("P3.6-D2")
}

/// `isValidIdempotencyKey`: 1..=255 UTF-16 code units.
pub fn is_valid_idempotency_key(value: &str) -> bool {
    let _ = value;
    todo!("P3.6-D2")
}

/// `redactOneTimeSecret`: an object output with a non-null `secret` gets
/// `secret: null`.
pub fn redact_one_time_secret(output: Value) -> Value {
    let _ = output;
    todo!("P3.6-D2")
}

/// `buildStoreKey`.
pub fn store_key(scope: &Scope<'_>) -> String {
    let _ = scope;
    todo!("P3.6-D2")
}

/// `claimIdempotencyKey`.
pub async fn claim_idempotency_key(
    store: &dyn CasStore,
    scope: &Scope<'_>,
    fingerprint: &str,
) -> Claim {
    let _ = (store, scope, fingerprint);
    todo!("P3.6-D2")
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
    let _ = (store, scope, fingerprint, claim_id, output, status);
    todo!("P3.6-D2")
}

/// `releaseIdempotencyKey`.
pub async fn release_idempotency_key(store: &dyn CasStore, scope: &Scope<'_>, claim_id: &str) {
    let _ = (store, scope, claim_id);
    todo!("P3.6-D2")
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
    let _ = (store, call, next);
    todo!("P3.6-D2")
}

/// The procedure input of a non-GET public call (oRPC compact input): the
/// path params merged with a JSON object body; a non-object JSON body alone;
/// a body that is not JSON as `__raw__:<sha256>`.
pub fn procedure_input(params: &[(&'static str, String)], body: &[u8]) -> Value {
    let _ = (params, body);
    todo!("P3.6-D2")
}
