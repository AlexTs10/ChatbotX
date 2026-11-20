//! The token, profile and avatar jobs: `updateContactAvatar` (`low` and
//! `integration`), the `profileSnapshot` queue (`capture`), and the
//! `refreshChannelTokens` / `dispatchProfileSnapshots` schedules.

use chatbotx_business::audit::{AuditActor, AuditService, with_audit_context};
use chatbotx_business::channel_api::LiveChannelHttp;
use chatbotx_business::channel_token_refresh::{RefreshDeps, refresh_scheduled};
use chatbotx_business::contact::channel_profile::{ProfileDeps, ensure_contact_avatar_mirrored};
use chatbotx_business::contact::profile_snapshot::{self, SnapshotTarget};
use chatbotx_jobs::{JobCtx, JobError};
use serde_json::Value;

use crate::{Deps, fail, id_of, is_blocked_workspace, str_of};

/// `LowJobAction.updateContactAvatar` / `IntegrationJobAction.updateContactAvatar`.
pub const UPDATE_CONTACT_AVATAR: &str = "updateContactAvatar";

/// `updateContactAvatar`: `ensureContactAvatarMirrored` (never fails).
pub(crate) async fn update_contact_avatar(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let workspace = str_of(data, "workspaceId").unwrap_or("");
    if is_blocked_workspace(&deps.db, workspace).await? {
        return Ok(());
    }
    let (Some(workspace_id), Some(contact_inbox_id)) =
        (id_of(&data["workspaceId"]), id_of(&data["contactInboxId"]))
    else {
        return Err(JobError::unrecoverable("updateContactAvatar: invalid ids"));
    };
    let deps = ProfileDeps {
        db: &deps.db,
        http: &LiveChannelHttp,
        uploader: deps.files.as_deref(),
    };
    ensure_contact_avatar_mirrored(deps, workspace_id, contact_inbox_id).await;
    Ok(())
}

/// The `profileSnapshot` queue processor (`capture`), behind the
/// blocked-owner guard and the `profile-snapshot:capture` audit source.
pub async fn handle_profile_snapshot(deps: Deps, job: JobCtx) -> Result<Value, JobError> {
    let kind = str_of(&job.data, "type").unwrap_or("").to_owned();
    if kind != profile_snapshot::CAPTURE {
        return Err(JobError::unrecoverable(format!(
            "Unhandled profile snapshot job type: {kind}"
        )));
    }
    let data = &job.data["data"];
    let workspace = str_of(data, "workspaceId").unwrap_or("").to_owned();
    if is_blocked_workspace(&deps.db, &workspace).await? {
        return Ok(Value::Null);
    }
    let (Some(contact_inbox_id), Some(inbox_id), Some(workspace_id)) = (
        id_of(&data["contactInboxId"]),
        id_of(&data["inboxId"]),
        id_of(&data["workspaceId"]),
    ) else {
        return Err(JobError::unrecoverable("profile snapshot: invalid ids"));
    };
    let target = SnapshotTarget {
        contact_inbox_id,
        inbox_id,
        workspace_id,
    };
    let actor = AuditActor {
        workspace_id: Some(workspace),
        source: Some(format!("profile-snapshot:{kind}")),
        ..AuditActor::default()
    };
    with_audit_context(
        actor,
        profile_snapshot::capture(&deps.db, &LiveChannelHttp, target),
    )
    .await
    .map_err(|e| fail(e.to_string()))?;
    Ok(Value::Null)
}

/// `dispatchProfileSnapshots`.
pub(crate) async fn dispatch_profile_snapshots(deps: &Deps) -> Result<(), JobError> {
    let kv = crate::inbound::kv(deps);
    profile_snapshot::dispatch(&deps.db, &*deps.jobs, &kv)
        .await
        .map_err(|e| fail(e.to_string()))
}

/// `refreshChannelTokens` (`data.channels`: the channels to refresh; absent
/// = all).
pub(crate) async fn refresh_channel_tokens(deps: &Deps, data: &Value) {
    let channels: Option<Vec<String>> = data["channels"].as_array().map(|list| {
        list.iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    });
    let audit = AuditService::new(deps.jobs.clone());
    let refresh = RefreshDeps {
        db: &deps.db,
        http: &LiveChannelHttp,
        audit: &audit,
        buses: deps.events.as_ref(),
    };
    refresh_scheduled(refresh, channels.as_deref()).await;
}
