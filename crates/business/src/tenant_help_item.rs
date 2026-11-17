//! `enterprise/tenant-help-item/service.ts` (Q.D11 slice 10): the help menu
//! links of a tenant. No list cache (no Redis caches in Rust).

use chatbotx_db::Db;
use chatbotx_db::repo::models::TenantHelpItem;
use chatbotx_db::repo::{self, new_id, now_ms};
use chatbotx_db::rusqlite::params;

use crate::{BusinessError, Result, tx};

/// `HelpItemData` (the parsed `helpItemSchema`: defaults applied).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpItemData {
    pub name: String,
    pub url: String,
    pub icon: Option<String>,
    pub position: i64,
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Help item not found")
}

/// `listByTenant`: by position, then id.
pub async fn list_by_tenant(db: &Db, tenant_id: i64) -> Result<Vec<TenantHelpItem>> {
    tx::read(db, move |c| {
        Ok(repo::find_many::<TenantHelpItem>(
            c,
            "WHERE \"tenantId\" = ?1 ORDER BY \"position\" ASC, \"id\" ASC",
            &[&tenant_id],
        )?)
    })
    .await
}

fn find(c: &chatbotx_db::rusqlite::Connection, id: i64) -> Result<TenantHelpItem> {
    repo::find_one::<TenantHelpItem>(c, "\"id\" = ?1", &[&id])?.ok_or_else(not_found)
}

/// `create(tenantId, data)`.
pub async fn create(db: &Db, tenant_id: i64, data: HelpItemData) -> Result<TenantHelpItem> {
    tx::write(db, move |c| {
        let id = new_id();
        c.execute(
            r#"INSERT INTO "TenantHelpItem" ("id", "createdAt", "updatedAt", "tenantId", "name", "url", "icon", "position")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![id, now_ms(), tenant_id, data.name, data.url, data.icon, data.position],
        )?;
        find(c, id)
    })
    .await
}

/// `update(id, tenantId, data)`: 404 outside the tenant.
pub async fn update(
    db: &Db,
    id: i64,
    tenant_id: i64,
    data: HelpItemData,
) -> Result<TenantHelpItem> {
    tx::write(db, move |c| {
        let changed = c.execute(
            r#"UPDATE "TenantHelpItem" SET "name" = ?3, "url" = ?4, "icon" = ?5, "position" = ?6, "updatedAt" = ?7
               WHERE "id" = ?1 AND "tenantId" = ?2"#,
            params![id, tenant_id, data.name, data.url, data.icon, data.position, now_ms()],
        )?;
        if changed == 0 {
            return Err(not_found());
        }
        find(c, id)
    })
    .await
}

/// `remove(id, tenantId)`: 404 outside the tenant.
pub async fn remove(db: &Db, id: i64, tenant_id: i64) -> Result<()> {
    tx::write(db, move |c| {
        let deleted = c.execute(
            r#"DELETE FROM "TenantHelpItem" WHERE "id" = ?1 AND "tenantId" = ?2"#,
            params![id, tenant_id],
        )?;
        if deleted == 0 {
            return Err(not_found());
        }
        Ok(())
    })
    .await
}
