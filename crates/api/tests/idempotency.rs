//! Ports of `apps/builder/__tests__/api-idempotency.test.ts`,
//! `api-idempotency-middleware.test.ts` and `idempotency-read-exemption.test.ts`
//! (P3.6-D2). The in-memory `Map` store of the originals is a real
//! `MemKv` (`CasStore`); the mocked claim / complete / release results are
//! real store states; `next` is a counting handler closure.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use chatbotx_api::idempotency::{
    BROADCAST_AUDIENCE_PREVIEW_TOKEN_PATH, Claim, IN_FLIGHT_TTL_MS, MAX_STORED_OUTPUT_BYTES,
    MiddlewareCall, Scope, api_idempotency_middleware, claim_idempotency_key,
    complete_idempotency_key, fingerprint_input, is_exempt_read_path, release_idempotency_key,
    store_key,
};
use chatbotx_kv::{BoxFuture, CasStore, KvError, MemKv, TestClock};
use serde_json::{Map, Value, json};

const SCOPE: Scope<'static> = Scope {
    credential_id: "api-token:token-1",
    procedure_path: "tags.create",
    idempotency_key: "key-1",
};

fn store() -> (Arc<TestClock>, MemKv) {
    let clock = Arc::new(TestClock::new(1_800_000_000_000));
    let kv = MemKv::new(clock.clone());
    (clock, kv)
}

/// A store whose every call fails (`mockRejectedValue(new Error("Redis unavailable"))`).
struct UnavailableStore;

fn unavailable() -> KvError {
    KvError::WrongType {
        key: "Redis unavailable".to_string(),
    }
}

impl CasStore for UnavailableStore {
    fn set_if_absent<'a>(
        &'a self,
        _key: &'a str,
        _value: &'a Value,
        _ttl_ms: u64,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        Box::pin(async { Err(unavailable()) })
    }
    fn get_raw<'a>(&'a self, _key: &'a str) -> BoxFuture<'a, Result<Option<String>, KvError>> {
        Box::pin(async { Err(unavailable()) })
    }
    fn get_json<'a>(&'a self, _key: &'a str) -> BoxFuture<'a, Result<Option<Value>, KvError>> {
        Box::pin(async { Err(unavailable()) })
    }
    fn del<'a>(&'a self, _key: &'a str) -> BoxFuture<'a, Result<(), KvError>> {
        Box::pin(async { Err(unavailable()) })
    }
    fn compare_and_swap<'a>(
        &'a self,
        _key: &'a str,
        _expected: Option<&'a Map<String, Value>>,
        _next: &'a Value,
        _ttl_ms: u64,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        Box::pin(async { Err(unavailable()) })
    }
    fn compare_and_delete<'a>(
        &'a self,
        _key: &'a str,
        _expected: &'a Map<String, Value>,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        Box::pin(async { Err(unavailable()) })
    }
}

async fn claimed(store: &dyn CasStore, scope: &Scope<'_>, fingerprint: &str) -> String {
    match claim_idempotency_key(store, scope, fingerprint).await {
        Claim::Claimed { claim_id } => claim_id,
        other => panic!("expected a claim, got {other:?}"),
    }
}

// ---- api-idempotency.test.ts ----

#[tokio::test]
async fn claims_once_rejects_concurrent_callers_and_replays_a_completed_output() {
    let (_clock, kv) = store();
    let fingerprint = fingerprint_input(&json!({ "name": "tag" }));
    let claim_id = claimed(&kv, &SCOPE, &fingerprint).await;
    assert_eq!(
        claim_idempotency_key(&kv, &SCOPE, &fingerprint).await,
        Claim::InFlight
    );
    complete_idempotency_key(
        &kv,
        &SCOPE,
        &fingerprint,
        &claim_id,
        Some(&json!({ "id": "tag-1" })),
        None,
    )
    .await;
    assert_eq!(
        claim_idempotency_key(&kv, &SCOPE, &fingerprint).await,
        Claim::Replay {
            output: Some(json!({ "id": "tag-1" })),
            status: None
        }
    );
}

