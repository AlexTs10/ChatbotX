//! Port of `packages/business/__tests__/ai-file.service.test.ts` (Q.D6).
//! Adapted: the mocked drizzle client, repository and uploader become a real
//! migrated database and an in-memory object store; the queue and audit
//! spies become recording enqueuers; `fetch` is a fake `HttpFetcher`.
//! Not applicable:
//! - the template guard tests ("guards re-chunking first inside the locked
//!   transaction", "a blocked re-chunk", "a plain reindex never takes the
//!   guard", "is refused for a file a template installed"): no template locks
//!   or delete guards (P3.6-D1 rule);
//! - "keeps the queued run when recording the failure also fails" and
//!   "swallows a storage failure": a real database write / in-memory delete
//!   cannot be made to fail; both branches log and keep going (see the
//!   module).

mod common;
mod x1_support;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use chatbotx_business::ai_file::{
    self as service, AiFileDeps, CreateAiFileInput, ListChunksInput, ReindexConfig,
};
use chatbotx_business::audit::{AuditActor, AuditService, with_audit_context};
use chatbotx_business::errors::BusinessError;
use chatbotx_business::tx;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use chatbotx_files::{
    Body, FetchError, FetchOptions, HttpFetcher, HttpResponse, InputFile, LocalUrlSigner, Uploader,
};
use chatbotx_jobs::Enqueuer;
use common::{OTHER_WS, WS, scalar_i64, scalar_text, setup, two_workspaces};
use http::{HeaderMap, HeaderValue};
use serde_json::{Value, json};
use x1_support::RecordingEnqueuer;

const FILE: i64 = 9101;
const PATH: &str = "workspaces/2001/ai-files/uploaded.pdf";

struct Fixture {
    db: common::TestDb,
    files: Arc<Uploader>,
    jobs: Arc<RecordingEnqueuer>,
    jobs_dyn: Arc<dyn Enqueuer>,
    audits: Arc<RecordingEnqueuer>,
    audit: AuditService,
    fetcher: Option<Arc<dyn HttpFetcher>>,
}

impl Fixture {
    fn deps(&self) -> AiFileDeps<'_> {
        AiFileDeps {
            db: &self.db,
            files: &self.files,
            jobs: Some(&self.jobs_dyn),
            audit: &self.audit,
            fetcher: self.fetcher.clone(),
        }
    }

    fn audit_details(&self) -> Vec<String> {
        self.audits
            .calls()
            .iter()
            .filter_map(|c| c.data["data"]["detail"].as_str().map(str::to_owned))
            .collect()
    }
}

fn actor() -> AuditActor {
    AuditActor {
        user_id: Some("1001".into()),
        workspace_id: Some(WS.to_string()),
        ..AuditActor::default()
    }
}

/// Two workspaces; `WS` has an OpenAI integration and file `FILE`.
async fn fixture() -> Fixture {
    let db = two_workspaces().await;
    setup(&db, |tx| {
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
            r#"INSERT INTO "AIFile" ("id", "name", "path", "size", "mimeType", "workspaceId")
               VALUES (?1, 'manual.pdf', ?2, 456, 'application/pdf', ?3)"#,
            params![FILE, PATH, WS],
        )?;
        Ok(())
    })
    .await;
    let signer = LocalUrlSigner::new("secret", "https://app.test").expect("signer");
    let files = Arc::new(Uploader::in_memory().with_url_signer(signer));
    let jobs = RecordingEnqueuer::new();
    let audits = RecordingEnqueuer::new();
    Fixture {
        db,
        files,
        jobs_dyn: jobs.clone(),
        jobs,
        audit: AuditService::new(audits.clone()),
        audits,
        fetcher: None,
    }
}

