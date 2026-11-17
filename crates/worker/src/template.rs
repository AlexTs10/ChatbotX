//! `apps/worker` template queue: `installTemplate` (Q.D4 slice 3). The sync,
//! fan-out and uninstall jobs fail as "not defined" until their slices.

use chatbotx_jobs::{JobCtx, JobError};
use serde_json::Value;

use crate::{Deps, id_of};

/// The `template` queue processor.
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = crate::str_of(&job.data, "type").unwrap_or("");
    let data = &job.data["data"];
    let (Some(workspace_id), installation_id) =
        (id_of(&data["workspaceId"]), id_of(&data["installationId"]))
    else {
        return Err(JobError::unrecoverable(format!("{kind}: bad data")));
    };
    match (kind, installation_id) {
        ("installTemplate", Some(installation_id)) => {
            if crate::is_blocked_workspace(&deps.db, &workspace_id.to_string()).await? {
                // `failBlockedTemplateInstall`.
                let db = deps.db.clone();
                chatbotx_business::tx::write(&db, move |conn| {
                    Ok(chatbotx_db::repo::template_installation::fail_unclaimed(
                        conn,
                        installation_id,
                        "workspaceOwnerBlocked",
                        "The workspace owner is blocked",
                        chatbotx_db::repo::now_ms()
                            - chatbotx_business::template::install::INSTALL_LEASE_MS,
                    )?)
                })
                .await
                .map_err(|e| crate::fail(e.to_string()))?;
                return Ok(Value::Null);
            }
            chatbotx_business::template::request::run_install_job(
                &deps.db,
                workspace_id,
                installation_id,
                &deps.app_url,
            )
            .await
            .map_err(|e| crate::fail(e.to_string()))?;
            Ok(Value::Null)
        }
        (other, _) => Err(JobError::unrecoverable(format!(
            "TemplateJobAction action is not defined: {other}"
        ))),
    }
}
