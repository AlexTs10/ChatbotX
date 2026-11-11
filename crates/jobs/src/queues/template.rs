//! The `template` queue (`queues/template/index.ts`).

use serde_json::{Value, json};

use crate::enqueue::{Enqueuer, JobsError};
use crate::options::{JobOptions, Retention};
use crate::queue::QueueName;
use crate::store::{JobInspector, JobState};

/// `TemplateJobPriority.interactive` (work a person waits on).
pub const TEMPLATE_PRIORITY_INTERACTIVE: u32 = 1;
/// `TemplateJobPriority.background` (syncs a save fans out).
pub const TEMPLATE_PRIORITY_BACKGROUND: u32 = 10;

/// `TemplateJobAction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TemplateJobAction {
    InstallTemplate,
    SyncTemplateInstallation,
    UninstallTemplate,
    FanOutTemplateUpdate,
}

impl TemplateJobAction {
    /// Every template action (`Object.values(TemplateJobAction)`).
    pub fn all() -> &'static [TemplateJobAction] {
        &[
            TemplateJobAction::InstallTemplate,
            TemplateJobAction::SyncTemplateInstallation,
            TemplateJobAction::UninstallTemplate,
            TemplateJobAction::FanOutTemplateUpdate,
        ]
    }

    /// `"installTemplate"`, `"syncTemplateInstallation"`, `"uninstallTemplate"`, `"fanOutTemplateUpdate"`.
    pub fn as_str(self) -> &'static str {
        match self {
            TemplateJobAction::InstallTemplate => "installTemplate",
            TemplateJobAction::SyncTemplateInstallation => "syncTemplateInstallation",
            TemplateJobAction::UninstallTemplate => "uninstallTemplate",
            TemplateJobAction::FanOutTemplateUpdate => "fanOutTemplateUpdate",
        }
    }
}

/// Who waits for the job: `"interactive"` or `"background"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateWork {
    Interactive,
    Background,
}

