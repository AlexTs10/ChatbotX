//! `apps/worker/src/default/worker.ts`: the `default` queue processor.

use std::sync::Arc;

use chatbotx_jobs::{Backoff, Enqueuer, JobCtx, JobError, JobOptions, QueueName, Retention};
use chatbotx_rt::revoke::{
    REVOKE_RETRY_JOB_ATTEMPTS, REVOKE_RETRY_JOB_DELAY_MS, REVOKE_RETRY_JOB_NAME,
    revoke_retry_job_payload,
};
use chatbotx_rt::{BoxError, BoxFuture, RevokeArgs, RevokeReason, RevokeRetry};
use serde_json::Value;

use crate::{Deps, fail, meta_catalog, str_of};

/// `startDefaultWorker` switch.
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = str_of(&job.data, "type").unwrap_or("");
    let data = &job.data["data"];
    // `withBlockedOwnerGuard(resolveWorkspaceId(job.data.data))`.
    if crate::is_blocked_workspace(&deps.db, str_of(data, "workspaceId").unwrap_or("")).await? {
        tracing::info!(
            job_type = kind,
            "Skipping workspace job for frozen workspace"
        );
        if kind == "processInboundWebhookRequest" {
            crate::inbound_webhook::skip_blocked_request(&deps, data).await?;
        }
        if kind == PROCESS_PIPELINE_STAGE_BULK
            && let Some(job) = StageBulkJob::of(data)
        {
            chatbotx_business::pipeline::structure::release_stage_bulk_claim(
                &deps.db,
                job.workspace_id,
                job.pipeline_id,
                job.stage_id,
                job.bulk_job_id,
            )
            .await
            .map_err(|e| JobError::retryable(e.to_string()))?;
        }
        return Ok(Value::Null);
    }
    match kind {
        crate::leaderboard::EXPORT_PARTICIPANTS => {
            crate::leaderboard::export_participants(&deps, data).await?
        }
        "importMetaCatalogProducts" => meta_catalog::import::handle(&deps, data).await?,
        "submitMetaCatalogSync" => meta_catalog::sync::submit(&deps, data).await?,
        "checkMetaCatalogSync" => meta_catalog::sync::check(&deps, data).await?,
        "sendAppointmentReminder" => {
            return crate::appointment::send_reminder(&deps, data).await;
        }
        "syncExternalCalendarEvent" => {
            return crate::appointment::sync_external(&deps, &job, data).await;
        }
        "processInboundWebhookRequest" => {
            crate::inbound_webhook::process_request(&deps, &job, data).await?;
        }
        REVOKE_RETRY_JOB_NAME => retry_realtime_member_revoke(&deps, &job, data).await?,
        PROCESS_PIPELINE_STAGE_BULK => {
            let Some(job) = StageBulkJob::of(data) else {
                return Err(JobError::unrecoverable(
                    "processPipelineStageBulk: bad data",
                ));
            };
            let actor = chatbotx_business::pipeline::mutation::PipelineActor {
                kind: str_of(data, "actorKind").unwrap_or("system").to_owned(),
                user_id: str_of(data, "actorUserId").and_then(|id| id.parse().ok()),
                source_id: str_of(data, "actorSourceId").map(str::to_owned),
            };
            chatbotx_business::pipeline::structure::process_queued_stage_bulk(
                &deps.db,
                job.workspace_id,
                job.pipeline_id,
                job.stage_id,
                job.bulk_job_id,
                str_of(data, "moveItemsToStageId").and_then(|id| id.parse().ok()),
                actor,
            )
            .await
            .map_err(|e| JobError::retryable(e.to_string()))?
        }
        // ponytail: the other default jobs are later units. An unported
        // action fails (kept for a retry) instead of the original's
        // "Unknown job name" completion, so no job is lost.
        other => {
            return Err(JobError::unrecoverable(format!(
                "DefaultJobAction action is not defined: {other}"
            )));
        }
    }
    Ok(Value::Null)
}

/// `DefaultJobAction.processPipelineStageBulk`.
pub const PROCESS_PIPELINE_STAGE_BULK: &str = "processPipelineStageBulk";

/// The stage claim of a `processPipelineStageBulk` job.
struct StageBulkJob {
    workspace_id: i64,
    pipeline_id: i64,
    stage_id: i64,
    bulk_job_id: String,
}

impl StageBulkJob {
    fn of(data: &Value) -> Option<StageBulkJob> {
        let id = |key: &str| str_of(data, key).and_then(|v| v.parse::<i64>().ok());
        Some(StageBulkJob {
            workspace_id: id("workspaceId")?,
            pipeline_id: id("pipelineId")?,
            stage_id: id("stageId")?,
            bulk_job_id: str_of(data, "bulkJobId")?.to_owned(),
        })
    }
}