#[tokio::test]
async fn never_stores_or_replays_a_one_time_secret() {
    let (_clock, kv) = store();
    let fingerprint = fingerprint_input(&json!({ "provider": "api" }));
    let claim_id = claimed(&kv, &SCOPE, &fingerprint).await;
    complete_idempotency_key(
        &kv,
        &SCOPE,
        &fingerprint,
        &claim_id,
        Some(&json!({
            "connection": { "id": "conn-1" },
            "session": null,
            "secret": { "kind": "api_channel_token", "token": "cbx_plaintext" },
        })),
        None,
    )
    .await;
    let raw = kv
        .get_raw(&store_key(&SCOPE))
        .await
        .expect("read")
        .expect("record");
    assert!(!raw.contains("cbx_plaintext"), "{raw}");
    assert_eq!(
        claim_idempotency_key(&kv, &SCOPE, &fingerprint).await,
        Claim::Replay {
            output: Some(
                json!({ "connection": { "id": "conn-1" }, "session": null, "secret": null })
            ),
            status: None
        }
    );
}

#[tokio::test]
async fn rejects_a_key_reused_with_a_different_payload() {
    let (_clock, kv) = store();
    claimed(&kv, &SCOPE, &fingerprint_input(&json!({ "name": "first" }))).await;
    assert_eq!(
        claim_idempotency_key(
            &kv,
            &SCOPE,
            &fingerprint_input(&json!({ "name": "second" }))
        )
        .await,
        Claim::FingerprintMismatch
    );
}

#[tokio::test]
async fn releases_only_its_own_claim() {
    let (_clock, kv) = store();
    let fingerprint = fingerprint_input(&json!({ "name": "tag" }));
    let claim_id = claimed(&kv, &SCOPE, &fingerprint).await;
    release_idempotency_key(&kv, &SCOPE, "stale").await;
    assert_eq!(
        claim_idempotency_key(&kv, &SCOPE, &fingerprint).await,
        Claim::InFlight
    );
    release_idempotency_key(&kv, &SCOPE, &claim_id).await;
    assert!(matches!(
        claim_idempotency_key(&kv, &SCOPE, &fingerprint).await,
        Claim::Claimed { .. }
    ));
}

#[tokio::test]
async fn does_not_release_a_claim_re_acquired_after_the_original_claim_expires() {
    let (clock, kv) = store();
    let fingerprint = fingerprint_input(&json!({ "name": "tag" }));
    let first = claimed(&kv, &SCOPE, &fingerprint).await;
    // `records.clear()`: the in-flight claim expires.
    clock.advance(i64::try_from(IN_FLIGHT_TTL_MS).expect("ttl") + 1);
    let second = claimed(&kv, &SCOPE, &fingerprint).await;
    assert_ne!(first, second);
    release_idempotency_key(&kv, &SCOPE, &first).await;
    assert_eq!(
        claim_idempotency_key(&kv, &SCOPE, &fingerprint).await,
        Claim::InFlight
    );
}

#[tokio::test]
async fn fails_open_when_the_store_is_unavailable() {
    assert_eq!(
        claim_idempotency_key(
            &UnavailableStore,
            &SCOPE,
            &fingerprint_input(&json!({ "name": "tag" }))
        )
        .await,
        Claim::Unprotected
    );
}

#[tokio::test]
async fn releases_an_oversized_completed_output() {
    let (_clock, kv) = store();
    let fingerprint = fingerprint_input(&json!({ "name": "tag" }));
    let claim_id = claimed(&kv, &SCOPE, &fingerprint).await;
    complete_idempotency_key(
        &kv,
        &SCOPE,
        &fingerprint,
        &claim_id,
        Some(&json!({ "value": "x".repeat(MAX_STORED_OUTPUT_BYTES) })),
        None,
    )
    .await;
    assert!(matches!(
        claim_idempotency_key(&kv, &SCOPE, &fingerprint).await,
        Claim::Claimed { .. }
    ));
}

