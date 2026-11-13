//! Audit log writer (`packages/business/src/audit`, P3.6-X1): the actor
//! context (`withAuditContext` / `getAuditActor`, an `AsyncLocalStorage` in
//! the original, a tokio task-local here) and `auditService.record`, which
//! enqueues one `sendAuditLog` job on the `default` queue.

use std::sync::Arc;

use chatbotx_jobs::enqueue::Enqueuer;

/// `SYSTEM_ACTOR`: the audit actor of work no user started.
pub const SYSTEM_ACTOR: &str = "system";

/// `AuditActor`: who the current request or job acts for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditActor {
    /// A user id, or [`SYSTEM_ACTOR`].
    pub user_id: Option<String>,
    pub workspace_id: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub source: Option<String>,
}

/// `createId` of the audit log row (a snowflake string).
pub type IdSource = Arc<dyn Fn() -> String + Send + Sync>;

/// `auditService`. Without an enqueuer (no job store wired) every record is
/// dropped with a debug log.
#[derive(Clone)]
pub struct AuditService {
    jobs: Option<Arc<dyn Enqueuer>>,
    ids: IdSource,
}

impl Default for AuditService {
    fn default() -> AuditService {
        AuditService::disabled()
    }
}

impl AuditService {
    /// Records go to `jobs` (the `default` queue).
    pub fn new(jobs: Arc<dyn Enqueuer>) -> AuditService {
        AuditService {
            jobs: Some(jobs),
            ids: Arc::new(chatbotx_core::utils::id::create_id),
        }
    }

    /// No enqueuer: records are dropped.
    pub fn disabled() -> AuditService {
        AuditService {
            jobs: None,
            ids: Arc::new(chatbotx_core::utils::id::create_id),
        }
    }

    /// Uses `ids` for the audit log ids (tests).
    pub fn with_ids(mut self, ids: IdSource) -> AuditService {
        self.ids = ids;
        self
    }

    /// `true` when records reach a job store.
    pub fn is_enabled(&self) -> bool {
        self.jobs.is_some()
    }
}

tokio::task_local! {
    static AUDIT_ACTOR: AuditActor;
}

/// `withAuditContext(actor, fn)` for a future: `actor` is the ambient actor
/// while `future` runs (nested calls replace it; the outer one comes back).
pub async fn with_audit_context<F: std::future::Future>(actor: AuditActor, future: F) -> F::Output {
    AUDIT_ACTOR.scope(actor, future).await
}

/// `withAuditContext(actor, fn)` for a synchronous callback.
pub fn with_audit_context_sync<T>(actor: AuditActor, callback: impl FnOnce() -> T) -> T {
    let _ = (actor, callback);
    todo!("P3.6-X1")
}

/// `getAuditActor()`: the ambient actor, if any.
pub fn audit_actor() -> Option<AuditActor> {
    todo!("P3.6-X1")
}

/// `AuditRecordInput`: explicit fields win over the ambient actor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditRecordInput {
    pub action: String,
    pub detail: String,
    pub user_id: Option<String>,
    pub workspace_id: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub source: Option<String>,
}

impl AuditRecordInput {
    /// `{ action, detail }` (`BaseService.audit(action, detail)`).
    pub fn new(action: impl Into<String>, detail: impl Into<String>) -> AuditRecordInput {
        AuditRecordInput {
            action: action.into(),
            detail: detail.into(),
            ..AuditRecordInput::default()
        }
    }
}

impl AuditService {
    /// `auditService.record`: without a user or a workspace (explicit or
    /// ambient) the record is dropped with a warning ("audit record dropped:
    /// missing userId or workspaceId"); else one `sendAuditLog` job on the
    /// `default` queue, job id `audit-log-<id>`. An enqueue failure is logged
    /// ("audit log enqueue failed") and swallowed.
    pub async fn record(&self, input: AuditRecordInput) {
        let _ = (&self.jobs, &self.ids, input);
        todo!("P3.6-X1")
    }

    /// `BaseService.audit(action, detail)`: [`AuditService::record`] with the
    /// ambient actor only.
    pub async fn audit(&self, action: &str, detail: &str) {
        self.record(AuditRecordInput::new(action, detail)).await;
    }
}
