//! `apps/worker/src/default/worker.ts`: the `default` queue processor.

use chatbotx_jobs::{JobCtx, JobError};
use serde_json::Value;

use crate::{Deps, str_of};

/// `startDefaultWorker` switch.
pub async fn handle(_deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = str_of(&job.data, "type").unwrap_or("");
    // ponytail: the Meta catalog import/submit/check jobs are the next
    // Q.W1 slice; the other default jobs are later units. An unported
    // action fails (kept for a retry) instead of the original's
    // "Unknown job name" completion, so no job is lost.
    Err(JobError::unrecoverable(format!(
        "DefaultJobAction action is not defined: {kind}"
    )))
}