#[tokio::test]
async fn isolates_keys_by_procedure_path() {
    let (_clock, kv) = store();
    let fingerprint = fingerprint_input(&json!({ "name": "tag" }));
    claimed(&kv, &SCOPE, &fingerprint).await;
    let other = Scope {
        procedure_path: "contacts.create",
        ..SCOPE
    };
    claimed(&kv, &other, &fingerprint).await;
}

#[test]
fn different_date_inputs_produce_different_fingerprints() {
    // A JSON input carries a Date as its ISO string.
    let first = fingerprint_input(&json!({ "startAt": "2026-09-25T10:00:00.000Z" }));
    let second = fingerprint_input(&json!({ "startAt": "2026-09-25T11:00:00.000Z" }));
    assert_ne!(second, first);
}

#[test]
fn key_order_does_not_change_the_fingerprint() {
    let first: Value =
        serde_json::from_str(r#"{"name":"tag","nested":{"color":"red","size":"large"}}"#)
            .expect("json");
    let second: Value =
        serde_json::from_str(r#"{"nested":{"size":"large","color":"red"},"name":"tag"}"#)
            .expect("json");
    assert_eq!(fingerprint_input(&second), fingerprint_input(&first));
}

// ---- idempotency-read-exemption.test.ts ----

#[test]
fn the_audience_preview_is_exempt_so_a_repeated_key_never_replays_stale_rows() {
    assert!(is_exempt_read_path(BROADCAST_AUDIENCE_PREVIEW_TOKEN_PATH));
}

#[test]
fn a_write_route_is_never_exempt() {
    assert!(!is_exempt_read_path("/v1/broadcasts"));
    assert!(!is_exempt_read_path("/v1/contacts/bulk/tags"));
}

// ---- api-idempotency-middleware.test.ts ----

fn headers(key: Option<&str>) -> HeaderMap {
    let mut map = HeaderMap::new();
    if let Some(key) = key {
        map.insert(
            "idempotency-key",
            HeaderValue::from_str(key).expect("header"),
        );
    }
    map
}

fn ok_json(body: Value) -> Response {
    chatbotx_api::handlers::json(200, body)
}

async fn body_json(response: Response) -> (u16, Value, Option<String>) {
    let status = response.status().as_u16();
    let replayed = response
        .headers()
        .get("idempotent-replayed")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("JSON body")
    };
    (status, json, replayed)
}

fn req<'a>(
    headers: &'a HeaderMap,
    method: &'a str,
    route_path: Option<&'a str>,
    procedure_path: &'a str,
    input: &'a Value,
) -> MiddlewareCall<'a> {
    MiddlewareCall {
        headers,
        method,
        route_path,
        procedure_path,
        credential_id: "api-token:1",
        input,
    }
}

/// Runs the middleware with a counting handler that answers `answer`.
async fn run(
    store: &dyn CasStore,
    call: MiddlewareCall<'_>,
    calls: &AtomicUsize,
    answer: Response,
) -> (u16, Value, Option<String>) {
    let response = api_idempotency_middleware(store, call, || async move {
        calls.fetch_add(1, Ordering::SeqCst);
        Some(answer)
    })
    .await
    .expect("handled");
    body_json(response).await
}

fn middleware_scope(procedure_path: &str) -> Scope<'_> {
    Scope {
        credential_id: "api-token:1",
        procedure_path,
        idempotency_key: "key-1",
    }
}

