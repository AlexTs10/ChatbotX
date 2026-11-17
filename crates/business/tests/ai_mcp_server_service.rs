//! Ports of `packages/business/__tests__/ai-mcp-server.service.test.ts`,
//! `ai-mcp-endpoint-policy.test.ts`, `packages/ai/__tests__/mcp-client.test.ts`
//! and `apps/builder/__tests__/validate-ai-mcp-server.action.test.ts` (Q.D6).
//! Adapted: the mocked drizzle client becomes a real migrated database, the
//! audit spy a recording enqueuer, the `checkSsrfSafety` mock a fake DoH
//! client, the `assertSafeMcpEndpoint` mock a fake [`EndpointGuard`].
//! Not applicable: the four template-lock tests (no template locks / guards,
//! P3.6-D1 rule).

mod common;
mod x1_support;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chatbotx_business::BusinessError;
use chatbotx_business::ai::mcp_client::{BoxFuture, EndpointGuard, McpAuth, McpClient};
use chatbotx_business::ai_mcp_server::{
    self as service, AiMcpServerInput, assert_safe_mcp_endpoint_with,
};
use chatbotx_business::audit::{AuditActor, AuditService, with_audit_context};
use chatbotx_business::net::ssrf_guard::{DohFetch, DohReply};
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, WS, scalar_i64, setup, two_workspaces};
use serde_json::{Map, json};
use x1_support::RecordingEnqueuer;

fn request() -> AiMcpServerInput {
    AiMcpServerInput {
        name: "Docs MCP".into(),
        url: "https://example.com/mcp".into(),
        auth: json!({ "type": "none" }),
        available_tools: Map::new(),
        selected_tools: vec![],
    }
}

fn audit() -> (AuditService, Arc<RecordingEnqueuer>) {
    let jobs = RecordingEnqueuer::new();
    (AuditService::new(jobs.clone()), jobs)
}

fn actor() -> AuditActor {
    AuditActor {
        user_id: Some("1001".into()),
        workspace_id: Some(WS.to_string()),
        ..AuditActor::default()
    }
}

fn last_audit_detail(jobs: &RecordingEnqueuer) -> Option<String> {
    jobs.calls()
        .last()
        .and_then(|call| call.data["data"]["detail"].as_str().map(str::to_string))
}

/// Server 5 "Old MCP" of `WS`.
async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "AIMCPServer" ("id", "name", "url", "auth", "availableTools", "selectedTools", "workspaceId")
               VALUES (5, 'Old MCP', 'https://old.example.com/mcp', '{"type":"none"}', '{}', '[]', ?1)"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    db
}

// describe("aiMcpServerService audit messages")

#[tokio::test]
async fn create_logs_by_id() {
    let db = two_workspaces().await;
    let (audit, jobs) = audit();
    let created = with_audit_context(actor(), service::create(&db, &audit, WS, request()))
        .await
        .expect("create");
    assert_eq!(
        last_audit_detail(&jobs),
        Some(format!("created a new MCP Server (#{})", created.id))
    );
}

#[tokio::test]
async fn update_logs_by_id() {
    let db = seeded().await;
    let (audit, jobs) = audit();
    with_audit_context(actor(), service::update(&db, &audit, WS, 5, request()))
        .await
        .expect("update");
    assert_eq!(
        last_audit_detail(&jobs).as_deref(),
        Some("updated an MCP Server (#5)")
    );
}

#[tokio::test]
async fn delete_logs_by_id() {
    let db = seeded().await;
    let (audit, jobs) = audit();
    with_audit_context(actor(), service::delete(&db, &audit, WS, 5))
        .await
        .expect("delete");
    assert_eq!(
        last_audit_detail(&jobs).as_deref(),
        Some("deleted an MCP Server (#5)")
    );
}

#[tokio::test]
async fn update_does_not_audit_when_the_id_does_not_exist() {
    let db = seeded().await;
    let (audit, jobs) = audit();
    let out = with_audit_context(actor(), service::update(&db, &audit, WS, 99, request()))
        .await
        .expect("update");
    assert!(out.is_none());
    assert!(jobs.calls().is_empty());
}

