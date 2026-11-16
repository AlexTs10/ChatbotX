//! `apps/worker/src/default/worker.ts`: the `default` queue processor.

use chatbotx_jobs::{JobCtx, JobError};
use serde_json::Value;

use crate::{Deps, meta_catalog, str_of};

/// `startDefaultWorker` switch.
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = str_of(&job.data, "type").unwrap_or("");
    let data = &job.data["data"];
    match kind {
        "importMetaCatalogProducts" => meta_catalog::import::handle(&deps, data).await?,
        "submitMetaCatalogSync" => meta_catalog::sync::submit(&deps, data).await?,
        "checkMetaCatalogSync" => meta_catalog::sync::check(&deps, data).await?,
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
