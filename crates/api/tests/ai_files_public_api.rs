//! Ports of `apps/builder/__tests__/ai-files-public-api.test.ts`,
//! `ai-files-reindex-api.test.ts`, `ai-files-resource-schema.test.ts`,
//! `ai-files-search-api.test.ts` and `create-ai-file-action.test.ts` (Q.D6).
//! Adapted: the captured oRPC procedures and the mocked `aiFileService`
//! become `handlers::dispatch` / `rpc_ai::call` over a real migrated database
//! and an in-memory object store; "the service was called with" becomes the
//! response and the stored rows; `schema.parse` becomes the mapped resource.
//! The translated messages are the `en` copies.
//! Not applicable here:
//! - "a read_only token cannot reindex", "a read_only token cannot search"
//!   and "a read_only token can still list chunks": the token permission is
//!   checked by the route auth before the handler (`routes`, P3.5);
//! - "a retry with the same Idempotency-Key replays the same file" and "is
//!   exempt from idempotency replay": the idempotency middleware wraps the
//!   handler in `routes` (its own tests; the exempt path is checked here).

mod handlers_common;

use std::sync::{Arc, Mutex};

use chatbotx_api::handlers::{ApiState, Call, dispatch};
use chatbotx_api::idempotency::is_exempt_read_path;
use chatbotx_api::routes::{route_by_id, scopes_of_module};
use chatbotx_api::rpc_ai::{RpcAiContext, call as rpc};
use chatbotx_business::ai::knowledge::{
    EMBEDDING_DIMENSIONS, EmbedFuture, Embedder, ResolvedEmbeddingModel,
};
use chatbotx_business::ai_file::parse_processing_run;
use chatbotx_db::rusqlite::params;
use chatbotx_files::{LocalUrlSigner, Uploader};
use chatbotx_jobs::{AddResult, BoxFuture, Enqueuer, JobOptions, JobsError, NewJob, QueueName};
use chatbotx_kv::{Kv, KvConfig, SystemClock};
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

const FILE: i64 = 9101;
const PATH: &str = "workspaces/2001/ai-files/prices.csv";

/// `heavyQueue.add` spy.
#[derive(Default)]
struct Jobs {
    calls: Mutex<Vec<(String, Value, JobOptions)>>,
}

impl Enqueuer for Jobs {
    fn add(
        &self,
        _queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        let job_id = opts.job_id.clone().unwrap_or_default();
        self.calls
            .lock()
            .expect("lock")
            .push((name.to_owned(), data, opts));
        Box::pin(async move {
            Ok(AddResult {
                job_id,
                created: true,
            })
        })
    }

    fn add_bulk(
        &self,
        _queue: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        let out = jobs
            .into_iter()
            .map(|job| AddResult {
                job_id: job.opts.job_id.unwrap_or_default(),
                created: true,
            })
            .collect();
        Box::pin(async move { Ok(out) })
    }
}

/// `WS` with an OpenAI integration and file `FILE` (3 chunks); files,
/// jobs and kv on. `jobs = None` = no queue.
async fn seeded(jobs: Option<Arc<Jobs>>) -> Fixture {
    let mut f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (80, ?1, 'openai')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationOpenai" ("id", "auth", "model", "maxOutputTokens", "workspaceId", "integrationId")
                   VALUES (81, '{}', 'gpt-4o', 1000, ?1, 80)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "AIFile" ("id", "name", "path", "size", "mimeType", "workspaceId", "createdAt", "updatedAt")
                   VALUES (?1, 'prices.csv', ?2, 10, 'text/csv', ?3, 1792800000000, 1792800000000)"#,
                params![FILE, PATH, WS],
            )?;
            for (id, index) in [(11_i64, 0_i64), (12, 1), (13, 2)] {
                tx.execute(
                    r#"INSERT INTO "AIEmbedding" ("id", "content", "status", "chunkIndex", "workspaceId", "aiFileId", "createdAt")
                       VALUES (?1, ?2, 'success', ?3, ?4, ?5, 1792800000000)"#,
                    params![id, format!("chunk {index}"), index, WS, FILE],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
    let signer = LocalUrlSigner::new("secret", "https://files.test").expect("signer");
    let files = Arc::new(Uploader::in_memory().with_url_signer(signer));
    let kv = Kv::start(
        Arc::clone(&f.state.db),
        Arc::new(SystemClock),
        KvConfig::default(),
    );
    let mut state: ApiState = f.state.clone().with_files(files).with_kv(kv);
    if let Some(jobs) = jobs {
        state = state.with_jobs(jobs);
    }
    f.state = state;
    f
}

const ID: &[(&str, &str)] = &[("id", "9101")];

/// A raw call (multipart bodies): status, `Retry-After`, JSON body.
async fn raw(f: &Fixture, route_id: &str, body: &[u8]) -> (u16, Option<String>, Value) {
    let response = dispatch(
        &f.state,
        route_id,
        Call {
            workspace_id: WS,
            params: &[],
            query: None,
            body,
        },
    )
    .await
    .expect("ported");
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("JSON body")
    };
    (status, retry_after, json)
}

