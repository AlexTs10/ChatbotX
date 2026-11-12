//! Port of `repositories/tenant/` (`repository.ts` and `default-template-repository.ts`,
//! P3.3).

use rusqlite::Connection;

use super::{Json, Result};

/// The columns `findById` / `findByOwner` read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantSummary {
    pub id: i64,
    pub owner_id: Option<i64>,
    pub status: String,
}

/// `findById`.
pub fn find_by_id(conn: &Connection, tenant_id: i64) -> Result<Option<TenantSummary>> {
    let _ = (conn, tenant_id);
    todo!("P3.3")
}

/// `findByOwner`.
pub fn find_by_owner(conn: &Connection, owner_id: i64) -> Result<Option<TenantSummary>> {
    let _ = (conn, owner_id);
    todo!("P3.3")
}

/// `defaultTemplateColumns()`.
#[derive(Debug, Clone, PartialEq)]
pub struct DefaultTemplate {
    pub id: i64,
    pub tenant_id: i64,
    pub name: String,
    pub workspace_id: i64,
    pub format_version: i64,
    pub version: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    pub share_enabled: bool,
    pub share_expires_at: Option<i64>,
    pub share_token: String,
    pub default_permissions: Json,
    pub default_auto_update: bool,
}

/// Row of [`find_with_template`].
#[derive(Debug, Clone, PartialEq)]
pub struct TenantDefaultTemplateWithTemplate {
    pub key_hash: String,
    pub encrypted_key: Json,
    pub template: DefaultTemplate,
}

/// Input of [`upsert_default_template`].
#[derive(Debug, Clone, PartialEq)]
pub struct UpsertDefaultTemplateInput {
    pub tenant_id: i64,
    pub template_id: i64,
    pub key_hash: String,
    pub encrypted_key: Json,
}

/// `tenantDefaultTemplateRepository.findWithTemplate`.
pub fn find_with_template(
    conn: &Connection,
    tenant_id: i64,
) -> Result<Option<TenantDefaultTemplateWithTemplate>> {
    let _ = (conn, tenant_id);
    todo!("P3.3")
}

/// `tenantDefaultTemplateRepository.upsert` (one row per tenant).
pub fn upsert_default_template(
    conn: &Connection,
    input: &UpsertDefaultTemplateInput,
) -> Result<()> {
    let _ = (conn, input);
    todo!("P3.3")
}

/// `tenantDefaultTemplateRepository.delete`: the removed `templateId`.
pub fn delete_default_template(conn: &Connection, tenant_id: i64) -> Result<Option<i64>> {
    let _ = (conn, tenant_id);
    todo!("P3.3")
}

/// `tenantDefaultTemplateRepository.recordEvent`.
pub fn record_default_template_event(
    conn: &Connection,
    tenant_id: i64,
    action: &str,
    template_id: i64,
    actor_user_id: i64,
) -> Result<()> {
    let _ = (conn, tenant_id, action, template_id, actor_user_id);
    todo!("P3.3")
}