async fn no_embedding_provider(db: &Db) {
    setup(db, |tx| {
        tx.execute(r#"DELETE FROM "Integration""#, [])?;
        Ok(())
    })
    .await;
}

fn path_input(path: &str) -> CreateAiFileInput {
    CreateAiFileInput::Path {
        name: "manual.pdf".into(),
        path: path.into(),
        mime_type: "application/pdf".into(),
        size: 456,
    }
}

async fn stored_run(db: &Db, id: i64) -> Value {
    scalar_text(
        db,
        r#"SELECT "processingRun" FROM "AIFile" WHERE "id" = ?1"#,
        id,
    )
    .await
    .map_or(Value::Null, |text| {
        serde_json::from_str(&text).expect("json")
    })
}

async fn set_run(db: &Db, id: i64, run: Value) {
    setup(db, move |tx| {
        tx.execute(
            r#"UPDATE "AIFile" SET "processingRun" = ?1 WHERE "id" = ?2"#,
            params![run.to_string(), id],
        )?;
        Ok(())
    })
    .await;
}

fn code(error: &BusinessError) -> (Option<&str>, Option<u16>) {
    (error.code(), error.status())
}

type Respond = Box<dyn Fn(&str) -> Result<HttpResponse, FetchError> + Send + Sync>;

/// `fetch`: records the URLs and answers with `respond`.
struct FakeFetcher {
    urls: Mutex<Vec<String>>,
    respond: Respond,
}

impl FakeFetcher {
    fn new(
        respond: impl Fn(&str) -> Result<HttpResponse, FetchError> + Send + Sync + 'static,
    ) -> Arc<FakeFetcher> {
        Arc::new(FakeFetcher {
            urls: Mutex::new(Vec::new()),
            respond: Box::new(respond),
        })
    }

    fn urls(&self) -> Vec<String> {
        self.urls.lock().expect("lock").clone()
    }
}

#[async_trait]
impl HttpFetcher for FakeFetcher {
    async fn fetch(&self, url: &str, _opts: FetchOptions) -> Result<HttpResponse, FetchError> {
        self.urls.lock().expect("lock").push(url.to_owned());
        (self.respond)(url)
    }
}

fn response(status: u16, headers: &[(&'static str, &str)], body: &'static [u8]) -> HttpResponse {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.insert(*name, HeaderValue::from_str(value).expect("header"));
    }
    HttpResponse {
        status,
        headers: map,
        body: Body::Buffered(Bytes::from_static(body)),
    }
}

// describe("aiFileService.create")

#[tokio::test]
async fn path_mode_inserts_enqueues_the_heavy_job_and_audits() {
    let f = fixture().await;
    let created = with_audit_context(actor(), service::create(&f.deps(), WS, path_input(PATH)))
        .await
        .expect("created");
    let calls = f.jobs.calls();
    assert_eq!(calls.len(), 1);
    let run = created.processing_run.clone().expect("run");
    assert_eq!(calls[0].name, "processAIFile");
    assert_eq!(
        calls[0].data,
        json!({ "type": "processAIFile", "data": { "aiFileId": created.file.id.to_string(), "runId": run["id"] } })
    );
    assert_eq!(
        calls[0].opts.job_id.as_deref(),
        Some(format!("heavy-ai-file-{}", created.file.id).as_str())
    );
    assert_eq!(
        f.audit_details(),
        vec![format!("created a new Knowledge (#{})", created.file.id)]
    );
    assert_eq!(
        (created.processing_status, created.chunks_count),
        ("pending", 0)
    );
    assert_eq!(
        (&run["status"], &run["heartbeatAt"], &run["stale"]),
        (&json!("queued"), &Value::Null, &json!(false))
    );
    assert_eq!(stored_run(&f.db, created.file.id).await["status"], "queued");
    assert!(created.url.starts_with("https://app.test/"));
}

#[tokio::test]
async fn path_mode_refuses_another_workspaces_object_another_folder_a_climb_and_a_bare_key() {
    let f = fixture().await;
    for path in [
        "workspaces/2002/ai-files/x.pdf",
        "workspaces/2001/secrets/x",
        "workspaces/2001/ai-files/../../2002/ai-files/x",
        "uploads/x.pdf",
    ] {
        let error = service::create(&f.deps(), WS, path_input(path))
            .await
            .expect_err(path);
        assert_eq!(code(&error), (Some("businessError"), Some(400)), "{path}");
    }
    assert_eq!(
        scalar_i64(
            &f.db,
            r#"SELECT count(*) FROM "AIFile" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(1)
    );
}

#[tokio::test]
async fn file_mode_uploads_the_file_and_returns_a_pending_resource() {
    let f = fixture().await;
    let file = InputFile {
        name: "manual.pdf".into(),
        mime_type: "application/pdf".into(),
        bytes: Bytes::from_static(b"hello"),
    };
    let created = service::create(&f.deps(), WS, CreateAiFileInput::File { name: None, file })
        .await
        .expect("created");
    assert!(
        created.file.path.starts_with("workspaces/2001/ai-files/"),
        "{}",
        created.file.path
    );
    assert_eq!(
        f.files
            .get_object(&created.file.path)
            .await
            .expect("stored"),
        Bytes::from_static(b"hello")
    );
    assert_eq!(
        (
            created.file.name.as_str(),
            created.file.size,
            created.processing_status,
            created.chunks_count
        ),
        ("manual.pdf", 5, "pending", 0)
    );
}

#[tokio::test]
async fn url_mode_checks_ssrf_safety_before_downloading_on_every_hop() {
    let mut f = fixture().await;
    let fetcher = FakeFetcher::new(|url| {
        Ok(if url.ends_with("redirect") {
            response(302, &[("location", "http://169.254.169.254/latest")], b"")
        } else {
            response(200, &[("content-type", "application/pdf")], b"%PDF")
        })
    });
    f.fetcher = Some(fetcher.clone());
    let created = service::create(
        &f.deps(),
        WS,
        CreateAiFileInput::Url {
            name: None,
            url: "https://93.184.215.34/manual.pdf".into(),
        },
    )
    .await
    .expect("created");
    assert_eq!(fetcher.urls(), vec!["https://93.184.215.34/manual.pdf"]);
    assert_eq!(
        (
            created.file.name.as_str(),
            created.file.mime_type.as_str(),
            created.file.size
        ),
        ("manual.pdf", "application/pdf", 4)
    );
    // A redirect hop to a private address is checked too and refused.
    let error = service::create(
        &f.deps(),
        WS,
        CreateAiFileInput::Url {
            name: None,
            url: "https://93.184.215.34/redirect".into(),
        },
    )
    .await
    .expect_err("blocked hop");
    assert_eq!(error.message(), "The provided URL is not allowed");
    assert_eq!(fetcher.urls().len(), 2);
}

#[tokio::test]
async fn url_mode_surfaces_an_ssrf_rejection_as_a_business_error_without_fetching() {
    let mut f = fixture().await;
    let fetcher = FakeFetcher::new(|_| Err(FetchError("unreachable".into())));
    f.fetcher = Some(fetcher.clone());
    let error = service::create(
        &f.deps(),
        WS,
        CreateAiFileInput::Url {
            name: None,
            url: "http://169.254.169.254/latest".into(),
        },
    )
    .await
    .expect_err("blocked");
    assert_eq!(code(&error), (Some("businessError"), Some(400)));
    assert_eq!(error.message(), "The provided URL is not allowed");
    assert!(!error.message().contains("169.254.169.254"));
    assert!(fetcher.urls().is_empty());
}

#[tokio::test]
async fn url_mode_reports_an_infrastructure_failure_as_a_generic_5xx_without_leaking_details() {
    let mut f = fixture().await;
    f.fetcher = Some(FakeFetcher::new(|_| {
        Err(FetchError("connect ECONNREFUSED 10.0.4.12:9000".into()))
    }));
    let error = service::create(
        &f.deps(),
        WS,
        CreateAiFileInput::Url {
            name: None,
            url: "https://93.184.215.34/manual.pdf".into(),
        },
    )
    .await
    .expect_err("down");
    assert_eq!(code(&error), (Some("systemError"), Some(502)));
    assert!(!error.message().contains("ECONNREFUSED"));
    assert!(!error.message().contains("10.0.4.12"));
}

#[tokio::test]
async fn rejects_when_neither_openai_nor_gemini_is_configured() {
    let f = fixture().await;
    no_embedding_provider(&f.db).await;
    let error = service::create(&f.deps(), WS, path_input(PATH))
        .await
        .expect_err("no provider");
    assert_eq!(error.code(), Some("noEmbeddingProvider"));
    assert!(error.message().contains("No embedding provider configured"));
    assert_eq!(
        scalar_i64(
            &f.db,
            r#"SELECT count(*) FROM "AIFile" WHERE "workspaceId" = ?1"#,
            WS
        )
        .await,
        Some(1)
    );
}

#[tokio::test]
async fn a_gemini_integration_is_enough() {
    let f = fixture().await;
    no_embedding_provider(&f.db).await;
    setup(&f.db, |tx| {
        tx.execute(
            r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (82, ?1, 'gemini')"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationGemini" ("id", "auth", "model", "maxOutputTokens", "workspaceId", "integrationId")
               VALUES (83, '{}', 'gemini-2.5-flash', 1000, ?1, 82)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    service::create(&f.deps(), WS, path_input(PATH))
        .await
        .expect("created");
}

// describe("aiFileService.create when enqueueing fails")

#[tokio::test]
async fn records_enqueue_failed_audits_and_still_returns_the_stored_file() {
    let f = fixture().await;
    f.jobs.fail_next("redis unavailable");
    let created = with_audit_context(actor(), service::create(&f.deps(), WS, path_input(PATH)))
        .await
        .expect("still created");
    let run = created.processing_run.clone().expect("run");
    assert_eq!(
        (&run["status"], &run["error"]),
        (&json!("error"), &json!("enqueueFailed"))
    );
    assert!(run["finishedAt"].is_string());
    let stored = stored_run(&f.db, created.file.id).await;
    assert_eq!(
        (&stored["status"], &stored["error"]),
        (&json!("error"), &json!("enqueueFailed"))
    );
    assert_eq!(
        f.audit_details(),
        vec![format!("created a new Knowledge (#{})", created.file.id)]
    );
}

// describe("aiFileService.reindex")

#[tokio::test]
async fn claims_a_run_and_enqueues_it_with_a_unique_reindex_job_id() {
    let f = fixture().await;
    let out = service::reindex(&f.deps(), WS, FILE, ReindexConfig::Keep)
        .await
        .expect("reindexed");
    let run = stored_run(&f.db, FILE).await;
    assert_eq!(
        (&run["status"], &run["heartbeatAt"]),
        (&json!("queued"), &Value::Null)
    );
    let calls = f.jobs.calls();
    assert_eq!(
        calls[0].data["data"],
        json!({ "aiFileId": FILE.to_string(), "runId": run["id"] })
    );
    assert_eq!(
        calls[0].opts.job_id.as_deref(),
        Some(
            format!(
                "heavy-ai-file-{FILE}-reindex-{}",
                run["id"].as_str().unwrap()
            )
            .as_str()
        )
    );
    assert!(!out.coalesced);
    assert_eq!(out.file.processing_run.expect("run")["id"], run["id"]);
}

fn active_run() -> Value {
    json!({
        "id": "run-active",
        "status": "processing",
        "error": null,
        "queuedAt": "2999-01-01T00:00:00.000Z",
        "startedAt": "2999-01-01T00:00:00.000Z",
        "heartbeatAt": "2999-01-01T00:00:00.000Z",
        "finishedAt": null,
    })
}

#[tokio::test]
async fn coalesces_without_adding_another_job_when_an_active_run_owns_the_file() {
    let f = fixture().await;
    set_run(&f.db, FILE, active_run()).await;
    let out = service::reindex(&f.deps(), WS, FILE, ReindexConfig::Keep)
        .await
        .expect("coalesced");
    assert!(out.coalesced);
    assert!(f.jobs.calls().is_empty());
    assert_eq!(out.file.processing_run.expect("run")["id"], "run-active");
}

#[tokio::test]
async fn marks_the_run_as_enqueue_failed_before_surfacing_an_enqueue_failure() {
    let f = fixture().await;
    f.jobs.fail_next("redis unavailable");
    let error = service::reindex(&f.deps(), WS, FILE, ReindexConfig::Keep)
        .await
        .expect_err("502");
    assert_eq!(code(&error), (Some("systemError"), Some(502)));
    let run = stored_run(&f.db, FILE).await;
    assert_eq!(
        (&run["status"], &run["error"]),
        (&json!("error"), &json!("enqueueFailed"))
    );
}

#[tokio::test]
async fn gives_every_claimed_run_its_own_job_id() {
    let f = fixture().await;
    service::reindex(&f.deps(), WS, FILE, ReindexConfig::Keep)
        .await
        .expect("first");
    // Adapted: the first run must finish before a second claim (a real claim, not a mock).
    let first = stored_run(&f.db, FILE).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        service::mark_processing_run_succeeded(&f.db, FILE, WS, first.clone())
            .await
            .expect("finished")
    );
    service::reindex(&f.deps(), WS, FILE, ReindexConfig::Keep)
        .await
        .expect("second");
    let ids: Vec<String> = f
        .jobs
        .calls()
        .iter()
        .filter_map(|c| c.opts.job_id.clone())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    assert_eq!(ids[0], format!("heavy-ai-file-{FILE}-reindex-{first}"));
}

#[tokio::test]
async fn does_not_audit_a_coalesced_reindex() {
    let f = fixture().await;
    set_run(&f.db, FILE, active_run()).await;
    with_audit_context(
        actor(),
        service::reindex(&f.deps(), WS, FILE, ReindexConfig::Keep),
    )
    .await
    .expect("coalesced");
    assert!(f.audit_details().is_empty());
}

#[tokio::test]
async fn audits_a_claimed_reindex() {
    let f = fixture().await;
    with_audit_context(
        actor(),
        service::reindex(&f.deps(), WS, FILE, ReindexConfig::Keep),
    )
    .await
    .expect("claimed");
    assert_eq!(
        f.audit_details(),
        vec![format!("reindexed a Knowledge (#{FILE})")]
    );
}

#[tokio::test]
async fn rejects_a_file_of_another_workspace_without_claiming() {
    let f = fixture().await;
    let error = service::reindex(&f.deps(), OTHER_WS, FILE, ReindexConfig::Keep)
        .await
        .expect_err("foreign");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(stored_run(&f.db, FILE).await, Value::Null);
}

#[tokio::test]
async fn rejects_a_workspace_without_an_embedding_provider_before_claiming() {
    let f = fixture().await;
    no_embedding_provider(&f.db).await;
    let error = service::reindex(&f.deps(), WS, FILE, ReindexConfig::Keep)
        .await
        .expect_err("no provider");
    assert_eq!(error.code(), Some("noEmbeddingProvider"));
    assert_eq!(stored_run(&f.db, FILE).await, Value::Null);
}

// describe("aiFileService.reindex with chunking parameters")

async fn stored_config(db: &Db) -> Option<Value> {
    scalar_text(
        db,
        r#"SELECT "chunkingConfig" FROM "AIFile" WHERE "id" = ?1"#,
        FILE,
    )
    .await
    .map(|text| serde_json::from_str(&text).expect("json"))
}

#[tokio::test]
async fn stores_the_normalized_parameters_in_the_claim() {
    let f = fixture().await;
    service::reindex(
        &f.deps(),
        WS,
        FILE,
        ReindexConfig::Set(json!({ "chunkSize": 300, "overlapSize": 400 })),
    )
    .await
    .expect("claimed");
    assert_eq!(
        stored_config(&f.db).await,
        Some(
            json!({ "strategy": "auto", "chunkSize": 300, "overlapSize": 150, "tabularHeaderRow": 1 })
        )
    );
}

/// Finishes the stored run (a real claim needs it before another reindex).
async fn finish_run(db: &Db) {
    let id = stored_run(db, FILE).await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    service::mark_processing_run_succeeded(db, FILE, WS, id)
        .await
        .expect("finished");
}

#[tokio::test]
async fn null_resets_to_the_defaults_and_omitting_keeps_the_stored_ones() {
    let f = fixture().await;
    service::reindex(
        &f.deps(),
        WS,
        FILE,
        ReindexConfig::Set(json!({ "chunkSize": 500 })),
    )
    .await
    .expect("set");
    finish_run(&f.db).await;
    service::reindex(&f.deps(), WS, FILE, ReindexConfig::Keep)
        .await
        .expect("keep");
    assert_eq!(stored_config(&f.db).await.expect("kept")["chunkSize"], 500);
    finish_run(&f.db).await;
    service::reindex(&f.deps(), WS, FILE, ReindexConfig::Reset)
        .await
        .expect("reset");
    assert_eq!(stored_config(&f.db).await, None);
}

#[tokio::test]
async fn rejects_parameters_outside_the_limits_without_claiming() {
    let f = fixture().await;
    let error = service::reindex(
        &f.deps(),
        WS,
        FILE,
        ReindexConfig::Set(json!({ "chunkSize": 50 })),
    )
    .await
    .expect_err("too small");
    let BusinessError::Exception { code, field, .. } = &error else {
        panic!("{error:?}")
    };
    assert_eq!(
        (code.as_str(), field.as_deref()),
        ("validation", Some("chunkingConfig"))
    );
    assert_eq!(stored_run(&f.db, FILE).await, Value::Null);
    assert!(f.jobs.calls().is_empty());
}

#[tokio::test]
async fn answers_409_instead_of_coalescing_when_parameters_were_sent() {
    let f = fixture().await;
    set_run(&f.db, FILE, active_run()).await;
    let error = service::reindex(
        &f.deps(),
        WS,
        FILE,
        ReindexConfig::Set(json!({ "chunkSize": 500 })),
    )
    .await
    .expect_err("conflict");
    assert_eq!(code(&error), (Some("conflict"), Some(409)));
    assert!(f.jobs.calls().is_empty());
    assert_eq!(stored_config(&f.db).await, None);
}

#[tokio::test]
async fn a_null_reset_on_an_active_run_is_also_a_conflict() {
    let f = fixture().await;
    set_run(&f.db, FILE, active_run()).await;
    let error = service::reindex(&f.deps(), WS, FILE, ReindexConfig::Reset)
        .await
        .expect_err("conflict");
    assert_eq!(error.code(), Some("conflict"));
}

// describe("aiFileService.listChunks")

/// Chunks `(id, chunkIndex)` of `FILE`.
async fn with_chunks(db: &Db, chunks: &'static [(i64, Option<i64>)]) {
    setup(db, move |tx| {
        for (id, index) in chunks {
            tx.execute(
                r#"INSERT INTO "AIEmbedding" ("id", "content", "status", "chunkIndex", "workspaceId", "aiFileId", "createdAt")
                   VALUES (?1, ?2, 'success', ?3, ?4, ?5, 1792800000000)"#,
                params![id, id.to_string(), index, WS, FILE],
            )?;
        }
        Ok(())
    })
    .await;
}

fn chunks_input(limit: i64) -> ListChunksInput {
    ListChunksInput {
        workspace_id: WS,
        ai_file_id: FILE,
        limit,
        ..ListChunksInput::default()
    }
}

fn decode(cursor: Option<&str>) -> Value {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor.unwrap_or_default())
        .expect("base64url");
    serde_json::from_slice(&bytes).expect("json")
}

#[tokio::test]
async fn list_chunks_rejects_a_file_of_another_workspace() {
    let f = fixture().await;
    let error = service::list_chunks(
        &f.db,
        ListChunksInput {
            workspace_id: OTHER_WS,
            ..chunks_input(2)
        },
    )
    .await
    .expect_err("foreign");
    assert_eq!(error.code(), Some("notFound"));
}

#[tokio::test]
async fn rejects_a_malformed_cursor_before_querying() {
    use base64::Engine as _;
    let f = fixture().await;
    let b64 = |v: Value| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string());
    for cursor in [
        "abc".to_owned(),
        "2".to_owned(),
        b64(json!({ "i": 1, "id": "x" })),
        b64(json!({ "i": -1, "id": "2" })),
    ] {
        let error = service::list_chunks(
            &f.db,
            ListChunksInput {
                cursor: Some(cursor.clone()),
                ..chunks_input(2)
            },
        )
        .await
        .expect_err(&cursor);
        assert_eq!(code(&error), (Some("invalidCursor"), Some(400)), "{cursor}");
    }
}

#[tokio::test]
async fn returns_an_opaque_next_cursor_only_when_another_page_exists() {
    let f = fixture().await;
    with_chunks(&f.db, &[(11, Some(0)), (12, Some(1)), (13, Some(2))]).await;
    let first = service::list_chunks(&f.db, chunks_input(2))
        .await
        .expect("first");
    assert_eq!(
        first.data.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![11, 12]
    );
    assert_eq!(
        decode(first.next_cursor.as_deref()),
        json!({ "i": 1, "id": "12" })
    );
    let last = service::list_chunks(
        &f.db,
        ListChunksInput {
            cursor: first.next_cursor,
            ..chunks_input(2)
        },
    )
    .await
    .expect("last");
    assert_eq!(last.data.iter().map(|c| c.id).collect::<Vec<_>>(), vec![13]);
    assert_eq!(last.next_cursor, None);
}

#[tokio::test]
async fn encodes_a_null_position_for_unnumbered_rows() {
    let f = fixture().await;
    with_chunks(&f.db, &[(21, None), (22, None), (23, None)]).await;
    let page = service::list_chunks(&f.db, chunks_input(2))
        .await
        .expect("page");
    assert_eq!(
        decode(page.next_cursor.as_deref()),
        json!({ "i": null, "id": "22" })
    );
    assert_eq!(page.data[0].chunk_index, None);
}

#[tokio::test]
async fn passes_from_chunk_index_to_the_first_page() {
    let f = fixture().await;
    with_chunks(&f.db, &[(31, Some(6)), (32, Some(7)), (33, Some(8))]).await;
    let page = service::list_chunks(
        &f.db,
        ListChunksInput {
            from_chunk_index: Some(7),
            ..chunks_input(2)
        },
    )
    .await
    .expect("page");
    assert_eq!(
        page.data.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![32, 33]
    );
}

// describe("aiFileService.findChunksContaining")

async fn contents(db: &Db, rows: &'static [(i64, &'static str)]) {
    setup(db, move |tx| {
        for (i, (id, content)) in rows.iter().enumerate() {
            tx.execute(
                r#"INSERT INTO "AIEmbedding" ("id", "content", "chunkIndex", "workspaceId", "aiFileId")
                   VALUES (?1, ?2, ?3, ?4, ?5)"#,
                params![id, content, i as i64, WS, FILE],
            )?;
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn escapes_like_wildcards_and_scopes_to_files_of_the_workspace() {
    let f = fixture().await;
    contents(
        &f.db,
        &[
            (41, "Get 50%_OFF\\ today"),
            (42, "Get 50 percent off"),
            (43, "500 off"),
        ],
    )
    .await;
    let matches = tx::read(&f.db, |conn| {
        service::find_chunks_containing(
            conn,
            WS,
            &[FILE.to_string(), "9".into()],
            "50%_off\\",
            None,
        )
    })
    .await
    .expect("matches");
    assert_eq!(
        matches,
        vec![service::ChunkContaining {
            chunk_id: 41,
            file_id: FILE,
            file_name: "manual.pdf".into(),
            chunk_index: Some(0),
        }]
    );
}

#[tokio::test]
async fn returns_nothing_when_no_file_belongs_to_the_workspace_or_the_text_is_blank() {
    let f = fixture().await;
    contents(&f.db, &[(51, "x")]).await;
    let (foreign, blank) = tx::read(&f.db, |conn| {
        Ok((
            service::find_chunks_containing(conn, OTHER_WS, &[FILE.to_string()], "x", None)?,
            service::find_chunks_containing(conn, WS, &[FILE.to_string()], "   ", None)?,
        ))
    })
    .await
    .expect("read");
    assert!(foreign.is_empty());
    assert!(blank.is_empty());
}

// describe("aiFileService.findManyByIds")

#[tokio::test]
async fn drops_ids_that_are_not_numeric_before_querying() {
    let f = fixture().await;
    let (found, none) = tx::read(&f.db, |conn| {
        Ok((
            service::find_many_by_ids(conn, WS, &[FILE.to_string(), "abc".into(), "2".into()])?,
            service::find_many_by_ids(conn, WS, &["abc".into()])?,
        ))
    })
    .await
    .expect("read");
    assert_eq!(found.iter().map(|f| f.id).collect::<Vec<_>>(), vec![FILE]);
    assert!(none.is_empty());
}

// describe("aiFileService.markProcessingRunFailed")

#[tokio::test]
async fn rejects_an_unknown_error_code_without_writing() {
    let f = fixture().await;
    set_run(&f.db, FILE, active_run()).await;
    service::mark_processing_run_failed(&f.db, FILE, WS, "run-active".into(), "boom")
        .await
        .expect_err("unknown code");
    assert_eq!(stored_run(&f.db, FILE).await["status"], "processing");
}

// describe("aiFileService.findWithProcessing chunking parameters and positions")

async fn found(f: &Fixture) -> service::AiFileWithProcessing {
    service::find_with_processing(&f.db, &f.files, WS, FILE)
        .await
        .expect("read")
        .expect("found")
}

#[tokio::test]
async fn reports_the_defaults_when_nothing_is_stored() {
    let f = fixture().await;
    let file = found(&f).await;
    assert_eq!(
        (json!(file.chunking_config), file.chunking_config_is_default),
        (
            json!({ "strategy": "auto", "chunkSize": 1000, "overlapSize": 200, "tabularHeaderRow": 1 }),
            true
        )
    );
}

#[tokio::test]
async fn reports_stored_parameters_merged_with_the_defaults() {
    let f = fixture().await;
    setup(&f.db, |tx| {
        tx.execute(
            r#"UPDATE "AIFile" SET "chunkingConfig" = '{"chunkSize":500}'"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let file = found(&f).await;
    assert_eq!(
        (
            file.chunking_config.chunk_size,
            file.chunking_config.overlap_size,
            file.chunking_config_is_default
        ),
        (500, 200, false)
    );
}

#[tokio::test]
async fn index_outdated_with_no_some_or_all_chunks_unnumbered() {
    for (chunks, expected) in [
        (&[][..], false),
        (&[(61, Some(0))][..], false),
        (&[(61, Some(0)), (62, None)][..], true),
        (&[(61, None)][..], true),
    ] {
        let f = fixture().await;
        let chunks: &'static [(i64, Option<i64>)] = Box::leak(chunks.to_vec().into_boxed_slice());
        with_chunks(&f.db, chunks).await;
        assert_eq!(found(&f).await.index_outdated, expected, "{chunks:?}");
    }
}

// describe("aiFileService.findWithProcessing derived fields")

fn chunking(strategy: &str, version: i64) -> Value {
    json!({
        "strategy": strategy,
        "chunkerVersion": version,
        "chunkSize": 1000,
        "overlapSize": 0,
        "chunkCount": 1,
        "processedAt": "2026-10-24T00:00:00.000Z",
    })
}

#[test]
fn chunking_outdated_with_missing_older_current_or_malformed_metadata() {
    // The TS test bumps `tabular` to 2 with a module mock; here the versions are an argument.
    let bumped: &[(&str, i64)] = &[("prose", 1), ("tabular", 2), ("markdown", 1)];
    let parsed = |v: Value| service::parse_chunking(Some(&v));
    for (stored, chunks, expected) in [
        (None, 1, true),
        (None, 0, false),
        (parsed(chunking("tabular", 1)), 1, true),
        (parsed(chunking("prose", 1)), 1, false),
        (parsed(json!({ "strategy": "tabular" })), 1, true),
    ] {
        assert_eq!(
            service::chunking_outdated(stored.as_ref(), chunks, bumped),
            expected,
            "{stored:?}"
        );
    }
}

const NOW: i64 = 1_792_843_200_000; // 2026-10-24T12:00:00.000Z
const HOUR: i64 = 3_600_000;

fn iso(offset: i64) -> String {
    chatbotx_core::js::to_iso_string(NOW + offset).expect("iso")
}

fn run(overrides: Value) -> Value {
    let mut run = json!({
        "id": "run-1",
        "status": "processing",
        "error": null,
        "queuedAt": iso(-3 * HOUR),
        "startedAt": null,
        "heartbeatAt": null,
        "finishedAt": null,
    });
    for (k, v) in overrides.as_object().expect("object") {
        run[k] = v.clone();
    }
    run
}

fn stale(overrides: Value) -> bool {
    let run = service::parse_processing_run(Some(&run(overrides))).expect("valid");
    service::run_with_stale(&run, NOW)["stale"]
        .as_bool()
        .expect("stale")
}

#[test]
fn stale_uses_heartbeat_at_when_present() {
    assert!(!stale(json!({ "heartbeatAt": iso(-HOUR + 1) })));
    assert!(stale(json!({ "heartbeatAt": iso(-HOUR - 1) })));
}

#[test]
fn stale_falls_back_to_queued_at_without_a_heartbeat() {
    assert!(!stale(
        json!({ "status": "queued", "queuedAt": iso(-HOUR + 1) })
    ));
    assert!(stale(
        json!({ "status": "queued", "queuedAt": iso(-HOUR - 1) })
    ));
}

#[test]
fn is_never_stale_once_finished() {
    assert!(!stale(
        json!({ "status": "success", "finishedAt": iso(-2 * HOUR) })
    ));
}

#[tokio::test]
async fn reads_a_malformed_run_as_null() {
    let f = fixture().await;
    set_run(&f.db, FILE, run(json!({ "status": "unknown-status" }))).await;
    assert_eq!(found(&f).await.processing_run, None);
}

// describe("aiFileService.delete")

#[tokio::test]
async fn deletes_storage_and_the_row_then_audits() {
    let f = fixture().await;
    f.files
        .put_object(
            PATH,
            Bytes::from_static(b"pdf"),
            chatbotx_files::PutOptions::default(),
            None,
        )
        .await
        .expect("stored");
    with_chunks(&f.db, &[(71, Some(0))]).await;
    with_audit_context(actor(), service::delete(&f.deps(), WS, FILE))
        .await
        .expect("deleted");
    assert!(f.files.get_object(PATH).await.is_err());
    assert_eq!(
        scalar_i64(
            &f.db,
            r#"SELECT count(*) FROM "AIFile" WHERE "id" = ?1"#,
            FILE
        )
        .await,
        Some(0)
    );
    assert_eq!(
        scalar_i64(
            &f.db,
            r#"SELECT count(*) FROM "AIEmbedding" WHERE "aiFileId" = ?1"#,
            FILE
        )
        .await,
        Some(0)
    );
    assert_eq!(
        f.audit_details(),
        vec![format!("deleted a Knowledge (#{FILE})")]
    );
}

#[tokio::test]
async fn delete_throws_when_the_file_does_not_belong_to_the_workspace() {
    let f = fixture().await;
    let error = service::delete(&f.deps(), OTHER_WS, FILE)
        .await
        .expect_err("foreign");
    assert_eq!(error.message(), format!("AIFile with id {FILE} not found"));
}

#[tokio::test]
async fn does_not_log_the_legacy_ai_agent_knowledge_base_message() {
    let f = fixture().await;
    with_audit_context(actor(), async {
        service::create(&f.deps(), WS, path_input(PATH))
            .await
            .expect("created");
        service::delete(&f.deps(), WS, FILE).await.expect("deleted");
    })
    .await;
    assert!(!f.audit_details().is_empty());
    for detail in f.audit_details() {
        assert!(!detail.contains("updated the AI Agent knowledge base"));
    }
}

// describe("aiFileService.listWithEmbeddingStatus")

#[tokio::test]
async fn maps_status_precedence_error_beats_pending_beats_success() {
    let f = fixture().await;
    setup(&f.db, |tx| {
        for (id, name) in [
            (9201, "error.pdf"),
            (9202, "pending.pdf"),
            (9203, "success.pdf"),
        ] {
            tx.execute(
                r#"INSERT INTO "AIFile" ("id", "name", "path", "size", "mimeType", "workspaceId")
                   VALUES (?1, ?2, ?2, 1, 'application/pdf', ?3)"#,
                params![id, name, WS],
            )?;
        }
        for (id, file, status) in [
            (81, 9201, "success"),
            (82, 9201, "error"),
            (83, 9202, "pending"),
            (84, 9203, "success"),
        ] {
            tx.execute(
                r#"INSERT INTO "AIEmbedding" ("id", "content", "status", "workspaceId", "aiFileId")
                   VALUES (?1, 'c', ?2, ?3, ?4)"#,
                params![id, status, WS, file],
            )?;
        }
        Ok(())
    })
    .await;
    let rows = service::list_with_embedding_status(&f.db, &f.files, WS)
        .await
        .expect("list");
    let status = |id: i64| {
        rows.iter()
            .find(|r| r.file.id == id)
            .map(|r| r.processing_status)
    };
    assert_eq!(
        (status(9201), status(9202), status(9203), status(FILE)),
        (
            Some("error"),
            Some("processing"),
            Some("success"),
            Some("pending")
        )
    );
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|r| r.url.starts_with("https://app.test/")));
}

// describe("aiFileService.markChunked")

fn chunking_input() -> Value {
    json!({
        "strategy": "tabular",
        "chunkerVersion": 1,
        "chunkSize": 1000,
        "overlapSize": 0,
        "chunkCount": 2,
        "processedAt": "2026-10-24T00:00:00.000Z",
    })
}

async fn stored_chunking(db: &Db) -> Option<String> {
    scalar_text(
        db,
        r#"SELECT "chunking" FROM "AIFile" WHERE "id" = ?1"#,
        FILE,
    )
    .await
}

#[tokio::test]
async fn validates_then_writes_the_workspace_scoped_chunking() {
    let f = fixture().await;
    service::mark_chunked(&f.db, FILE, OTHER_WS, chunking_input())
        .await
        .expect("no-op for another workspace");
    assert_eq!(stored_chunking(&f.db).await, None);
    service::mark_chunked(&f.db, FILE, WS, chunking_input())
        .await
        .expect("written");
    let stored: Value =
        serde_json::from_str(&stored_chunking(&f.db).await.expect("stored")).unwrap();
    assert_eq!(stored, chunking_input());
}

#[tokio::test]
async fn rejects_invalid_or_unknown_metadata_without_writing() {
    let f = fixture().await;
    let mut bad_date = chunking_input();
    bad_date["processedAt"] = json!("not-an-iso-date");
    let mut unknown = chunking_input();
    unknown["unexpected"] = json!(true);
    for chunking in [bad_date, unknown] {
        service::mark_chunked(&f.db, FILE, WS, chunking)
            .await
            .expect_err("invalid");
    }
    assert_eq!(stored_chunking(&f.db).await, None);
}