fn multipart(fields: &[(&str, Option<&str>, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, filename, value) in fields {
        out.extend_from_slice(b"--XyZ\r\n");
        match filename {
            Some(filename) => out.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: text/csv\r\n\r\n"
                )
                .as_bytes(),
            ),
            None => out.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
            ),
        }
        out.extend_from_slice(value.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"--XyZ--\r\n");
    out
}

fn first_issue(body: &Value) -> (Value, Value) {
    let issue = &body["data"]["issues"][0];
    (issue["path"].clone(), issue["message"].clone())
}

#[test]
fn registers_the_ai_files_public_router_under_the_automation_scope() {
    assert_eq!(
        scopes_of_module("features/ai-files/api/public"),
        vec!["automation"]
    );
}

// describe("GET /v1/ai-files")

#[tokio::test]
async fn list_returns_the_workspaces_files_with_their_processing_fields() {
    let f = seeded(None).await;
    let (status, body) = call(&f, "aiFiles.list", WS, &[], Some("page=1&perPage=50"), None).await;
    assert_eq!(status, 200);
    assert_eq!(body["pageCount"], 1);
    let row = &body["data"][0];
    assert_eq!(
        (
            &row["id"],
            &row["workspaceId"],
            &row["chunksCount"],
            &row["processingStatus"],
            &row["indexOutdated"],
            &row["chunkingOutdated"]
        ),
        (
            &json!("9101"),
            &json!("2001"),
            &json!(3),
            &json!("success"),
            &json!(false),
            &json!(true)
        )
    );
    assert!(
        row["url"]
            .as_str()
            .is_some_and(|u| u.starts_with("https://files.test/"))
    );
    let (_, other) = call(&f, "aiFiles.list", OTHER_WS, &[], None, None).await;
    assert_eq!(other["data"], json!([]));
}

// describe("GET /v1/ai-files/{id}")

#[tokio::test]
async fn get_returns_the_file_of_the_workspace() {
    let f = seeded(None).await;
    let (status, body) = call(&f, "aiFiles.get", WS, ID, None, None).await;
    assert_eq!(status, 200);
    assert_eq!(
        (
            &body["id"],
            &body["createdAt"],
            &body["chunkingConfig"],
            &body["processingRun"]
        ),
        (
            &json!("9101"),
            &json!("2026-10-24T00:00:00.000Z"),
            &json!({ "strategy": "auto", "chunkSize": 1000, "overlapSize": 200, "tabularHeaderRow": 1, "isDefault": true }),
            &Value::Null
        )
    );
}

#[tokio::test]
async fn get_throws_not_found_when_the_file_does_not_exist() {
    let f = seeded(None).await;
    let (status, body) = call(&f, "aiFiles.get", OTHER_WS, ID, None, None).await;
    assert_eq!(
        (status, &body["code"], &body["message"]),
        (404, &json!("notFound"), &json!("AI file not found"))
    );
}

// describe("POST /v1/ai-files") + describe("createAIFilePublicRequest")

