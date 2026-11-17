//! `enterprise/tenant/service.ts` (`tenantService`, Q.D11 slice 10) and the
//! read-only `enterprise/custom-domain/service.ts`: the `Tenant` row
//! (identity, lifecycle, branding) and the `CustomDomain` reads.
//!
//! ponytail: no tenant / custom domain caches (one SQLite process).
//! ponytail: `suspend` flips the status only; `deactivateOwnerWorkspaces`
//! (pause teardown) and the `reactivate` connection / inbox resume sweep
//! belong to the connection lifecycle units.

use chatbotx_core::utils::channel::creatable_channels;
use chatbotx_db::Db;
use chatbotx_db::repo::models::{CustomDomain, Tenant};
use chatbotx_db::repo::{self, now_ms};
use chatbotx_db::rusqlite::types::Value as SqlValue;

use crate::Result;
use crate::tx;

/// `ROOT_TENANT_ID`.
pub const ROOT_TENANT_ID: i64 = 1;

/// `tenantService.findById`.
pub async fn find_by_id(db: &Db, tenant_id: i64) -> Result<Option<Tenant>> {
    tx::read(db, move |c| {
        Ok(repo::find_one::<Tenant>(c, r#""id" = ?1"#, &[&tenant_id])?)
    })
    .await
}

/// `tenantService.findByOwner`.
pub async fn find_by_owner(db: &Db, owner_id: i64) -> Result<Option<Tenant>> {
    tx::read(db, move |c| {
        Ok(repo::find_one::<Tenant>(
            c,
            r#""ownerId" = ?1"#,
            &[&owner_id],
        )?)
    })
    .await
}

fn hidden(tenant: Option<&Tenant>) -> Vec<String> {
    tenant
        .and_then(|t| t.hidden_channels.as_ref())
        .and_then(|j| serde_json::from_value::<Vec<String>>(j.0.clone()).ok())
        .unwrap_or_default()
}

/// `resolveVisibleChannels(ownerId)`: the creatable channels minus the
/// platform's hidden channels and those of the owner's active tenant.
pub async fn resolve_visible_channels(db: &Db, owner_id: i64) -> Result<Vec<&'static str>> {
    let root = find_by_id(db, ROOT_TENANT_ID).await?;
    let owned = find_by_owner(db, owner_id)
        .await?
        .filter(|t| t.status == "active");
    let mut hidden_set = hidden(root.as_ref());
    hidden_set.extend(hidden(owned.as_ref()));
    Ok(creatable_channels()
        .into_iter()
        .map(|c| c.as_str())
        .filter(|c| !hidden_set.iter().any(|h| h == c))
        .collect())
}

/// `provisionForOwner`: the owner's tenant id, inserting one when missing
/// (`Tenant_ownerId_key` keeps it to one per owner).
pub async fn provision_for_owner(db: &Db, owner_id: i64) -> Result<i64> {
    tx::write(db, move |c| {
        let now = now_ms();
        c.execute(
            r#"INSERT INTO "Tenant" ("id", "createdAt", "updatedAt", "ownerId") VALUES (?1, ?2, ?2, ?3)
               ON CONFLICT DO NOTHING"#,
            chatbotx_db::rusqlite::params![repo::new_id(), now, owner_id],
        )?;
        Ok(c.query_row(
            r#"SELECT "id" FROM "Tenant" WHERE "ownerId" = ?1"#,
            [owner_id],
            |r| r.get(0),
        )?)
    })
    .await
}

/// `listActiveOwnerIds`: owners of the active tenants (the root tenant has
/// no owner).
pub async fn list_active_owner_ids(db: &Db) -> Result<Vec<i64>> {
    tx::read(db, |c| {
        let mut stmt = c.prepare(
            r#"SELECT "ownerId" FROM "Tenant" WHERE "status" = 'active' AND "ownerId" IS NOT NULL ORDER BY "id""#,
        )?;
        let ids = stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<Vec<i64>, _>>()?;
        Ok(ids)
    })
    .await
}

/// The `TenantBrandingData` columns a caller may set.
const BRANDING_COLUMNS: &[&str] = &[
    "brandName",
    "customCss",
    "customJs",
    "faviconPath",
    "forgotPasswordEmailTemplate",
    "hiddenChannels",
    "logoDarkPath",
    "logoLightPath",
    "magicLinkEmailTemplate",
    "accountCredentialsEmailTemplate",
    "policyUrl",
    "signupEmailTemplate",
    "status",
    "storageUrl",
    "termsOfServiceUrl",
    "theme",
];

async fn update(
    db: &Db,
    column: &'static str,
    key: i64,
    data: Vec<(&'static str, SqlValue)>,
) -> Result<()> {
    if data.is_empty() {
        return Ok(());
    }
    assert!(
        data.iter().all(|(c, _)| BRANDING_COLUMNS.contains(c)),
        "not a tenant branding column"
    );
    tx::write(db, move |c| {
        let sets: Vec<String> = data
            .iter()
            .enumerate()
            .map(|(i, (col, _))| format!(r#""{col}" = ?{}"#, i + 3))
            .collect();
        let sql = format!(
            r#"UPDATE "Tenant" SET "updatedAt" = ?2, {} WHERE "{column}" = ?1"#,
            sets.join(", ")
        );
        let mut params: Vec<SqlValue> = vec![SqlValue::Integer(key), SqlValue::Integer(now_ms())];
        params.extend(data.into_iter().map(|(_, v)| v));
        c.execute(&sql, chatbotx_db::rusqlite::params_from_iter(params))?;
        Ok(())
    })
    .await
}

/// `upsertByOwner(ownerId, data)`: updates the owner's tenant (no row = no-op).
pub async fn upsert_by_owner(
    db: &Db,
    owner_id: i64,
    data: Vec<(&'static str, SqlValue)>,
) -> Result<()> {
    update(db, "ownerId", owner_id, data).await
}

/// `upsertById(tenantId, data)`.
pub async fn upsert_by_id(
    db: &Db,
    tenant_id: i64,
    data: Vec<(&'static str, SqlValue)>,
) -> Result<()> {
    update(db, "id", tenant_id, data).await
}

/// `setStatusByOwner`.
pub async fn set_status_by_owner(db: &Db, owner_id: i64, status: &str) -> Result<()> {
    upsert_by_owner(
        db,
        owner_id,
        vec![("status", SqlValue::Text(status.to_owned()))],
    )
    .await
}

/// `suspend(ownerId)` (status only, see the module note).
pub async fn suspend(db: &Db, owner_id: i64) -> Result<()> {
    set_status_by_owner(db, owner_id, "suspended").await
}

/// `reactivate(ownerId)` (status only, see the module note).
pub async fn reactivate(db: &Db, owner_id: i64) -> Result<()> {
    set_status_by_owner(db, owner_id, "active").await
}

/// `downgrade(ownerId)`: suspend + clear the white-label entitlement flags.
pub async fn downgrade(db: &Db, owner_id: i64) -> Result<()> {
    suspend(db, owner_id).await?;
    crate::user_quota::clear_white_label_entitlements(db, owner_id).await
}

/// What [`reconcile_owner_entitlement`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reconciled {
    Provisioned,
    Reactivated,
    Downgraded,
    Unchanged,
}

/// `reconcileOwnerEntitlement(ownerId)`: a tenant exactly while the owner
/// holds the white-label entitlement.
pub async fn reconcile_owner_entitlement(db: &Db, owner_id: i64) -> Result<Reconciled> {
    let has_white_label = crate::user_quota::has_white_label_entitlement(db, owner_id).await?;
    let tenant = find_by_owner(db, owner_id).await?;
    Ok(match (has_white_label, tenant) {
        (true, None) => {
            provision_for_owner(db, owner_id).await?;
            Reconciled::Provisioned
        }
        (true, Some(t)) if t.status != "active" => {
            reactivate(db, owner_id).await?;
            Reconciled::Reactivated
        }
        (false, Some(t)) if t.status == "active" => {
            downgrade(db, owner_id).await?;
            Reconciled::Downgraded
        }
        _ => Reconciled::Unchanged,
    })
}

/// `customDomainService.findActiveByDomain`.
pub async fn find_active_domain(db: &Db, domain: &str) -> Result<Option<CustomDomain>> {
    let domain = domain.to_owned();
    tx::read(db, move |c| {
        Ok(repo::find_one::<CustomDomain>(
            c,
            r#""domain" = ?1 AND "status" = 'active'"#,
            &[&domain],
        )?)
    })
    .await
}

/// `customDomainService.findActiveByTenantId`.
pub async fn find_active_domain_by_tenant(db: &Db, tenant_id: i64) -> Result<Option<CustomDomain>> {
    tx::read(db, move |c| {
        Ok(repo::find_one::<CustomDomain>(
            c,
            r#""tenantId" = ?1 AND "status" = 'active'"#,
            &[&tenant_id],
        )?)
    })
    .await
}

/// `customDomainService.findByTenantId`.
pub async fn find_domains_by_tenant(db: &Db, tenant_id: i64) -> Result<Vec<CustomDomain>> {
    tx::read(db, move |c| {
        Ok(repo::find_many::<CustomDomain>(
            c,
            r#"WHERE "tenantId" = ?1 ORDER BY "id""#,
            &[&tenant_id],
        )?)
    })
    .await
}

/// `customDomainService.listActiveDomains`.
pub async fn list_active_domains(db: &Db) -> Result<Vec<String>> {
    tx::read(db, |c| {
        let mut stmt = c.prepare(
            r#"SELECT "domain" FROM "CustomDomain" WHERE "status" = 'active' ORDER BY "id""#,
        )?;
        let rows = stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(rows)
    })
    .await
}

/// `resolveTenantByDomain` (`@chatbotx.io/auth/tenant`): the tenant of an
/// active custom domain whose tenant is active; otherwise the root tenant.
pub async fn resolve_tenant_by_domain(db: &Db, domain: &str) -> Result<i64> {
    if domain.is_empty() {
        return Ok(ROOT_TENANT_ID);
    }
    let Some(custom) = find_active_domain(db, domain).await? else {
        return Ok(ROOT_TENANT_ID);
    };
    Ok(match find_by_id(db, custom.tenant_id).await? {
        Some(tenant) if tenant.status == "active" => custom.tenant_id,
        _ => ROOT_TENANT_ID,
    })
}

/// `resolveTenantOwnerId`: `None` for the root tenant or an unknown tenant.
pub async fn resolve_tenant_owner_id(db: &Db, tenant_id: i64) -> Result<Option<i64>> {
    if tenant_id == ROOT_TENANT_ID {
        return Ok(None);
    }
    Ok(find_by_id(db, tenant_id).await?.and_then(|t| t.owner_id))
}

/// `resolveCredentialOwnerIdForWorkspace` (`google-ads/owner.ts`): the
/// tenant owner of a reseller workspace, else the workspace owner.
/// `loaded` is `(ownerId, tenantId)` of an already-read workspace.
pub async fn resolve_credential_owner_id_for_workspace(
    db: &Db,
    workspace_id: i64,
    loaded: Option<(i64, i64)>,
) -> Result<i64> {
    let (owner_id, tenant_id) = match loaded {
        Some(workspace) => workspace,
        None => {
            let workspace = crate::workspace::find_by_id(db, workspace_id).await?;
            (workspace.owner_id, workspace.tenant_id)
        }
    };
    if tenant_id != ROOT_TENANT_ID
        && let Some(owner) = find_by_id(db, tenant_id).await?.and_then(|t| t.owner_id)
    {
        return Ok(owner);
    }
    Ok(owner_id)
}

/// `resolvePlatformOwnerId` (`apps/builder/src/lib/platform-credential-owner.ts`):
/// whose platform credentials a channel connect uses. The host (`x-domain`)
/// wins over the workspace; non-cloud editions always use the current user.
pub async fn resolve_platform_owner_id(
    db: &Db,
    edition: crate::keys::Edition,
    domain: &str,
    user_id: i64,
    workspace_id: Option<i64>,
) -> Result<i64> {
    if !edition.is_cloud() {
        return Ok(user_id);
    }
    let domain_tenant = resolve_tenant_by_domain(db, domain).await?;
    if domain_tenant != ROOT_TENANT_ID {
        return Ok(resolve_tenant_owner_id(db, domain_tenant)
            .await?
            .unwrap_or(user_id));
    }
    if let Some(id) = workspace_id
        && let Some(workspace) = tx::read(db, move |c| Ok(crate::workspace::find(c, id)?)).await?
    {
        return resolve_credential_owner_id_for_workspace(
            db,
            id,
            Some((workspace.owner_id, workspace.tenant_id)),
        )
        .await;
    }
    Ok(user_id)
}