impl TemplateWork {
    /// [`TEMPLATE_PRIORITY_INTERACTIVE`] or [`TEMPLATE_PRIORITY_BACKGROUND`].
    pub fn priority(self) -> u32 {
        match self {
            TemplateWork::Interactive => TEMPLATE_PRIORITY_INTERACTIVE,
            TemplateWork::Background => TEMPLATE_PRIORITY_BACKGROUND,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallTemplateData {
    pub installation_id: String,
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncTemplateInstallationData {
    pub sync_id: String,
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UninstallTemplateData {
    pub installation_id: String,
    pub workspace_id: String,
    pub attempt: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FanOutTemplateUpdateData {
    pub template_id: String,
    pub workspace_id: String,
    pub version: u32,
    /// The page cursor; `None` = first page (`-first`).
    pub after_id: Option<String>,
}

/// `enqueueInstallTemplate`: `install-template-<installationId>`, interactive.
pub async fn enqueue_install_template<Q>(
    queue: &Q,
    data: InstallTemplateData,
) -> Result<(), JobsError>
where
    Q: Enqueuer + JobInspector + ?Sized,
{
    let job_id = format!("install-template-{}", data.installation_id);
    let payload = json!({
        "installationId": data.installation_id,
        "workspaceId": data.workspace_id,
    });
    add_template_job(
        queue,
        TemplateJobAction::InstallTemplate,
        payload,
        job_id,
        TemplateWork::Interactive,
    )
    .await
}

/// `enqueueSyncTemplateInstallation`: `tpl-sync-<syncId>`. An interactive
/// request for a job still prioritized as background raises it once
/// (`change_priority`); waiting, active or already raised jobs are left
/// alone; a job that became active during the change is not an error.
pub async fn enqueue_sync_template_installation<Q>(
    queue: &Q,
    data: SyncTemplateInstallationData,
    work: TemplateWork,
) -> Result<(), JobsError>
where
    Q: Enqueuer + JobInspector + ?Sized,
{
    let job_id = format!("tpl-sync-{}", data.sync_id);
    let payload = json!({ "syncId": data.sync_id, "workspaceId": data.workspace_id });
    add_template_job(
        queue,
        TemplateJobAction::SyncTemplateInstallation,
        payload,
        job_id.clone(),
        work,
    )
    .await?;
    if work == TemplateWork::Interactive {
        promote_queued_job(queue, &job_id).await?;
    }
    Ok(())
}

/// `enqueueUninstallTemplate`: `tpl-uninstall-<installationId>-<attempt>`, interactive.
pub async fn enqueue_uninstall_template<Q>(
    queue: &Q,
    data: UninstallTemplateData,
) -> Result<(), JobsError>
where
    Q: Enqueuer + JobInspector + ?Sized,
{
    // The attempt in the id keeps a retry from being swallowed.
    let job_id = format!("tpl-uninstall-{}-{}", data.installation_id, data.attempt);
    let payload = json!({
        "installationId": data.installation_id,
        "workspaceId": data.workspace_id,
        "attempt": data.attempt,
    });
    add_template_job(
        queue,
        TemplateJobAction::UninstallTemplate,
        payload,
        job_id,
        TemplateWork::Interactive,
    )
    .await
}

/// `enqueueFanOutTemplateUpdate`: `tpl-fanout-<templateId>-<version>-<afterId|first>`,
/// raised like a sync.
pub async fn enqueue_fan_out_template_update<Q>(
    queue: &Q,
    data: FanOutTemplateUpdateData,
    work: TemplateWork,
) -> Result<(), JobsError>
where
    Q: Enqueuer + JobInspector + ?Sized,
{
    let job_id = format!(
        "tpl-fanout-{}-{}-{}",
        data.template_id,
        data.version,
        data.after_id.as_deref().unwrap_or("first")
    );
    let mut payload = json!({
        "templateId": data.template_id,
        "workspaceId": data.workspace_id,
        "version": data.version,
    });
    if let Some(after) = &data.after_id {
        payload["afterId"] = Value::String(after.clone());
    }
    add_template_job(
        queue,
        TemplateJobAction::FanOutTemplateUpdate,
        payload,
        job_id.clone(),
        work,
    )
    .await?;
    if work == TemplateWork::Interactive {
        promote_queued_job(queue, &job_id).await?;
    }
    Ok(())
}

/// Adds one template job. The row is the outbox, so finished jobs are not
/// kept (a retained job would make the sweep's next add a no-op).
async fn add_template_job<Q>(
    queue: &Q,
    action: TemplateJobAction,
    data: Value,
    job_id: String,
    work: TemplateWork,
) -> Result<(), JobsError>
where
    Q: Enqueuer + JobInspector + ?Sized,
{
    let name = action.as_str();
    queue
        .add(
            QueueName::Template,
            name,
            json!({ "type": name, "data": data }),
            JobOptions {
                job_id: Some(job_id),
                priority: Some(work.priority()),
                remove_on_complete: Some(Retention::Remove),
                remove_on_fail: Some(Retention::Remove),
                ..JobOptions::default()
            },
        )
        .await
        .map(|_| ())
}

/// `promoteQueuedJob`: raises a job still prioritized as background work.
/// A `waiting` job already runs before every prioritized job; a started job
/// has nothing to promote.
async fn promote_queued_job<Q>(queue: &Q, job_id: &str) -> Result<(), JobsError>
where
    Q: Enqueuer + JobInspector + ?Sized,
{
    let Some(job) = queue.get_job(QueueName::Template, job_id).await? else {
        return Ok(());
    };
    if job.priority == TEMPLATE_PRIORITY_INTERACTIVE {
        return Ok(());
    }
    if queue.get_state(QueueName::Template, job_id).await? != JobState::Prioritized {
        return Ok(());
    }
    match queue
        .change_priority(QueueName::Template, job_id, TEMPLATE_PRIORITY_INTERACTIVE)
        .await
    {
        Ok(_) => Ok(()),
        Err(err) => {
            // Picked up between the state read and the change: nothing to promote.
            if queue.get_state(QueueName::Template, job_id).await? == JobState::Prioritized {
                Err(err)
            } else {
                Ok(())
            }
        }
    }
}
