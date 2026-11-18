//! `template/install-intent.ts` (K35 of P2.7: durable `take`) and
//! `stageSharedTemplateInstall` (`installation-stage.ts`): "this user is
//! creating a workspace from this template's share page", so that template
//! takes the place of the tenant's default in the new workspace.

use chatbotx_db::repo::{
    now_ms, template as templates, template_installation as installs, template_version,
};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_kv::DurableKv;

use super::request::PUBLISHER_UNINSTALL_DISCLOSURE_VERSION;
use crate::errors::Result;

/// `INSTALL_INTENT_TTL_SECONDS`.
const TTL_SECONDS: u64 = 300;

fn key(user_id: i64) -> String {
    format!("template-install-intent:{user_id}")
}

/// `remember`: one slot per user, a newer choice replaces an older one. Only
/// a convenience: a store failure is logged, never raised.
pub async fn remember(kv: &DurableKv, user_id: i64, share_token: &str) {
    if let Err(error) = kv.put(&key(user_id), share_token, Some(TTL_SECONDS)).await {
        tracing::warn!(%error, "Could not remember the template install intent");
    }
}

/// `take`: reads and deletes in one step (of two creations at once, one gets it).
pub async fn take(kv: &DurableKv, user_id: i64) -> Option<String> {
    kv.take::<String>(&key(user_id))
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "Could not read the template install intent");
            None
        })
}

/// `clear`.
pub async fn clear(kv: &DurableKv, user_id: i64) {
    take(kv, user_id).await;
}

/// `stageSharedTemplateInstall`: inside a new workspace's creation
/// transaction, the `pending` installation of the share link's template;
/// `None` when the link no longer works here (the tenant default applies).
/// The creator owns the new workspace, so every category is theirs.
pub fn stage_shared_install(
    conn: &Connection,
    workspace_id: i64,
    tenant_id: i64,
    created_by: i64,
    share_token: &str,
) -> Result<Option<i64>> {
    let template = templates::find_live_by_share_token(conn, share_token)?.filter(|t| {
        t.share_enabled
            && t.share_expires_at.is_none_or(|at| at > now_ms())
            && (t.tenant_id == tenant_id
                || t.tenant_id == chatbotx_db::repo::platform_admin::ROOT_TENANT_ID)
    });
    let Some(template) = template else {
        tracing::warn!(tenant_id, workspace_id, "Skipped the share link's template");
        return Ok(None);
    };
    let version = template_version::pin_version_number(conn, template.id, template.version)?;
    // Across tenants a publisher never gets access.
    let grant = if template.tenant_id == tenant_id {
        PUBLISHER_UNINSTALL_DISCLOSURE_VERSION
    } else {
        0
    };
    let row = installs::create(
        conn,
        &vec![
            ("workspaceId", Sql::Integer(workspace_id)),
            ("templateId", Sql::Integer(template.id)),
            ("sourceTemplateId", Sql::Integer(template.id)),
            ("templateName", Sql::Text(template.name.clone())),
            ("sourceWorkspaceId", Sql::Integer(template.workspace_id)),
            ("formatVersion", Sql::Integer(template.format_version)),
            ("status", Sql::Text("pending".into())),
            (
                "permissions",
                Sql::Text(template.default_permissions.0.to_string()),
            ),
            (
                "autoUpdate",
                Sql::Integer(i64::from(template.default_auto_update)),
            ),
            ("sourceUpdatedAt", Sql::Integer(template.updated_at)),
            ("sourceVersion", Sql::Integer(version)),
            (
                "publisherRef",
                Sql::Text(chatbotx_db::repo::random_uuid().replace('-', "")),
            ),
            ("publisherUninstallGrant", Sql::Integer(grant)),
            ("installedBy", Sql::Integer(created_by)),
            ("installedByKind", Sql::Text("member".into())),
            ("installSource", Sql::Text("manual".into())),
        ],
    )?;
    Ok(Some(row.id))
}

/// `dispatchStagedInstalls`: after a workspace's creation commits, enqueues
/// the installs staged for it (a share link's template or the tenant
/// default). Best effort: the installation sweep recovers a lost enqueue, so
/// a failure never fails the caller.
pub async fn dispatch_staged_installs(
    db: &chatbotx_db::Db,
    jobs: &dyn chatbotx_jobs::Enqueuer,
    workspace_id: i64,
) {
    let pending = crate::tx::read(db, move |conn| {
        Ok(installs::list_pending_ids(conn, workspace_id)?)
    })
    .await;
    let pending = match pending {
        Ok(pending) => pending,
        Err(error) => {
            tracing::error!(%error, workspace_id, "Failed to dispatch staged template installs");
            return;
        }
    };
    for id in pending {
        if let Err(error) = super::save::enqueue_install(jobs, id, workspace_id).await {
            tracing::error!(%error, id, workspace_id, "Failed to dispatch a staged template install");
        }
    }
}