/// `retryRealtimeMemberRevoke`: the revoke again; the last failed attempt
/// is logged as exhausted.
async fn retry_realtime_member_revoke(
    deps: &Deps,
    job: &JobCtx,
    data: &Value,
) -> Result<(), JobError> {
    let (Some(workspace_id), Some(user_id)) = (str_of(data, "workspaceId"), str_of(data, "userId"))
    else {
        return Err(JobError::unrecoverable(
            "retryRealtimeMemberRevoke: bad data",
        ));
    };
    let Some(realtime) = &deps.realtime else {
        // No gateway in this process: no socket to close.
        return Ok(());
    };
    let args = RevokeArgs {
        workspace_id: workspace_id.to_owned(),
        user_id: user_id.to_owned(),
        reason: if str_of(data, "reason") == Some("deleted") {
            RevokeReason::Deleted
        } else {
            RevokeReason::Reauth
        },
        revoked_at_ms: data.get("revokedAtMs").and_then(Value::as_u64),
    };
    realtime.revoke_member(args).await.map_err(|err| {
        if job.is_final_attempt() {
            tracing::error!(err = %err, user_id, workspace_id, "Realtime member connection revoke retry exhausted");
        }
        fail(err.to_string())
    })
}

/// `enqueueWorkspaceMemberRealtimeConnectionRevocationRetry`: the durable
/// retry on the `default` queue (8 attempts, exponential 1 s).
pub struct JobsRevokeRetry(pub Arc<dyn Enqueuer>);

impl RevokeRetry for JobsRevokeRetry {
    fn enqueue<'a>(&'a self, args: &'a RevokeArgs) -> BoxFuture<'a, Result<(), BoxError>> {
        Box::pin(async move {
            let opts = JobOptions {
                attempts: Some(REVOKE_RETRY_JOB_ATTEMPTS),
                backoff: Some(Backoff::Exponential(REVOKE_RETRY_JOB_DELAY_MS)),
                remove_on_complete: Some(Retention::Remove),
                ..JobOptions::default()
            };
            self.0
                .add(
                    QueueName::Default,
                    REVOKE_RETRY_JOB_NAME,
                    revoke_retry_job_payload(args),
                    opts,
                )
                .await
                .map(|_| ())
                .map_err(|err| Box::new(err) as BoxError)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chatbotx_jobs::{AddResult, JobsError, NewJob};
    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct Recording(Mutex<Vec<(QueueName, String, Value, JobOptions)>>);

    impl Enqueuer for Recording {
        fn add(
            &self,
            queue: QueueName,
            name: &str,
            data: Value,
            opts: JobOptions,
        ) -> chatbotx_jobs::BoxFuture<'_, Result<AddResult, JobsError>> {
            self.0
                .lock()
                .unwrap()
                .push((queue, name.into(), data, opts));
            Box::pin(async {
                Ok(AddResult {
                    job_id: "1".into(),
                    created: true,
                })
            })
        }

        fn add_bulk(
            &self,
            _: QueueName,
            _: Vec<NewJob>,
        ) -> chatbotx_jobs::BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    /// `default-worker.test.ts` "skips member revocations for blocked
    /// workspaces" / "skips bulk tag contact jobs for frozen workspaces": a
    /// blocked workspace completes the job without running it.
    #[tokio::test]
    async fn blocked_workspaces_skip_the_job() {
        let f = crate::steps::test_support::Fixture::new().await;
        let job = |ws: &str| {
            crate::steps::test_support::job(
                json!({ "type": "bulkTagContacts", "data": { "workspaceId": ws } }),
            )
        };
        assert!(
            handle(f.deps.clone(), job("2")).await.is_err(),
            "an active workspace runs (unported: fails)"
        );
        f.exec(r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = 2"#)
            .await;
        assert_eq!(handle(f.deps.clone(), job("2")).await.unwrap(), Value::Null);
        assert_eq!(
            handle(f.deps.clone(), job("999")).await.unwrap(),
            Value::Null
        );
    }

    #[tokio::test]
    async fn revoke_retry_queues_the_original_job() {
        let jobs = Arc::new(Recording::default());
        let args = RevokeArgs {
            workspace_id: "7".into(),
            user_id: "9".into(),
            reason: RevokeReason::Deleted,
            revoked_at_ms: Some(42),
        };
        JobsRevokeRetry(jobs.clone()).enqueue(&args).await.unwrap();
        let calls = jobs.0.lock().unwrap();
        let (queue, name, data, opts) = &calls[0];
        assert_eq!(
            (*queue, name.as_str()),
            (QueueName::Default, "retryRealtimeMemberRevoke")
        );
        assert_eq!(
            data,
            &json!({ "type": "retryRealtimeMemberRevoke", "data": {
                "reason": "deleted", "revokedAtMs": 42, "userId": "9", "workspaceId": "7" } })
        );
        assert_eq!(opts.attempts, Some(8));
        assert_eq!(opts.backoff, Some(Backoff::Exponential(1000)));
        assert_eq!(opts.remove_on_complete, Some(Retention::Remove));
    }
}
