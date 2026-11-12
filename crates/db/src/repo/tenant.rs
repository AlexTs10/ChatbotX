//! Port of `repositories/tenant/` (`repository.ts` and `default-template-repository.ts`,
//! P3.3).

use rusqlite::{Connection, OptionalExtension, Row};

use super::{Json, Result, new_id, now_ms};

/// The columns `findById` / `findByOwner` read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantSummary {
    pub id: i64,
    pub owner_id: Option<i64>,
    pub status: String,
}

/// `findById`.
pub fn find_by_id(conn: &Connection, tenant_id: i64) -> Result<Option<TenantSummary>> {
    summary(conn, "\"id\" = ?1", tenant_id)
}

/// `findByOwner`.
pub fn find_by_owner(conn: &Connection, owner_id: i64) -> Result<Option<TenantSummary>> {
    summary(conn, "\"ownerId\" = ?1", owner_id)
}

fn summary(conn: &Connection, filter: &str, value: i64) -> Result<Option<TenantSummary>> {
    let sql =
        format!("SELECT \"id\", \"ownerId\", \"status\" FROM \"Tenant\" WHERE {filter} LIMIT 1");
    let mut stmt = conn.prepare_cached(&sql)?;
    Ok(stmt
        .query_row([value], |row| {
            Ok(TenantSummary {
                id: row.get(0)?,
                owner_id: row.get(1)?,
                status: row.get(2)?,
            })
        })
        .optional()?)
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
    let mut stmt = conn.prepare_cached(
        "SELECT d.\"keyHash\", d.\"encryptedKey\", t.\"id\", t.\"tenantId\", t.\"name\", t.\"workspaceId\", \
                t.\"formatVersion\", t.\"version\", t.\"updatedAt\", t.\"deletedAt\", t.\"shareEnabled\", \
                t.\"shareExpiresAt\", t.\"shareToken\", t.\"defaultPermissions\", t.\"defaultAutoUpdate\" \
         FROM \"TenantDefaultTemplate\" d INNER JOIN \"Template\" t ON t.\"id\" = d.\"templateId\" \
         WHERE d.\"tenantId\" = ?1 LIMIT 1",
    )?;
    Ok(stmt.query_row([tenant_id], with_template).optional()?)
}

fn with_template(row: &Row<'_>) -> rusqlite::Result<TenantDefaultTemplateWithTemplate> {
    Ok(TenantDefaultTemplateWithTemplate {
        key_hash: row.get(0)?,
        encrypted_key: row.get(1)?,
        template: DefaultTemplate {
            id: row.get(2)?,
            tenant_id: row.get(3)?,
            name: row.get(4)?,
            workspace_id: row.get(5)?,
            format_version: row.get(6)?,
            version: row.get(7)?,
            updated_at: row.get(8)?,
            deleted_at: row.get(9)?,
            share_enabled: row.get(10)?,
            share_expires_at: row.get(11)?,
            share_token: row.get(12)?,
            default_permissions: row.get(13)?,
            default_auto_update: row.get(14)?,
        },
    })
}

/// `tenantDefaultTemplateRepository.upsert` (one row per tenant).
pub fn upsert_default_template(
    conn: &Connection,
    input: &UpsertDefaultTemplateInput,
) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO \"TenantDefaultTemplate\" (\"tenantId\", \"templateId\", \"keyHash\", \"encryptedKey\", \"createdAt\", \"updatedAt\") \
         VALUES (?1, ?2, ?3, ?4, ?5, ?5) \
         ON CONFLICT (\"tenantId\") DO UPDATE SET \"templateId\" = excluded.\"templateId\", \
         \"keyHash\" = excluded.\"keyHash\", \"encryptedKey\" = excluded.\"encryptedKey\", \"updatedAt\" = excluded.\"updatedAt\"",
        rusqlite::params![input.tenant_id, input.template_id, input.key_hash, input.encrypted_key, now],
    )?;
    Ok(())
}

/// `tenantDefaultTemplateRepository.delete`: the removed `templateId`.
pub fn delete_default_template(conn: &Connection, tenant_id: i64) -> Result<Option<i64>> {
    let mut stmt = conn.prepare_cached(
        "DELETE FROM \"TenantDefaultTemplate\" WHERE \"tenantId\" = ?1 RETURNING \"templateId\"",
    )?;
    Ok(stmt.query_row([tenant_id], |row| row.get(0)).optional()?)
}

/// `tenantDefaultTemplateRepository.recordEvent`.
pub fn record_default_template_event(
    conn: &Connection,
    tenant_id: i64,
    action: &str,
    template_id: i64,
    actor_user_id: i64,
) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO \"TenantDefaultTemplateEvent\" (\"id\", \"createdAt\", \"updatedAt\", \"tenantId\", \"action\", \"templateId\", \"actorUserId\") \
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![new_id(), now, tenant_id, action, template_id, actor_user_id],
    )?;
    Ok(())
}