#[tokio::test]
async fn delete_does_not_audit_when_the_id_does_not_exist() {
    let db = seeded().await;
    let (audit, jobs) = audit();
    let out = with_audit_context(actor(), service::delete(&db, &audit, WS, 99))
        .await
        .expect("delete");
    assert!(out.is_empty());
    assert!(jobs.calls().is_empty());
}

// describe("aiMcpServerService cross-workspace isolation")

#[tokio::test]
async fn update_scopes_its_where_clause_to_the_requesting_workspace_not_just_the_id() {
    let db = seeded().await;
    let (audit, jobs) = audit();
    let out = with_audit_context(
        actor(),
        service::update(&db, &audit, OTHER_WS, 5, request()),
    )
    .await
    .expect("update");
    assert!(out.is_none());
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "AIMCPServer" WHERE "id" = ?1 AND "name" = 'Old MCP'"#,
            5
        )
        .await,
        Some(1)
    );
    assert!(jobs.calls().is_empty());
}

#[tokio::test]
async fn delete_scopes_its_where_clause_to_the_requesting_workspace_not_just_the_id() {
    let db = seeded().await;
    let (audit, jobs) = audit();
    let out = with_audit_context(actor(), service::delete(&db, &audit, OTHER_WS, 5))
        .await
        .expect("delete");
    assert!(out.is_empty());
    assert_eq!(
        scalar_i64(
            &db,
            r#"SELECT count(*) FROM "AIMCPServer" WHERE "id" = ?1"#,
            5
        )
        .await,
        Some(1)
    );
    assert!(jobs.calls().is_empty());
}

// ---- ai-mcp-endpoint-policy.test.ts ----

/// `checkSsrfSafety` mock: an A record (public or private) or a failure.
struct FakeDoh {
    ip: Option<&'static str>,
    calls: AtomicUsize,
}

impl FakeDoh {
    fn new(ip: Option<&'static str>) -> FakeDoh {
        FakeDoh {
            ip,
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
        let ip = self.ip.ok_or("network down")?;
        Ok(DohReply {
            status: 200,
            body: json!({ "Status": 0, "Answer": [{ "type": 1, "data": ip }] }).to_string(),
        })
    }
}

fn is_rejected(result: Result<(), BusinessError>) -> bool {
    matches!(result, Err(e) if e.code() == Some("invalidMcpServerEndpoint") && e.status() == Some(400))
}

// describe("MCP endpoint policy on cloud")

#[tokio::test]
async fn accepts_an_endpoint_the_shared_ssrf_guard_considers_public() {
    let doh = FakeDoh::new(Some("93.184.216.34"));
    assert!(
        assert_safe_mcp_endpoint_with("https://mcp.example.com/mcp", true, &doh)
            .await
            .is_ok()
    );
    assert!(doh.calls() > 0);
}

#[tokio::test]
async fn rejects_endpoints_the_shared_ssrf_guard_marks_unsafe() {
    let doh = FakeDoh::new(Some("10.0.0.8"));
    assert!(is_rejected(
        assert_safe_mcp_endpoint_with("https://internal.example.com/mcp", true, &doh).await
    ));
}

#[tokio::test]
async fn rejects_an_unparseable_url_without_consulting_the_guard() {
    let doh = FakeDoh::new(Some("93.184.216.34"));
    assert!(is_rejected(
        assert_safe_mcp_endpoint_with("not a url", true, &doh).await
    ));
    assert_eq!(doh.calls(), 0);
}

#[tokio::test]
async fn rejects_reserved_hostname_before_resolving_it() {
    for url in [
        "http://metadata.google.internal/mcp",
        "https://metadata.google.internal./mcp",
        "https://api.localhost/mcp",
    ] {
        let doh = FakeDoh::new(Some("93.184.216.34"));
        assert!(
            is_rejected(assert_safe_mcp_endpoint_with(url, true, &doh).await),
            "{url}"
        );
        assert_eq!(doh.calls(), 0, "{url}");
    }
}

#[tokio::test]
async fn requires_https() {
    let doh = FakeDoh::new(Some("93.184.216.34"));
    assert!(is_rejected(
        assert_safe_mcp_endpoint_with("http://mcp.example.com/mcp", true, &doh).await
    ));
    assert_eq!(doh.calls(), 0);
}

// describe("MCP endpoint policy on self-hosted editions")

#[tokio::test]
async fn accepts_without_consulting_the_ssrf_guard() {
    for url in [
        "http://mcp-tools:8080/mcp",
        "http://10.0.0.5/mcp",
        "https://mcp.internal.example/mcp",
    ] {
        let doh = FakeDoh::new(None);
        assert!(
            assert_safe_mcp_endpoint_with(url, false, &doh)
                .await
                .is_ok(),
            "{url}"
        );
        assert_eq!(doh.calls(), 0, "{url}");
    }
}

#[tokio::test]
async fn rejects_on_self_hosted() {
    for url in ["ftp://mcp.example.com/mcp", "not a url"] {
        let doh = FakeDoh::new(None);
        assert!(
            is_rejected(assert_safe_mcp_endpoint_with(url, false, &doh).await),
            "{url}"
        );
        assert_eq!(doh.calls(), 0, "{url}");
    }
}

// ---- mcp-client.test.ts ----

/// `assertSafeMcpEndpoint` mock: refuses or passes, and counts calls.
struct FakeGuard {
    refuse: bool,
    calls: AtomicUsize,
    urls: std::sync::Mutex<Vec<String>>,
}

impl FakeGuard {
    fn new(refuse: bool) -> Arc<FakeGuard> {
        Arc::new(FakeGuard {
            refuse,
            calls: AtomicUsize::new(0),
            urls: std::sync::Mutex::new(Vec::new()),
        })
    }
}

impl EndpointGuard for FakeGuard {
    fn check<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<(), BusinessError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.urls.lock().unwrap().push(url.to_string());
        let refuse = self.refuse;
        Box::pin(async move {
            if refuse {
                Err(BusinessError::exception(
                    "blocked",
                    "invalidMcpServerEndpoint",
                    400,
                ))
            } else {
                Ok(())
            }
        })
    }
}

