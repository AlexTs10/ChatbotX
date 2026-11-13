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