#[tokio::test]
async fn skips_requests_without_a_key() {
    let (_clock, kv) = store();
    let calls = AtomicUsize::new(0);
    let input = json!({ "name": "tag" });
    let out = run(
        &kv,
        req(&headers(None), "POST", None, "tags.create", &input),
        &calls,
        ok_json(json!({ "id": "tag-1" })),
    )
    .await;
    assert_eq!(out, (200, json!({ "id": "tag-1" }), None));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let key = store_key(&Scope {
        idempotency_key: "key-1",
        ..middleware_scope("tags.create")
    });
    assert_eq!(kv.get_raw(&key).await.expect("read"), None);
}

#[tokio::test]
async fn skips_get_requests_with_a_key() {
    let (_clock, kv) = store();
    let calls = AtomicUsize::new(0);
    run(
        &kv,
        req(
            &headers(Some("key-1")),
            "GET",
            None,
            "tags.list",
            &json!({}),
        ),
        &calls,
        ok_json(json!({ "id": "tag-1" })),
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let key = store_key(&middleware_scope("tags.list"));
    assert_eq!(kv.get_raw(&key).await.expect("read"), None);
}

#[tokio::test]
async fn skips_a_post_that_only_reads_so_a_repeated_key_never_replays_stale_rows() {
    let (_clock, kv) = store();
    let calls = AtomicUsize::new(0);
    let input = json!({ "channels": ["omnichannel"] });
    let out = run(
        &kv,
        req(
            &headers(Some("key-1")),
            "POST",
            Some("/v1/broadcasts/audience/preview"),
            "broadcasts.previewAudience",
            &input,
        ),
        &calls,
        ok_json(json!({ "total": 3, "data": [] })),
    )
    .await;
    assert_eq!(out, (200, json!({ "total": 3, "data": [] }), None));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let key = store_key(&middleware_scope("broadcasts.previewAudience"));
    assert_eq!(kv.get_raw(&key).await.expect("read"), None);
}

#[tokio::test]
async fn still_protects_a_write_post_on_a_similar_path() {
    let (_clock, kv) = store();
    let calls = AtomicUsize::new(0);
    run(
        &kv,
        req(
            &headers(Some("key-1")),
            "POST",
            Some("/v1/broadcasts"),
            "broadcasts.create",
            &json!({}),
        ),
        &calls,
        ok_json(json!({ "id": "b1" })),
    )
    .await;
    let key = store_key(&middleware_scope("broadcasts.create"));
    assert!(kv.get_raw(&key).await.expect("read").is_some());
}

#[tokio::test]
async fn returns_a_replay_without_calling_the_handler() {
    let (_clock, kv) = store();
    let input = json!({ "name": "tag" });
    let scope = middleware_scope("tags.create");
    let fingerprint = fingerprint_input(&input);
    let claim_id = claimed(&kv, &scope, &fingerprint).await;
    complete_idempotency_key(
        &kv,
        &scope,
        &fingerprint,
        &claim_id,
        Some(&json!({ "id": "tag-1" })),
        Some(200),
    )
    .await;
    let calls = AtomicUsize::new(0);
    let out = run(
        &kv,
        req(&headers(Some("key-1")), "POST", None, "tags.create", &input),
        &calls,
        ok_json(json!({ "id": "tag-2" })),
    )
    .await;
    assert_eq!(
        out,
        (200, json!({ "id": "tag-1" }), Some("true".to_string()))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn rejects_a_key_reused_with_a_different_request() {
    let (_clock, kv) = store();
    let scope = middleware_scope("tags.create");
    claimed(&kv, &scope, &fingerprint_input(&json!({ "name": "other" }))).await;
    let calls = AtomicUsize::new(0);
    let (status, body, _) = run(
        &kv,
        req(
            &headers(Some("key-1")),
            "POST",
            None,
            "tags.create",
            &json!({ "name": "tag" }),
        ),
        &calls,
        ok_json(json!({})),
    )
    .await;
    assert_eq!(
        (status, &body["code"], &body["status"]),
        (422, &json!("idempotencyKeyReused"), &json!(422))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn rejects_an_in_flight_key() {
    let (_clock, kv) = store();
    let input = json!({ "name": "tag" });
    let scope = middleware_scope("tags.create");
    claimed(&kv, &scope, &fingerprint_input(&input)).await;
    let calls = AtomicUsize::new(0);
    let (status, body, _) = run(
        &kv,
        req(&headers(Some("key-1")), "POST", None, "tags.create", &input),
        &calls,
        ok_json(json!({})),
    )
    .await;
    assert_eq!(
        (status, &body["code"], &body["status"]),
        (409, &json!("idempotencyKeyConflict"), &json!(409))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn passes_through_when_the_idempotency_store_is_unavailable() {
    let calls = AtomicUsize::new(0);
    let out = run(
        &UnavailableStore,
        req(
            &headers(Some("key-1")),
            "POST",
            None,
            "tags.create",
            &json!({ "name": "tag" }),
        ),
        &calls,
        ok_json(json!({ "id": "tag-1" })),
    )
    .await;
    assert_eq!(out, (200, json!({ "id": "tag-1" }), None));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn completes_a_claimed_key_with_the_handler_output() {
    let (_clock, kv) = store();
    let calls = AtomicUsize::new(0);
    let out = run(
        &kv,
        req(
            &headers(Some("key-1")),
            "POST",
            None,
            "tags.create",
            &json!({ "name": "tag" }),
        ),
        &calls,
        ok_json(json!({ "id": "tag-1" })),
    )
    .await;
    assert_eq!(out, (200, json!({ "id": "tag-1" }), None));
    let record = kv
        .get_json(&store_key(&middleware_scope("tags.create")))
        .await
        .expect("read")
        .expect("record");
    assert_eq!(record["state"], json!("completed"));
    let output: Value =
        serde_json::from_str(record["output"].as_str().expect("output")).expect("json");
    assert_eq!(output, json!({ "id": "tag-1" }));
}

#[tokio::test]
async fn releases_a_claimed_key_when_the_handler_throws() {
    let (_clock, kv) = store();
    let calls = AtomicUsize::new(0);
    let failure = (StatusCode::INTERNAL_SERVER_ERROR, "failed").into_response();
    let (status, _, _) = run(
        &kv,
        req(
            &headers(Some("key-1")),
            "POST",
            None,
            "tags.create",
            &json!({ "name": "tag" }),
        ),
        &calls,
        failure,
    )
    .await;
    assert_eq!(status, 500);
    let key = store_key(&middleware_scope("tags.create"));
    assert_eq!(kv.get_raw(&key).await.expect("read"), None);
}

#[tokio::test]
async fn rejects_an_invalid_idempotency_key() {
    let (_clock, kv) = store();
    let calls = AtomicUsize::new(0);
    let (status, body, _) = run(
        &kv,
        req(&headers(Some(" ")), "POST", None, "tags.create", &json!({})),
        &calls,
        ok_json(json!({})),
    )
    .await;
    assert_eq!(
        (status, &body["code"], &body["status"]),
        (422, &json!("idempotencyKeyInvalid"), &json!(422))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn replays_an_http_response_without_re_running_the_handler() {
    let (_clock, kv) = store();
    let calls = AtomicUsize::new(0);
    let input = Value::Null;
    let first = run(
        &kv,
        req(
            &headers(Some("key-1")),
            "POST",
            Some("/idempotency-replay"),
            "replay",
            &input,
        ),
        &calls,
        ok_json(json!({ "at": "2026-09-25T10:00:00.000Z" })),
    )
    .await;
    let second = run(
        &kv,
        req(
            &headers(Some("key-1")),
            "POST",
            Some("/idempotency-replay"),
            "replay",
            &input,
        ),
        &calls,
        ok_json(json!({ "at": "2026-09-25T11:00:00.000Z" })),
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(second.2.as_deref(), Some("true"));
    assert_eq!(second.1, first.1);
}