fn token() -> McpAuth {
    McpAuth::Token("secret".into())
}

#[tokio::test]
async fn rejects_unsafe_endpoints_before_sending_a_request() {
    let guard = FakeGuard::new(true);
    let mut client = McpClient::new("http://127.0.0.1:3000/mcp", &token(), None, guard.clone());
    let error = client.initialize().await.expect_err("blocked");
    assert_eq!(error.message(), "blocked");
    assert_eq!(*guard.urls.lock().unwrap(), ["http://127.0.0.1:3000/mcp"]);
}

#[tokio::test]
async fn validates_the_endpoint_once_across_consecutive_requests() {
    let guard = FakeGuard::new(false);
    // Port 9 (discard) on loopback: the request fails at the network layer.
    let mut client = McpClient::new("http://127.0.0.1:9/mcp", &token(), None, guard.clone());
    let _ = client.initialize().await;
    let _ = client.initialize().await;
    assert_eq!(guard.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn re_validates_after_a_rejected_check_instead_of_caching_the_failure() {
    let guard = FakeGuard::new(true);
    let mut client = McpClient::new("https://mcp.example.com/mcp", &token(), None, guard.clone());
    assert_eq!(
        client.initialize().await.expect_err("1").message(),
        "blocked"
    );
    assert_eq!(
        client.initialize().await.expect_err("2").message(),
        "blocked"
    );
    assert_eq!(guard.calls.load(Ordering::SeqCst), 2);
}

// ---- validate-ai-mcp-server.action.test.ts ----

#[tokio::test]
async fn does_not_resolve_or_send_an_authorization_header_for_a_blocked_endpoint() {
    let db = two_workspaces().await;
    let guard = FakeGuard::new(true);
    let error = service::validate(
        &db,
        guard.clone(),
        WS,
        "http://127.0.0.1:3000/mcp",
        // A bot-field template that would fail to resolve (no such field):
        // the guard refuses first, so it is never resolved.
        &json!({ "type": "token", "token": "{{bot_field:42}}" }),
    )
    .await
    .expect_err("blocked");
    assert_eq!(error.message(), "blocked");
    assert_eq!(guard.calls.load(Ordering::SeqCst), 1);
}
