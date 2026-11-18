//! `enterprise/tenant/default-template.service.ts` (`set`, `clear`),
//! `template/default-template-status.ts` and `installation-stage.ts`
//! (`stageTenantDefaultInstall`; Q.D4 slice 6).

use chatbotx_core::encryption::Encryption;
use chatbotx_core::utils::crypto::sha256_hex;
use chatbotx_db::Db;
use chatbotx_db::repo::platform_admin::ROOT_TENANT_ID;
use chatbotx_db::repo::template_installation as installs;
use chatbotx_db::repo::tenant::{
    self, TenantDefaultTemplateWithTemplate, UpsertDefaultTemplateInput,
};
use chatbotx_db::repo::{Json, now_ms, template as templates, template_version};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value as Sql;

use super::request::PUBLISHER_UNINSTALL_DISCLOSURE_VERSION;
use crate::errors::{BusinessError, Result};
use crate::tx;

/// Whose default template (`DefaultTemplateScope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// A reseller's own tenant.
    Owner(i64),
    /// The platform's root tenant.
    Root,
}

pub fn unavailable() -> BusinessError {
    BusinessError::exception(
        "The template ID or key is not valid",
        "tenantDefaultTemplateUnavailable",
        404,
    )
}

pub fn share_disabled() -> BusinessError {
    BusinessError::exception(
        "Turn the template's share link on before using it as a default",
        "tenantDefaultTemplateShareDisabled",
        409,
    )
}

/// `keyAad`: binds the stored key to its tenant.
fn key_aad(tenant_id: i64) -> String {
    format!("tenantDefaultTemplate:{tenant_id}")
}

/// `resolveTenantId`: never taken from client input.
fn tenant_of(conn: &Connection, scope: Scope) -> Result<Option<i64>> {
    Ok(match scope {
        Scope::Root => Some(ROOT_TENANT_ID),
        Scope::Owner(owner) => tenant::find_by_owner(conn, owner)?.map(|t| t.id),
    })
}

/// `resolveDefaultTemplateStatus`.
pub fn status(
    tenant_id: i64,
    config: &TenantDefaultTemplateWithTemplate,
    now: i64,
) -> &'static str {
    let t = &config.template;
    if t.deleted_at.is_some() || t.tenant_id != tenant_id {
        "templateUnavailable"
    } else if !(t.share_enabled && t.share_expires_at.is_none_or(|at| at >= now)) {
        "shareDisabled"
    } else if sha256_hex(&t.share_token) != config.key_hash {
        // A rotated share token no longer matches the stored digest.
        "keyRotated"
    } else {
        "ready"
    }
}

/// `set`: one answer (404 `tenantDefaultTemplateUnavailable`) for a missing,
/// deleted, other-tenant template or a wrong key; 409 when its share is off.
pub async fn set(
    db: &Db,
    encryption: &Encryption,
    scope: Scope,
    template_id: i64,
    template_key: String,
    actor_user_id: i64,
) -> Result<()> {
    let key_hash = sha256_hex(&template_key);
    let tenant_id = tx::read(db, move |conn| tenant_of(conn, scope))
        .await?
        .ok_or_else(unavailable)?;
    let encrypted = encryption
        .encrypt_text(&template_key, Some(&key_aad(tenant_id)))
        .map_err(|e| BusinessError::exception(e.to_string(), "systemError", 500))?;
    let encrypted = serde_json::to_value(encrypted)
        .map_err(|e| BusinessError::exception(e.to_string(), "systemError", 500))?;
    tx::write(db, move |tx| {
        let template = templates::find_by_id_for_tenant(tx, template_id, tenant_id)?
            .filter(|t| t.deleted_at.is_none() && sha256_hex(&t.share_token) == key_hash)
            .ok_or_else(unavailable)?;
        let now = now_ms();
        if !(template.share_enabled && template.share_expires_at.is_none_or(|at| at >= now)) {
            return Err(share_disabled());
        }
        tenant::upsert_default_template(
            tx,
            &UpsertDefaultTemplateInput {
                tenant_id,
                template_id: template.id,
                key_hash,
                encrypted_key: Json(encrypted),
            },
        )?;
        tenant::record_default_template_event(tx, tenant_id, "set", template.id, actor_user_id)?;
        Ok(())
    })
    .await
}

/// `clear`.
pub async fn clear(db: &Db, scope: Scope, actor_user_id: i64) -> Result<()> {
    tx::write(db, move |tx| {
        let Some(tenant_id) = tenant_of(tx, scope)? else {
            return Ok(());
        };
        if let Some(removed) = tenant::delete_default_template(tx, tenant_id)? {
            tenant::record_default_template_event(
                tx,
                tenant_id,
                "cleared",
                removed,
                actor_user_id,
            )?;
        }
        Ok(())
    })
    .await
}

/// `stageTenantDefaultInstall`: inside a new workspace's creation
/// transaction, the `pending` installation of the tenant's default template
/// (the template sweep enqueues it); `None` when there is none or it can no
/// longer install.
pub fn stage_install(conn: &Connection, workspace_id: i64, tenant_id: i64) -> Result<Option<i64>> {
    if tenant::find_by_id(conn, tenant_id)?.is_none_or(|t| t.status != "active") {
        return Ok(None);
    }
    let Some(config) = tenant::find_with_template(conn, tenant_id)? else {
        return Ok(None);
    };
    let ready = status(tenant_id, &config, now_ms());
    let template = &config.template;
    if ready != "ready" {
        tracing::warn!(
            tenant_id,
            workspace_id,
            template_id = template.id,
            status = ready,
            "Skipped the tenant default template"
        );
        return Ok(None);
    }
    let version = template_version::pin_version_number(conn, template.id, template.version)?;
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
            (
                "publisherUninstallGrant",
                Sql::Integer(PUBLISHER_UNINSTALL_DISCLOSURE_VERSION),
            ),
            ("installedByKind", Sql::Text("apiToken".into())),
            ("installSource", Sql::Text("tenantDefault".into())),
        ],
    )?;
    Ok(Some(row.id))
}
