//! Audit log writer (`packages/business/src/audit`, P3.6-X1): the actor
//! context (`withAuditContext` / `getAuditActor`, an `AsyncLocalStorage` in
//! the original, a tokio task-local here) and `auditService.record`, which
//! enqueues one `sendAuditLog` job on the `default` queue.

use std::sync::Arc;

use chatbotx_db::Db;
use chatbotx_jobs::enqueue::Enqueuer;
use chatbotx_jobs::{JobOptions, QueueName};
use serde_json::{Map, Value, json};

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

/// Where records go.
#[derive(Clone)]
enum Sink {
    /// Dropped (no job store wired).
    Off,
    /// One `sendAuditLog` job on the `default` queue (the original).
    Jobs(Arc<dyn Enqueuer>),
    /// The `sendAuditLog` handler run in the request: one `"AuditLog"` row;
    /// `write_rows: false` (community edition) writes nothing.
    Direct { db: Arc<Db>, write_rows: bool },
}

/// `auditService`.
#[derive(Clone)]
pub struct AuditService {
    sink: Sink,
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
            sink: Sink::Jobs(jobs),
            ids: Arc::new(chatbotx_core::utils::id::create_id),
        }
    }

    /// Records are written to `"AuditLog"` in the request (the
    /// `sendAuditLog` job handler inline, P3.6-X1 "Decisions"); on the
    /// community edition (`write_rows: false`) nothing is written, as the
    /// handler of the original.
    pub fn direct(db: Arc<Db>, write_rows: bool) -> AuditService {
        AuditService {
            sink: Sink::Direct { db, write_rows },
            ids: Arc::new(chatbotx_core::utils::id::create_id),
        }
    }

    /// No enqueuer: records are dropped.
    pub fn disabled() -> AuditService {
        AuditService {
            sink: Sink::Off,
            ids: Arc::new(chatbotx_core::utils::id::create_id),
        }
    }

    /// Uses `ids` for the audit log ids (tests).
    pub fn with_ids(mut self, ids: IdSource) -> AuditService {
        self.ids = ids;
        self
    }

    /// `true` when records go somewhere.
    pub fn is_enabled(&self) -> bool {
        !matches!(self.sink, Sink::Off)
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
    AUDIT_ACTOR.sync_scope(actor, callback)
}

/// `getAuditActor()`: the ambient actor, if any.
pub fn audit_actor() -> Option<AuditActor> {
    AUDIT_ACTOR.try_with(Clone::clone).ok()
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
        let actor = audit_actor().unwrap_or_default();
        let user_id = input.user_id.or(actor.user_id);
        let workspace_id = input.workspace_id.or(actor.workspace_id);
        let (Some(user_id), Some(workspace_id)) = (
            user_id.filter(|id| !id.is_empty()),
            workspace_id.filter(|id| !id.is_empty()),
        ) else {
            tracing::warn!(
                action = %input.action,
                source = ?input.source,
                "audit record dropped: missing userId or workspaceId"
            );
            return;
        };
        let record = SendAuditLog {
            audit_log_id: (self.ids)(),
            user_id,
            workspace_id,
            action: input.action,
            detail: input.detail,
            ip_address: input.ip_address.or(actor.ip_address),
            user_agent: input.user_agent.or(actor.user_agent),
            source: input.source.or(actor.source),
        };
        let outcome = match &self.sink {
            Sink::Off => Ok(()),
            Sink::Jobs(jobs) => {
                let opts = JobOptions {
                    job_id: Some(format!("audit-log-{}", record.audit_log_id)),
                    ..JobOptions::default()
                };
                jobs.add(
                    QueueName::Default,
                    SEND_AUDIT_LOG,
                    json!({ "type": SEND_AUDIT_LOG, "data": record.data() }),
                    opts,
                )
                .await
                .map(|_| ())
                .map_err(|err| err.to_string())
            }
            Sink::Direct { db, write_rows } => {
                if *write_rows {
                    send_audit_log(db, &record).await
                } else {
                    Ok(())
                }
            }
        };
        if let Err(err) = outcome {
            tracing::warn!(
                err = %err,
                workspace_id = %record.workspace_id,
                user_id = %record.user_id,
                action = %record.action,
                source = ?record.source,
                "audit log enqueue failed"
            );
        }
    }

    /// `BaseService.audit(action, detail)`: [`AuditService::record`] with the
    /// ambient actor only.
    pub async fn audit(&self, action: &str, detail: &str) {
        self.record(AuditRecordInput::new(action, detail)).await;
    }
}

/// `DefaultJobAction.sendAuditLog`.
pub const SEND_AUDIT_LOG: &str = "sendAuditLog";

/// `JobSendAuditLog["data"]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendAuditLog {
    pub audit_log_id: String,
    pub user_id: String,
    pub workspace_id: String,
    pub action: String,
    pub detail: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub source: Option<String>,
}

impl SendAuditLog {
    /// The job data (an unset field is left out, as `undefined`).
    pub fn data(&self) -> Value {
        let mut data = Map::new();
        data.insert("auditLogId".into(), Value::from(self.audit_log_id.as_str()));
        data.insert("userId".into(), Value::from(self.user_id.as_str()));
        data.insert(
            "workspaceId".into(),
            Value::from(self.workspace_id.as_str()),
        );
        data.insert("action".into(), Value::from(self.action.as_str()));
        data.insert("detail".into(), Value::from(self.detail.as_str()));
        for (key, value) in [
            ("ipAddress", &self.ip_address),
            ("userAgent", &self.user_agent),
            ("source", &self.source),
        ] {
            if let Some(value) = value {
                data.insert(key.into(), Value::from(value.as_str()));
            }
        }
        Value::Object(data)
    }
}

/// `sendAuditLog` (`apps/worker/src/default/handlers/send-audit-log.ts`,
/// without the edition check): one `"AuditLog"` row, `onConflictDoNothing`;
/// the [`SYSTEM_ACTOR`] user is stored as `NULL`.
pub async fn send_audit_log(db: &Db, record: &SendAuditLog) -> Result<(), String> {
    let id = record
        .audit_log_id
        .parse::<i64>()
        .map_err(|_| format!("invalid audit log id {}", record.audit_log_id))?;
    let workspace_id = record
        .workspace_id
        .parse::<i64>()
        .map_err(|_| format!("invalid workspace id {}", record.workspace_id))?;
    let user_id = if record.user_id == SYSTEM_ACTOR {
        None
    } else {
        Some(
            record
                .user_id
                .parse::<i64>()
                .map_err(|_| format!("invalid user id {}", record.user_id))?,
        )
    };
    let row = record.clone();
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO "AuditLog" ("id", "action", "detail", "ipAddress", "userAgent", "source", "workspaceId", "userId") VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT DO NOTHING"#,
            chatbotx_db::rusqlite::params![
                id,
                row.action,
                row.detail,
                row.ip_address,
                row.user_agent,
                row.source,
                workspace_id,
                user_id
            ],
        )?;
        Ok(())
    })
    .await
    .map_err(|err| err.to_string())
}