#[tokio::test]
async fn create_with_a_file_upload_stores_it_and_answers_201() {
    let jobs = Arc::new(Jobs::default());
    let f = seeded(Some(jobs.clone())).await;
    let (status, _, body) = raw(
        &f,
        "aiFiles.create",
        &multipart(&[("file", Some("manual.csv"), "a,b\n1,2")]),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(
        (
            &body["name"],
            &body["mimeType"],
            &body["size"],
            &body["processingStatus"],
            &body["processingRun"]["status"]
        ),
        (
            &json!("manual.csv"),
            &json!("text/csv"),
            &json!(7),
            &json!("pending"),
            &json!("queued")
        )
    );
    let calls = jobs.calls.lock().expect("lock").clone();
    assert_eq!(calls[0].0, "processAIFile");
    assert_eq!(
        calls[0].2.job_id.as_deref(),
        Some(format!("heavy-ai-file-{}", body["id"].as_str().unwrap()).as_str())
    );
}

#[tokio::test]
async fn create_with_a_name_field_uses_it() {
    let f = seeded(None).await;
    let (status, _, body) = raw(
        &f,
        "aiFiles.create",
        &multipart(&[
            ("name", None, "Price list"),
            ("file", Some("manual.csv"), "a"),
        ]),
    )
    .await;
    assert_eq!((status, &body["name"]), (201, &json!("Price list")));
}

#[tokio::test]
async fn create_with_a_source_url_goes_through_the_url_guard() {
    let f = seeded(None).await;
    // Accepted by the schema; the download is refused by the SSRF guard (no fetch).
    let (status, body) = call(
        &f,
        "aiFiles.create",
        WS,
        &[],
        None,
        Some(json!({ "url": "http://169.254.169.254/latest" })),
    )
    .await;
    assert_eq!(
        (status, &body["code"], &body["message"]),
        (
            400,
            &json!("businessError"),
            &json!("The provided URL is not allowed")
        )
    );
}

#[tokio::test]
async fn rejects_a_payload_carrying_both_file_and_url_instead_of_silently_dropping_one() {
    let f = seeded(None).await;
    let (status, _, body) = raw(
        &f,
        "aiFiles.create",
        &multipart(&[
            ("file", Some("manual.csv"), "a"),
            ("url", None, "https://example.com/manual.pdf"),
        ]),
    )
    .await;
    assert_eq!(status, 422);
    assert_eq!(
        first_issue(&body),
        (
            json!([]),
            json!("Provide either 'file' or 'url', not both.")
        )
    );
}

#[tokio::test]
async fn rejects_a_payload_with_neither_file_nor_url() {
    let f = seeded(None).await;
    let (status, body) = call(
        &f,
        "aiFiles.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "manual.pdf" })),
    )
    .await;
    assert_eq!(status, 422);
    assert_eq!(
        first_issue(&body),
        (json!([]), json!("Provide either 'file' or 'url'."))
    );
}

#[tokio::test]
async fn rejects_a_url_that_is_not_a_url() {
    let f = seeded(None).await;
    let (status, body) = call(
        &f,
        "aiFiles.create",
        WS,
        &[],
        None,
        Some(json!({ "url": "not a url" })),
    )
    .await;
    assert_eq!(
        (status, first_issue(&body)),
        (422, (json!(["url"]), json!("Invalid URL")))
    );
}

// describe("aiFiles.create when processing could not be queued")

#[tokio::test]
async fn still_answers_201_with_the_failed_run_so_the_caller_can_reindex() {
    let f = seeded(None).await;
    let (status, _, body) = raw(
        &f,
        "aiFiles.create",
        &multipart(&[("file", Some("prices.csv"), "a")]),
    )
    .await;
    assert_eq!(status, 201);
    assert_eq!(
        (
            &body["processingRun"]["status"],
            &body["processingRun"]["error"]
        ),
        (&json!("error"), &json!("enqueueFailed"))
    );
}

// describe("DELETE /v1/ai-files/{id}")

#[tokio::test]
async fn delete_removes_the_file_and_answers_204() {
    let f = seeded(None).await;
    let (status, body) = call(&f, "aiFiles.delete", WS, ID, None, None).await;
    assert_eq!((status, body), (204, Value::Null));
    assert_eq!(
        scalar(&f, r#"SELECT count(*) FROM "AIFile" WHERE "id" = ?1"#, FILE).await,
        Some(json!(0))
    );
}

// describe("phase-two AI file procedures")

#[tokio::test]
async fn list_chunks_pages_the_workspaces_file_with_the_default_limit() {
    let f = seeded(None).await;
    let (status, body) = call(&f, "aiFiles.listChunks", WS, ID, None, None).await;
    assert_eq!(status, 200);
    assert_eq!(body["data"].as_array().map(Vec::len), Some(3));
    assert_eq!(body["nextCursor"], Value::Null);
    assert_eq!(
        body["data"][0],
        json!({ "id": "11", "chunkIndex": 0, "content": "chunk 0", "status": "success", "createdAt": "2026-10-24T00:00:00.000Z" })
    );
    let (_, page) = call(&f, "aiFiles.listChunks", WS, ID, Some("limit=2"), None).await;
    let cursor = page["nextCursor"].as_str().expect("cursor").to_owned();
    let (_, last) = call(
        &f,
        "aiFiles.listChunks",
        WS,
        ID,
        Some(&format!("limit=2&cursor={cursor}")),
        None,
    )
    .await;
    assert_eq!(last["data"][0]["id"], "13");
    let (status, body) = call(&f, "aiFiles.listChunks", OTHER_WS, ID, None, None).await;
    assert_eq!((status, &body["code"]), (404, &json!("notFound")));
    let (status, body) = call(&f, "aiFiles.listChunks", WS, ID, Some("cursor=abc"), None).await;
    assert_eq!((status, &body["code"]), (400, &json!("invalidCursor")));
}

// describe("aiFiles.reindex")

async fn reindex(f: &Fixture, body: Option<Value>) -> (u16, Value) {
    call(f, "aiFiles.reindex", WS, ID, None, body).await
}

#[tokio::test]
async fn answers_202_with_the_new_run() {
    let jobs = Arc::new(Jobs::default());
    let f = seeded(Some(jobs.clone())).await;
    let (status, body) = reindex(&f, None).await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(
        (
            &body["id"],
            &body["coalesced"],
            &body["processingRun"]["status"],
            &body["processingRun"]["stale"]
        ),
        (
            &json!("9101"),
            &json!(false),
            &json!("queued"),
            &json!(false)
        )
    );
    // A new operation must not echo workspaceId.
    assert!(body.get("workspaceId").is_none());
    assert_eq!(jobs.calls.lock().expect("lock").len(), 1);
}

async fn finish_run(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "AIFile" SET "processingRun" = json_set("processingRun", '$.status', 'success')"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("finish");
}

#[tokio::test]
async fn passes_the_chunking_parameters_through_and_null_to_reset_them() {
    let f = seeded(Some(Arc::new(Jobs::default()))).await;
    let (status, body) = reindex(
        &f,
        Some(json!({ "chunkingConfig": { "chunkSize": 500, "overlapSize": 0, "tabularHeaderRow": 3 } })),
    )
    .await;
    assert_eq!(status, 202);
    assert_eq!(
        body["chunkingConfig"],
        json!({ "strategy": "auto", "chunkSize": 500, "overlapSize": 0, "tabularHeaderRow": 3, "isDefault": false })
    );
    finish_run(&f).await;
    let (status, body) = reindex(&f, Some(json!({ "chunkingConfig": null }))).await;
    assert_eq!(
        (status, &body["chunkingConfig"]["isDefault"]),
        (202, &json!(true))
    );
}

#[tokio::test]
async fn rejects_chunking_parameters_outside_the_schema_before_reindexing() {
    let jobs = Arc::new(Jobs::default());
    let f = seeded(Some(jobs.clone())).await;
    for config in [
        json!({ "chunkSize": 100 }),
        json!({ "chunkSize": 2001 }),
        json!({ "chunkSize": 5000 }),
        json!({ "overlapSize": 900 }),
        json!({ "tabularHeaderRow": 0 }),
        json!({ "strategy": "markdown" }),
        json!({ "unknown": true }),
    ] {
        let (status, body) = reindex(&f, Some(json!({ "chunkingConfig": config }))).await;
        assert_eq!(
            (status, &body["code"]),
            (422, &json!("invalidRequestData")),
            "{config}"
        );
    }
    assert!(jobs.calls.lock().expect("lock").is_empty());
}

async fn active(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "AIFile" SET "processingRun" = '{"id":"run-1","status":"processing","error":null,"queuedAt":"2999-01-01T00:00:00.000Z","startedAt":null,"heartbeatAt":"2999-01-01T00:00:00.000Z","finishedAt":null}'"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("active");
}

#[tokio::test]
async fn answers_409_when_parameters_meet_an_active_run() {
    let f = seeded(Some(Arc::new(Jobs::default()))).await;
    active(&f).await;
    let (status, body) = reindex(&f, Some(json!({ "chunkingConfig": { "chunkSize": 500 } }))).await;
    assert_eq!((status, &body["code"]), (409, &json!("conflict")));
}

#[tokio::test]
async fn reports_a_coalesced_run_without_hiding_the_existing_one() {
    let f = seeded(Some(Arc::new(Jobs::default()))).await;
    active(&f).await;
    let (status, body) = reindex(&f, None).await;
    assert_eq!(
        (status, &body["coalesced"], &body["processingRun"]["status"]),
        (202, &json!(true), &json!("processing"))
    );
}

#[tokio::test]
async fn rejects_a_non_numeric_id_before_reaching_the_service() {
    let f = seeded(None).await;
    let (status, body) = call(&f, "aiFiles.reindex", WS, &[("id", "abc")], None, None).await;
    assert_eq!((status, &body["code"]), (422, &json!("invalidRequestData")));
}

#[tokio::test]
async fn the_sixth_reindex_in_a_window_is_rate_limited() {
    let f = seeded(Some(Arc::new(Jobs::default()))).await;
    let params: Vec<(&'static str, String)> = vec![("id", FILE.to_string())];
    finish_run(&f).await;
    // ponytail: the first five calls fill the bucket in a tight loop (the route's own
    // check), so a 10 s window boundary cannot split the six calls on a loaded host.
    for _ in 0..chatbotx_api::handlers::ai_files::AI_FILE_OPERATION_LIMIT {
        assert!(
            chatbotx_api::handlers::ai_files::check_rate_limit(&f.state, "reindex", WS).is_ok()
        );
    }
    let response = dispatch(
        &f.state,
        "aiFiles.reindex",
        Call {
            workspace_id: WS,
            params: &params,
            query: None,
            body: b"",
        },
    )
    .await
    .expect("ported");
    let status = response.status().as_u16();
    let retry = response
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let body: Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!((status, &body["code"]), (429, &json!("tooManyRequests")));
    let retry: u64 = retry.expect("Retry-After").parse().expect("seconds");
    assert!((1..=10).contains(&retry), "{retry}");
}

// describe("server actions share the AI file buckets")

fn context() -> RpcAiContext {
    RpcAiContext::new(WS, 1001)
}

async fn action(
    f: &Fixture,
    path: &str,
    bind: Option<i64>,
    input: Option<Value>,
) -> Result<Value, chatbotx_api::orpc::OrpcError> {
    let mut args = vec![json!(WS.to_string())];
    args.extend(bind.map(|id| json!(id.to_string())));
    args.extend(input);
    rpc(&f.state, path, &context(), &json!({ "args": args }))
        .await
        .expect("ported")
        .map(|out| out.json)
}

#[tokio::test]
async fn the_sixth_reindex_action_call_is_rate_limited_with_a_translated_message_and_shares_the_api_bucket()
 {
    let f = seeded(Some(Arc::new(Jobs::default()))).await;
    let mut refused = None;
    // The public API fills the bucket; the action draws from the same one.
    for _ in 0..5 {
        finish_run(&f).await;
        reindex(&f, None).await;
    }
    for _ in 0..6 {
        finish_run(&f).await;
        if let Err(error) = action(&f, "actions.reindexAIFileAction", Some(FILE), None).await {
            refused = Some(error);
            break;
        }
    }
    let error = refused.expect("rate limited");
    assert_eq!(
        (error.code.as_str(), error.status, error.message.as_str()),
        (
            "tooManyRequests",
            429,
            "Too many requests. Try again shortly."
        )
    );
}

#[tokio::test]
async fn the_reindex_action_returns_whether_a_run_was_already_active() {
    let f = seeded(Some(Arc::new(Jobs::default()))).await;
    let out = action(&f, "actions.reindexAIFileAction", Some(FILE), None)
        .await
        .expect("reindexed");
    assert_eq!(out, json!({ "coalesced": false }));
    let error = action(
        &f,
        "actions.reindexAIFileAction",
        Some(FILE),
        Some(json!({ "chunkingConfig": { "chunkSize": 500 } })),
    )
    .await
    .expect_err("conflict");
    assert_eq!(
        (error.code.as_str(), error.status, error.message.as_str()),
        (
            "conflict",
            409,
            "This file is being processed, so the new parameters were not applied. Try again when processing finishes."
        )
    );
}

// describe("createAIFileAction")

fn create_input(path: &str) -> Value {
    json!({ "name": "manual.pdf", "path": path, "mimeType": "application/pdf", "size": 100 })
}

#[tokio::test]
async fn create_action_stores_an_uploaded_object_and_returns_its_id() {
    let f = seeded(None).await;
    let out = action(
        &f,
        "actions.createAIFileAction",
        None,
        Some(create_input("workspaces/2001/ai-files/manual.pdf")),
    )
    .await
    .expect("created");
    assert!(out["id"].as_str().is_some_and(|id| !id.is_empty()));
    // No queue in this fixture: the run is recorded as enqueueFailed.
    assert_eq!(out["processingRunError"], "enqueueFailed");
}

#[tokio::test]
async fn translates_a_no_embedding_provider_service_error() {
    let f = seeded(None).await;
    f.state
        .db
        .write(|tx| {
            tx.execute(r#"DELETE FROM "Integration""#, [])?;
            Ok(())
        })
        .await
        .expect("no provider");
    let error = action(
        &f,
        "actions.createAIFileAction",
        None,
        Some(create_input("workspaces/2001/ai-files/manual.pdf")),
    )
    .await
    .expect_err("no provider");
    assert_eq!(
        error.message,
        "No embedding provider configured. AI file embeddings require OpenAI or Gemini integration. DeepSeek, Claude and Muse do not support embedding models."
    );
}

#[tokio::test]
async fn rethrows_any_other_service_error_untranslated() {
    let f = seeded(None).await;
    let error = action(
        &f,
        "actions.createAIFileAction",
        None,
        Some(create_input("path/to/file")),
    )
    .await
    .expect_err("refused path");
    assert_eq!(error.message, "The uploaded file path is not allowed");
}

#[tokio::test]
async fn delete_action_deletes_the_bound_file() {
    let f = seeded(None).await;
    let out = action(&f, "actions.deleteAIFileAction", Some(FILE), None)
        .await
        .expect("deleted");
    assert_eq!(out, Value::Null);
    assert_eq!(
        scalar(&f, r#"SELECT count(*) FROM "AIFile" WHERE "id" = ?1"#, FILE).await,
        Some(json!(0))
    );
}

#[tokio::test]
async fn the_private_procedures_list_and_get_the_workspaces_files() {
    let f = seeded(None).await;
    let list = rpc(
        &f.state,
        "aiFilesAPI.listAIFilesAuthenticatedAPI",
        &context(),
        &json!({ "workspaceId": WS.to_string() }),
    )
    .await
    .expect("ported")
    .expect("listed");
    assert_eq!(list.json["data"][0]["id"], "9101");
    assert_eq!(
        list.meta,
        vec![
            json!([1, "data", 0, "createdAt"]),
            json!([1, "data", 0, "updatedAt"])
        ]
    );
    let get = rpc(
        &f.state,
        "aiFilesAPI.getAIFileAuthenticatedAPI",
        &context(),
        &json!({ "workspaceId": WS.to_string(), "id": FILE.to_string() }),
    )
    .await
    .expect("ported")
    .expect("found");
    assert_eq!(get.json["processingStatus"], "success");
}

// describe("AI file response schemas")

#[tokio::test]
async fn exposes_chunking_the_effective_parameters_and_the_run() {
    let f = seeded(None).await;
    let chunking = json!({
        "strategy": "tabular",
        "chunkerVersion": 1,
        "chunkSize": 1000,
        "overlapSize": 0,
        "chunkCount": 2,
        "processedAt": "2026-10-24T00:00:00.000Z",
        "extractedChars": 2600,
    });
    let run = json!({
        "id": "run-1",
        "status": "success",
        "error": null,
        "queuedAt": "2026-10-24T00:00:00.000Z",
        "startedAt": "2026-10-24T00:00:01.000Z",
        "heartbeatAt": "2026-10-24T00:00:02.000Z",
        "finishedAt": "2026-10-24T00:00:03.000Z",
    });
    let (c, r) = (chunking.to_string(), run.to_string());
    f.state
        .db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "AIFile" SET "chunking" = ?1, "processingRun" = ?2, "chunkingConfig" = '{"chunkSize":500}'"#,
                params![c, r],
            )?;
            Ok(())
        })
        .await
        .expect("stored");
    let (_, body) = call(&f, "aiFiles.get", WS, ID, None, None).await;
    let mut expected_run = run.clone();
    expected_run["stale"] = json!(false);
    assert_eq!(body["chunking"], chunking);
    assert_eq!(body["chunking"]["extractedChars"], 2600);
    assert_eq!(body["processingRun"], expected_run);
    assert_eq!(
        (
            &body["chunkingConfig"]["chunkSize"],
            &body["chunkingConfig"]["isDefault"],
            &body["chunkingOutdated"]
        ),
        (&json!(500), &json!(false), &json!(false))
    );
}

#[test]
fn rejects_a_run_with_an_unknown_error_code_or_a_non_utc_timestamp() {
    let run = json!({
        "id": "run-1",
        "status": "success",
        "error": null,
        "queuedAt": "2026-10-24T00:00:00.000Z",
        "startedAt": null,
        "heartbeatAt": null,
        "finishedAt": null,
    });
    assert!(parse_processing_run(Some(&run)).is_some());
    let mut boom = run.clone();
    boom["error"] = json!("boom");
    let mut offset = run.clone();
    offset["queuedAt"] = json!("2026-10-24T07:00:00.000+07:00");
    assert!(parse_processing_run(Some(&boom)).is_none());
    assert!(parse_processing_run(Some(&offset)).is_none());
}

// describe("aiFiles.search")

/// Answers every query with `[1, 0, ...]`; counts the calls.
#[derive(Default)]
struct FakeEmbedder {
    calls: Mutex<Vec<String>>,
}

impl Embedder for FakeEmbedder {
    fn embed<'a>(&'a self, _model: &'a ResolvedEmbeddingModel, value: &'a str) -> EmbedFuture<'a> {
        self.calls.lock().expect("lock").push(value.to_owned());
        Box::pin(async move {
            let mut vector = vec![0.0_f32; EMBEDDING_DIMENSIONS];
            vector[0] = 1.0;
            Ok(vector)
        })
    }
}

