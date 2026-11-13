//! Port of `packages/business/__tests__/audit.service.test.ts` (P3.6-X1):
//! the audit actor context and `auditService.record`. The `defaultQueue.add`
//! mock is a recording enqueuer, `createId` a fixed id source, the logger
//! spy a recording tracing layer.

mod x1_support;

use std::sync::Arc;

use chatbotx_business::audit::{
    AuditActor, AuditRecordInput, AuditService, audit_actor, with_audit_context,
    with_audit_context_sync,
};
use chatbotx_jobs::{JobOptions, QueueName};
use serde_json::json;
use tracing::Level;
use x1_support::{RecordingEnqueuer, RecordingLog};

const AUDIT_LOG_ID: &str = "11643703873355776";

fn service(jobs: &Arc<RecordingEnqueuer>) -> AuditService {
    AuditService::new(Arc::clone(jobs) as Arc<dyn chatbotx_jobs::Enqueuer>)
        .with_ids(Arc::new(|| AUDIT_LOG_ID.to_string()))
}

fn actor(user_id: Option<&str>, workspace_id: Option<&str>) -> AuditActor {
    AuditActor {
        user_id: user_id.map(str::to_string),
        workspace_id: workspace_id.map(str::to_string),
        ..AuditActor::default()
    }
}

fn job_options() -> JobOptions {
    JobOptions {
        job_id: Some(format!("audit-log-{AUDIT_LOG_ID}")),
        ..JobOptions::default()
    }
}

// audit context > nests workspace context without dropping request actor details
#[test]
fn nests_workspace_context_without_dropping_request_actor_details() {
    let request = AuditActor {
        user_id: Some("user-1".into()),
        ip_address: Some("203.0.113.10".into()),
        user_agent: Some("Vitest".into()),
        ..AuditActor::default()
    };
    with_audit_context_sync(request.clone(), || {
        assert_eq!(audit_actor(), Some(request.clone()));
        let nested = AuditActor {
            workspace_id: Some("workspace-1".into()),
            ..audit_actor().unwrap_or_default()
        };
        with_audit_context_sync(nested, || {
            assert_eq!(
                audit_actor(),
                Some(AuditActor {
                    user_id: Some("user-1".into()),
                    workspace_id: Some("workspace-1".into()),
                    ip_address: Some("203.0.113.10".into()),
                    user_agent: Some("Vitest".into()),
                    source: None,
                })
            );
        });
        assert_eq!(audit_actor(), Some(request.clone()));
    });
    assert_eq!(audit_actor(), None);
}

// audit context > keeps concurrent actors isolated
#[tokio::test]
async fn keeps_concurrent_actors_isolated() {
    let (first, second) = tokio::join!(
        with_audit_context(actor(Some("user-1"), None), async { audit_actor() }),
        with_audit_context(actor(Some("user-2"), None), async { audit_actor() }),
    );
    assert_eq!(first, Some(actor(Some("user-1"), None)));
    assert_eq!(second, Some(actor(Some("user-2"), None)));
}

// audit service > records an audit job from ambient context
#[tokio::test]
async fn records_an_audit_job_from_ambient_context() {
    let jobs = RecordingEnqueuer::new();
    let audit = service(&jobs);
    let ambient = AuditActor {
        user_id: Some("user-1".into()),
        workspace_id: Some("workspace-1".into()),
        ip_address: Some("203.0.113.10".into()),
        user_agent: Some("Vitest".into()),
        source: None,
    };
    with_audit_context(
        ambient,
        audit.record(AuditRecordInput::new("create", "Created thing")),
    )
    .await;
    let calls = jobs.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].queue, QueueName::Default);
    assert_eq!(calls[0].name, "sendAuditLog");
    assert_eq!(
        calls[0].data,
        json!({
            "type": "sendAuditLog",
            "data": {
                "auditLogId": AUDIT_LOG_ID,
                "userId": "user-1",
                "workspaceId": "workspace-1",
                "action": "create",
                "detail": "Created thing",
                "ipAddress": "203.0.113.10",
                "userAgent": "Vitest",
            },
        })
    );
    assert_eq!(calls[0].opts, job_options());
}

// audit service > lets explicit input override ambient actor fields
#[tokio::test]
async fn lets_explicit_input_override_ambient_actor_fields() {
    let jobs = RecordingEnqueuer::new();
    let audit = service(&jobs);
    let ambient = AuditActor {
        user_id: Some("ambient-user".into()),
        workspace_id: Some("ambient-workspace".into()),
        ip_address: Some("203.0.113.10".into()),
        user_agent: Some("Ambient".into()),
        source: None,
    };
    let input = AuditRecordInput {
        action: "export".into(),
        detail: "Exported contacts".into(),
        user_id: Some("user-2".into()),
        workspace_id: Some("workspace-2".into()),
        ip_address: Some("198.51.100.8".into()),
        user_agent: Some("Override".into()),
        source: Some("export-contacts".into()),
    };
    with_audit_context(ambient, audit.record(input)).await;
    let calls = jobs.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "sendAuditLog");
    assert_eq!(
        calls[0].data,
        json!({
            "type": "sendAuditLog",
            "data": {
                "auditLogId": AUDIT_LOG_ID,
                "userId": "user-2",
                "workspaceId": "workspace-2",
                "action": "export",
                "detail": "Exported contacts",
                "ipAddress": "198.51.100.8",
                "userAgent": "Override",
                "source": "export-contacts",
            },
        })
    );
    assert_eq!(calls[0].opts, job_options());
}

// audit service > does not enqueue when actor or workspace is missing
#[tokio::test]
async fn does_not_enqueue_when_actor_or_workspace_is_missing() {
    let jobs = RecordingEnqueuer::new();
    let audit = service(&jobs);
    audit
        .record(AuditRecordInput::new("update", "No context"))
        .await;
    with_audit_context(
        actor(Some("user-1"), None),
        audit.record(AuditRecordInput::new("update", "No workspace")),
    )
    .await;
    assert!(jobs.calls().is_empty());
}

// audit service > swallows enqueue failures and logs a warning
#[tokio::test]
async fn swallows_enqueue_failures_and_logs_a_warning() {
    let log = RecordingLog::install();
    let jobs = RecordingEnqueuer::new();
    jobs.fail_next("redis unavailable");
    let audit = service(&jobs);
    audit
        .record(AuditRecordInput {
            user_id: Some("user-1".into()),
            workspace_id: Some("workspace-1".into()),
            ..AuditRecordInput::new("delete", "Deleted thing")
        })
        .await;
    let entry = log
        .find("audit log enqueue failed")
        .expect("the enqueue failure is logged");
    assert_eq!(entry.level, Level::WARN);
    assert_eq!(entry.field("err"), Some("redis unavailable"));
    assert_eq!(entry.field("workspace_id"), Some("workspace-1"));
    assert_eq!(entry.field("user_id"), Some("user-1"));
    assert_eq!(entry.field("action"), Some("delete"));
}

// audit service > BaseService.audit delegates using only ambient context
#[tokio::test]
async fn base_service_audit_delegates_using_only_ambient_context() {
    let jobs = RecordingEnqueuer::new();
    let audit = service(&jobs);
    with_audit_context(
        actor(Some("user-1"), Some("workspace-1")),
        audit.audit("update", "Test detail"),
    )
    .await;
    let calls = jobs.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].data,
        json!({
            "type": "sendAuditLog",
            "data": {
                "auditLogId": AUDIT_LOG_ID,
                "userId": "user-1",
                "workspaceId": "workspace-1",
                "action": "update",
                "detail": "Test detail",
            },
        })
    );
    assert_eq!(calls[0].opts, job_options());
}
