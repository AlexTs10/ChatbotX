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
    match kind {
        "importMetaCatalogProducts" => meta_catalog::import::handle(&deps, data).await?,
        "submitMetaCatalogSync" => meta_catalog::sync::submit(&deps, data).await?,
        "checkMetaCatalogSync" => meta_catalog::sync::check(&deps, data).await?,
        REVOKE_RETRY_JOB_NAME => retry_realtime_member_revoke(&deps, &job, data).await?,
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