/// The stored vector with cosine similarity `s` to the query.
fn vector(s: f32) -> Vec<u8> {
    let mut v = vec![0.0_f32; EMBEDDING_DIMENSIONS];
    v[0] = s;
    v[1] = (1.0 - s * s).max(0.0).sqrt();
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// [`seeded`] with a usable OpenAI key, chunk vectors (11: 0.9, 12: 0.8 with
/// "Pro: 499.000", 13: 0.2) and a fake embedder.
async fn searchable() -> (Fixture, Arc<FakeEmbedder>) {
    let mut f = seeded(None).await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "IntegrationOpenai" SET "auth" = '{"authType":"secretText","secretText":"sk-test"}'"#,
                [],
            )?;
            for (id, s, content) in [(11_i64, 0.9_f32, "chunk 0"), (12, 0.8, "Pro: 499.000"), (13, 0.2, "chunk 2")] {
                tx.execute(
                    r#"UPDATE "AIEmbedding" SET "embedding" = ?1, "content" = ?2 WHERE "id" = ?3"#,
                    params![vector(s), content, id],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
    let embedder = Arc::new(FakeEmbedder::default());
    f.state = f.state.clone().with_embedder(embedder.clone());
    (f, embedder)
}

async fn search(f: &Fixture, body: Value) -> (u16, Value) {
    call(f, "aiFiles.search", WS, &[], None, Some(body)).await
}

/// Port of: apps/builder/__tests__/ai-files-public-api.test.ts:268 "phase-two AI file procedures > search delegates to the shared retrieval implementation"
#[tokio::test]
async fn search_delegates_to_the_shared_retrieval_implementation() {
    let (f, embedder) = searchable().await;
    let (status, body) = search(&f, json!({ "query": "hello", "fileIds": ["9101"] })).await;

    assert_eq!(status, 200);
    assert_eq!(*embedder.calls.lock().unwrap(), vec!["hello"]);
    assert_eq!(body["mode"], json!("files"));
    assert_eq!(body["files"][0]["id"], json!("9101"));
}

/// Port of: apps/builder/__tests__/ai-files-search-api.test.ts:121 "aiFiles.search > searches in the token's workspace"
#[tokio::test]
async fn searches_in_the_tokens_workspace() {
    let (f, _) = searchable().await;
    let (status, body) = search(&f, json!({ "query": "price", "fileIds": ["9101"] })).await;
    assert_eq!((status, &body["diagnosis"]), (200, &json!("ok")));

    // Adapted: the other workspace does not see the file.
    let (status, body) = call(
        &f,
        "aiFiles.search",
        OTHER_WS,
        &[],
        None,
        Some(json!({ "query": "price", "fileIds": ["9101"] })),
    )
    .await;
    assert_eq!((status, &body["code"]), (404, &json!("notFound")));
}

/// Port of: apps/builder/__tests__/ai-files-search-api.test.ts:132 "aiFiles.search > forwards the expected text and returns the verdict"
#[tokio::test]
async fn forwards_the_expected_text_and_returns_the_verdict() {
    let (f, _) = searchable().await;
    let (status, body) = search(
        &f,
        json!({ "query": "price", "fileIds": ["9101"], "expectedText": " 499.000 " }),
    )
    .await;

    assert_eq!(status, 200);
    assert_eq!(
        body["expected"],
        json!({
            "verdict": "inTopResults",
            "matches": [{ "fileId": "9101", "fileName": "prices.csv", "chunkId": "12", "chunkIndex": 1, "rank": 2 }],
        })
    );
}

/// Port of: apps/builder/__tests__/ai-files-search-api.test.ts:164 "aiFiles.search > rejects an expected text over 200 characters or blank"
#[tokio::test]
async fn rejects_an_expected_text_over_200_characters_or_blank() {
    let (f, embedder) = searchable().await;
    for expected in ["x".repeat(201), "   ".to_owned()] {
        let (status, body) = search(
            &f,
            json!({ "query": "price", "fileIds": ["9101"], "expectedText": expected }),
        )
        .await;
        assert_eq!((status, &body["code"]), (422, &json!("invalidRequestData")));
    }
    assert!(embedder.calls.lock().unwrap().is_empty());
}

/// Port of: apps/builder/__tests__/ai-files-search-api.test.ts:177 "aiFiles.search > returns chunk positions and agent visibility per result"
#[tokio::test]
async fn returns_chunk_positions_and_agent_visibility_per_result() {
    let (f, _) = searchable().await;
    let (_, body) = search(&f, json!({ "query": "price", "fileIds": ["9101"] })).await;

    let first = &body["results"][1];
    assert_eq!(
        (
            &first["chunkIndex"],
            &first["agentVisible"],
            &first["usedByAgent"],
            &first["content"]
        ),
        (
            &json!(1),
            &json!(true),
            &Value::Null,
            &json!("Pro: 499.000")
        )
    );
    assert_eq!(body["results"][2]["agentVisible"], json!(false));
}

/// Port of: apps/builder/__tests__/ai-files-search-api.test.ts:203 "aiFiles.search > rejects a request with both or neither of agentId and fileIds"
#[tokio::test]
async fn rejects_a_request_with_both_or_neither_of_agent_id_and_file_ids() {
    let (f, embedder) = searchable().await;
    for body in [
        json!({ "query": "price" }),
        json!({ "query": "price", "agentId": "1", "fileIds": ["9101"] }),
    ] {
        let (status, body) = search(&f, body).await;
        assert_eq!((status, &body["code"]), (422, &json!("invalidRequestData")));
        assert_eq!(
            first_issue(&body),
            (
                json!(["agentId"]),
                json!("Provide exactly one of agentId or fileIds.")
            )
        );
    }
    assert!(embedder.calls.lock().unwrap().is_empty());
}

/// Port of: apps/builder/__tests__/ai-files-search-api.test.ts:241 "aiFiles.search > listChunks forwards fromChunkIndex and returns positions"
#[tokio::test]
async fn list_chunks_forwards_from_chunk_index_and_returns_positions() {
    let f = seeded(None).await;
    let (status, body) = call(
        &f,
        "aiFiles.listChunks",
        WS,
        ID,
        Some("fromChunkIndex=1"),
        None,
    )
    .await;

    assert_eq!(status, 200);
    let indexes: Vec<&Value> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| &c["chunkIndex"])
        .collect();
    assert_eq!(indexes, vec![&json!(1), &json!(2)]);
}

/// Port of: apps/builder/__tests__/ai-files-search-api.test.ts:267 "aiFiles.search > the sixth search in a window is rate limited before embedding"
#[tokio::test]
async fn the_sixth_search_in_a_window_is_rate_limited_before_embedding() {
    let (f, embedder) = searchable().await;
    let body = json!({ "query": "q", "fileIds": ["9101"] }).to_string();
    let mut last = (0, None, Value::Null);
    let mut sent = 0;
    for _ in 0..12 {
        sent += 1;
        last = raw(&f, "aiFiles.search", body.as_bytes()).await;
        if last.0 == 429 {
            break;
        }
    }
    // Adapted: a window boundary may split the calls; the refused call never embeds.
    let (status, retry, body) = last;
    assert_eq!((status, &body["code"]), (429, &json!("tooManyRequests")));
    let retry: u64 = retry.expect("Retry-After").parse().expect("seconds");
    assert!((1..=10).contains(&retry), "{retry}");
    assert_eq!(embedder.calls.lock().unwrap().len(), sent - 1);
}

/// Port of: apps/builder/__tests__/ai-files-search-api.test.ts:318 "aiFiles.search > is registered at the path the idempotency exemption names"
#[test]
fn is_registered_at_the_path_the_idempotency_exemption_names() {
    let route = route_by_id("aiFiles.search").expect("route");
    assert_eq!((route.method, route.path), ("POST", "/v1/ai-files/search"));
    assert!(is_exempt_read_path(route.path));
}

// describe("searchAIFilesAction")

#[tokio::test]
async fn the_search_action_shares_the_api_bucket_with_a_translated_message() {
    let (f, _) = searchable().await;
    let input = json!({ "query": "q", "fileIds": ["9101"] });
    let out = action(&f, "actions.searchAIFilesAction", None, Some(input.clone()))
        .await
        .expect("searched");
    assert_eq!(out["diagnosis"], json!("ok"));
    let mut refused = None;
    for _ in 0..12 {
        if let Err(error) =
            action(&f, "actions.searchAIFilesAction", None, Some(input.clone())).await
        {
            refused = Some(error);
            break;
        }
    }
    let error = refused.expect("rate limited");
    assert_eq!(
        (error.code.as_str(), error.status, error.message.as_str()),
        (
            "tooManyRequests",
            429,
            "Too many test requests. Try again shortly."
        )
    );
}
