//! Member revoke (spec 1.5, 2.1, 3.5): the durable marker
//! `realtime:revoked:<ws>:<user>` and the durable retry job.

use std::fmt;

use chatbotx_kv::{BoxFuture, DurableKv};
use serde_json::{Value, json};

use crate::BoxError;
pub use crate::protocol::RevokeReason;
use crate::protocol::member_revoked_key;
use crate::token::MEMBER_REVOKED_TTL_SECS;

/// `REVOKE_RETRY_ATTEMPTS` (marker + close, linear backoff).
pub const REVOKE_RETRY_ATTEMPTS: u32 = 3;
/// `REVOKE_RETRY_DELAY_MS`: attempt `k` waits `k * 250` ms before `k + 1`.
pub const REVOKE_RETRY_DELAY_MS: u64 = 250;
/// `DefaultJobAction.retryRealtimeMemberRevoke`.
pub const REVOKE_RETRY_JOB_NAME: &str = "retryRealtimeMemberRevoke";
/// `REVOKE_RETRY_JOB_ATTEMPTS`.
pub const REVOKE_RETRY_JOB_ATTEMPTS: u32 = 8;
/// `REVOKE_RETRY_JOB_DELAY_MS` (exponential backoff base).
pub const REVOKE_RETRY_JOB_DELAY_MS: u64 = 1000;

/// `WorkspaceMemberRealtimeConnectionRevocationArgs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevokeArgs {
    pub workspace_id: String,
    pub user_id: String,
    pub reason: RevokeReason,
    /// Default: now.
    pub revoked_at_ms: Option<u64>,
}

/// Where the revoke marker lives (production: [`DurableKv`]).
pub trait RevokeStore: Send + Sync {
    /// The raw marker value, `None` when absent.
    fn read_marker<'a>(
        &'a self,
        workspace_id: &'a str,
        user_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, BoxError>>;

    /// Raises the marker to `revoked_at_ms` (never lowers it), TTL 65 s.
    fn raise_marker<'a>(
        &'a self,
        workspace_id: &'a str,
        user_id: &'a str,
        revoked_at_ms: u64,
    ) -> BoxFuture<'a, Result<(), BoxError>>;
}

impl RevokeStore for DurableKv {
    fn read_marker<'a>(
        &'a self,
        workspace_id: &'a str,
        user_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, BoxError>> {
        Box::pin(async move {
            let key = member_revoked_key(workspace_id, user_id);
            let value: Option<Value> = self.get(&key).await?;
            Ok(value.map(|value| match value {
                Value::String(text) => text,
                other => other.to_string(),
            }))
        })
    }

    fn raise_marker<'a>(
        &'a self,
        workspace_id: &'a str,
        user_id: &'a str,
        revoked_at_ms: u64,
    ) -> BoxFuture<'a, Result<(), BoxError>> {
        Box::pin(async move {
            let key = member_revoked_key(workspace_id, user_id);
            let value = i64::try_from(revoked_at_ms).unwrap_or(i64::MAX);
            self.raise_number(&key, value, MEMBER_REVOKED_TTL_SECS)
                .await?;
            Ok(())
        })
    }
}

/// The durable retry of a failed revoke (jobs: `retryRealtimeMemberRevoke`,
/// 8 attempts, exponential 1 000 ms). `args.revoked_at_ms` is always set.
pub trait RevokeRetry: Send + Sync {
    fn enqueue<'a>(&'a self, args: &'a RevokeArgs) -> BoxFuture<'a, Result<(), BoxError>>;
}

/// A retry target that only logs (until the jobs wiring of Phase 3).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoRevokeRetry;

impl RevokeRetry for NoRevokeRetry {
    fn enqueue<'a>(&'a self, args: &'a RevokeArgs) -> BoxFuture<'a, Result<(), BoxError>> {
        Box::pin(async move {
            tracing::warn!(
                workspace_id = %args.workspace_id,
                user_id = %args.user_id,
                "No durable realtime revoke retry is wired"
            );
            Ok(())
        })
    }
}

/// The job data of `retryRealtimeMemberRevoke`:
/// `{"type":..,"data":{"reason","revokedAtMs","userId","workspaceId"}}`.
pub fn revoke_retry_job_payload(args: &RevokeArgs) -> Value {
    json!({
        "type": REVOKE_RETRY_JOB_NAME,
        "data": {
            "reason": args.reason.as_str(),
            "revokedAtMs": args.revoked_at_ms,
            "userId": args.user_id,
            "workspaceId": args.workspace_id,
        },
    })
}

/// A revoke that failed after its retry budget.
#[derive(Debug)]
pub struct RevokeError {
    pub source: BoxError,
}

impl fmt::Display for RevokeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "realtime member revoke failed: {}", self.source)
    }
}

impl std::error::Error for RevokeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}
