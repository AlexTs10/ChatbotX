//! The `template` queue (`queues/template/index.ts`).

use crate::enqueue::{Enqueuer, JobsError};
use crate::store::JobInspector;

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
        todo!()
    }

    /// `"installTemplate"`, `"syncTemplateInstallation"`, `"uninstallTemplate"`, `"fanOutTemplateUpdate"`.
    pub fn as_str(self) -> &'static str {
        todo!()
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
        todo!()
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
    todo!()
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
    todo!()
}

/// `enqueueUninstallTemplate`: `tpl-uninstall-<installationId>-<attempt>`, interactive.
pub async fn enqueue_uninstall_template<Q>(
    queue: &Q,
    data: UninstallTemplateData,
) -> Result<(), JobsError>
where
    Q: Enqueuer + JobInspector + ?Sized,
{
    todo!()
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
    todo!()
}
