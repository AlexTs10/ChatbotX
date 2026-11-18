//! `apps/worker` template queue: `installTemplate` (Q.D4 slice 3),
//! `syncTemplateInstallation` and `fanOutTemplateUpdate` (slice 4) and
//! `uninstallTemplate` (slice 5).

use chatbotx_business::template::{install::INSTALL_LEASE_MS, request, syncs, uninstall};
use chatbotx_db::repo::{now_ms, template_installation, template_installation_sync};
use chatbotx_jobs::{JobCtx, JobError};
use serde_json::Value;

use crate::{Deps, id_of};

/// The `template` queue processor.
pub async fn handle(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = crate::str_of(&job.data, "type").unwrap_or("").to_string();
    let data = &job.data["data"];
    let bad = || JobError::unrecoverable(format!("{kind}: bad data"));
    let workspace_id = id_of(&data["workspaceId"]).ok_or_else(bad)?;
    let failed = |e: chatbotx_business::errors::BusinessError| crate::fail(e.to_string());
    match kind.as_str() {
        "installTemplate" => {
            let installation_id = id_of(&data["installationId"]).ok_or_else(bad)?;
            if crate::is_blocked_workspace(&deps.db, &workspace_id.to_string()).await? {
                // `failBlockedTemplateInstall`.
                chatbotx_business::tx::write(&deps.db, move |conn| {
                    Ok(template_installation::fail_unclaimed(
                        conn,
                        installation_id,
                        "workspaceOwnerBlocked",
                        "The workspace owner is blocked",
                        now_ms() - INSTALL_LEASE_MS,
                    )?)
                })
                .await
                .map_err(failed)?;
                return Ok(Value::Null);
            }
            request::run_install_job(&deps.db, workspace_id, installation_id, &deps.app_url)
                .await
                .map_err(failed)?;
        }
        "syncTemplateInstallation" => {
            let sync_id = id_of(&data["syncId"]).ok_or_else(bad)?;
            if crate::is_blocked_workspace(&deps.db, &workspace_id.to_string()).await? {
                // `skipBlockedTemplateSync`.
                chatbotx_business::tx::write(&deps.db, move |conn| {
                    Ok(template_installation_sync::close_unclaimed(
                        conn,
                        sync_id,
                        "skipped",
                        "workspaceOwnerBlocked",
                        Some(now_ms() - syncs::SYNC_LEASE_MS),
                    )?)
                })
                .await
                .map_err(failed)?;
                return Ok(Value::Null);
            }
            syncs::run_sync_job(&deps.db, workspace_id, sync_id, &deps.app_url)
                .await
                .map_err(failed)?;
        }
        "fanOutTemplateUpdate" => {
            let template_id = id_of(&data["templateId"]).ok_or_else(bad)?;
            let version = data["version"].as_i64().ok_or_else(bad)?;
            syncs::fan_out(
                &deps.db,
                Some(deps.jobs.as_ref()),
                template_id,
                workspace_id,
                version,
                id_of(&data["afterId"]),
            )
            .await
            .map_err(failed)?;
        }
        "uninstallTemplate" => {
            let installation_id = id_of(&data["installationId"]).ok_or_else(bad)?;
            if crate::is_blocked_workspace(&deps.db, &workspace_id.to_string()).await? {
                uninstall::skip_blocked(&deps.db, installation_id)
                    .await
                    .map_err(failed)?;
                return Ok(Value::Null);
            }
            uninstall::run_uninstall_job(&deps.db, workspace_id, installation_id)
                .await
                .map_err(failed)?;
        }
        other => {
            return Err(JobError::unrecoverable(format!(
                "TemplateJobAction action is not defined: {other}"
            )));
        }
    }
    Ok(Value::Null)
}
