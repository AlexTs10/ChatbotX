//! Ports of `packages/business/__tests__/integration-openai-compatible-base-url.test.ts`
//! and `integration-openai-compatible-service.test.ts` (Q.D6). Adapted: the
//! mocked `isCloud` / `assertPublicUrl` become the `cloud` flag and a fake
//! DoH client (a lookup = one DoH call); the mocked drizzle client becomes a
//! real migrated database.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use chatbotx_business::audit::AuditService;
use chatbotx_business::integration_openai_compatible::{
    self as service, ConnectInput, UpdateInput, normalize_base_url,
    validate_base_url_for_environment,
};
use chatbotx_business::net::ssrf_guard::{DohFetch, DohReply};
use chatbotx_db::rusqlite::params;
use common::{WS, setup, two_workspaces};
use serde_json::json;

struct FakeDoh {
    calls: AtomicUsize,
}

impl FakeDoh {
    fn new() -> FakeDoh {
        FakeDoh {
            calls: AtomicUsize::new(0),
        }
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl DohFetch for FakeDoh {
    async fn get(&self, _url: &str) -> Result<DohReply, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(DohReply {
            status: 200,
            body: json!({ "Status": 0, "Answer": [{ "type": 1, "data": "93.184.216.34" }] })
                .to_string(),
        })
    }
}

// describe("OpenAI-compatible base URL validation")

#[test]
fn normalizes_trimmed_http_and_https_urls() {
    assert_eq!(
        normalize_base_url(" https://example.com/v1 ").expect("https"),
        "https://example.com/v1"
    );
    assert_eq!(
        normalize_base_url("http://127.0.0.1:1234/v1").expect("http"),
        "http://127.0.0.1:1234/v1"
    );
}

#[test]
fn rejects_invalid_non_http_and_oversized_urls() {
    let long = format!("https://example.com/{}", "a".repeat(2049));
    for url in ["", "ftp://example.com", long.as_str()] {
        let error = normalize_base_url(url).expect_err("invalid");
        assert_eq!(error.message(), "OpenAI-compatible base URL is invalid.");
    }
}

#[tokio::test]
async fn blocks_unsafe_urls_in_cloud_mode() {
    let error =
        validate_base_url_for_environment("http://127.0.0.1:1234/v1", true, &FakeDoh::new())
            .await
            .expect_err("blocked");
    assert_eq!(
        (error.code(), error.status()),
        (Some("ssrfBlocked"), Some(400))
    );
}

#[tokio::test]
async fn allows_private_urls_in_self_host_mode_without_ssrf_lookup() {
    let doh = FakeDoh::new();
    let url = validate_base_url_for_environment("http://127.0.0.1:1234/v1", false, &doh)
        .await
        .expect("self-host");
    assert_eq!(url, "http://127.0.0.1:1234/v1");
    assert_eq!(doh.calls(), 0);
}

// describe("IntegrationOpenaiCompatibleService")

async fn stored_base_url(db: &common::TestDb, id: i64) -> String {
    db.read(move |conn| {
        Ok(conn.query_row(
            r#"SELECT "baseURL" FROM "IntegrationOpenaiCompatible" WHERE "id" = ?1"#,
            params![id],
            |r| r.get::<_, String>(0),
        )?)
    })
    .await
    .expect("read")
}

async fn seeded(base_url: &'static str) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        tx.execute(
            r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (80, ?1, 'openaiCompatible')"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationOpenaiCompatible" ("id", "baseURL", "defaultModel", "integrationId", "name", "preset", "workspaceId")
               VALUES (81, ?1, 'model-1', 80, 'NIM', 'nim', ?2)"#,
            params![base_url, WS],
        )?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn validates_and_persists_normalized_base_url_on_connect() {
    let db = two_workspaces().await;
    let id = service::connect(
        &db,
        &AuditService::disabled(),
        WS,
        ConnectInput {
            name: "NIM".into(),
            preset: "nim".into(),
            base_url: " https://example.com/v1 ".into(),
            default_model: "model-1".into(),
            api_key: "secret".into(),
            ..ConnectInput::default()
        },
        false,
        &FakeDoh::new(),
    )
    .await
    .expect("connect");
    assert_eq!(stored_base_url(&db, id).await, "https://example.com/v1");
}

#[tokio::test]
async fn validates_changed_base_url_on_update_and_persists_normalized_value() {
    let db = seeded("https://old.example.com/v1").await;
    let doh = FakeDoh::new();
    let data = UpdateInput {
        base_url: Some(" https://new.example.com/v1 ".into()),
        ..UpdateInput::default()
    };
    service::update(&db, &AuditService::disabled(), WS, 81, data, true, &doh)
        .await
        .expect("update");
    assert!(doh.calls() > 0);
    assert_eq!(stored_base_url(&db, 81).await, "https://new.example.com/v1");
}

#[tokio::test]
async fn does_not_run_environment_validation_when_normalized_base_url_is_unchanged() {
    let db = seeded("https://example.com/v1").await;
    let doh = FakeDoh::new();
    let data = UpdateInput {
        base_url: Some(" https://example.com/v1 ".into()),
        ..UpdateInput::default()
    };
    service::update(&db, &AuditService::disabled(), WS, 81, data, true, &doh)
        .await
        .expect("update");
    assert_eq!(doh.calls(), 0);
    assert_eq!(stored_base_url(&db, 81).await, "https://example.com/v1");
}
