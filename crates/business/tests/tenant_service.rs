//! `tenant-reconcile.test.ts`, `tenant-channel-visibility.test.ts` and
//! `custom-domain-service.test.ts` (Q.D11 slice 10) on a real database
//! instead of spied service methods.

mod common;

use chatbotx_business::tenant::{self, ROOT_TENANT_ID, Reconciled};
use chatbotx_db::DbError;
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value as SqlValue;
use common::{fresh_db, setup};

const OWNER: i64 = 11;

fn user(tx: &Connection, id: i64) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "User" ("id", "email") VALUES (?1, ?2)"#,
        chatbotx_db::rusqlite::params![id, format!("u{id}@tenant.test")],
    )?;
    Ok(())
}

fn white_label(tx: &Connection, id: i64, on: bool) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "UserQuota" ("id", "userId", "whiteLabel", "ssoSaml") VALUES (?1, ?1, ?2, 1)"#,
        chatbotx_db::rusqlite::params![id, on],
    )?;
    Ok(())
}

fn tenant_row(
    tx: &Connection,
    id: i64,
    owner: i64,
    status: &str,
    hidden: &str,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "Tenant" ("id", "ownerId", "status", "hiddenChannels") VALUES (?1, ?2, ?3, ?4)"#,
        chatbotx_db::rusqlite::params![id, owner, status, hidden],
    )?;
    Ok(())
}

async fn status(db: &chatbotx_db::Db) -> Option<String> {
    tenant::find_by_owner(db, OWNER)
        .await
        .unwrap()
        .map(|t| t.status)
}

#[tokio::test]
async fn reconcile_provisions_reactivates_and_downgrades() {
    // "white-label owner with no tenant → provisions one".
    let db = fresh_db().await;
    setup(&db, |tx| {
        user(tx, OWNER)?;
        white_label(tx, OWNER, true)
    })
    .await;
    assert_eq!(
        tenant::reconcile_owner_entitlement(&db, OWNER)
            .await
            .unwrap(),
        Reconciled::Provisioned
    );
    assert_eq!(status(&db).await.as_deref(), Some("active"));
    // "white-label owner with an active tenant → no-op".
    assert_eq!(
        tenant::reconcile_owner_entitlement(&db, OWNER)
            .await
            .unwrap(),
        Reconciled::Unchanged
    );
    let id = tenant::find_by_owner(&db, OWNER).await.unwrap().unwrap().id;
    assert_eq!(tenant::provision_for_owner(&db, OWNER).await.unwrap(), id);

    // "white-label owner with a suspended tenant → reactivates it".
    tenant::suspend(&db, OWNER).await.unwrap();
    assert_eq!(
        tenant::reconcile_owner_entitlement(&db, OWNER)
            .await
            .unwrap(),
        Reconciled::Reactivated
    );
    assert_eq!(status(&db).await.as_deref(), Some("active"));

    // "non-white-label owner with an active tenant → downgrades it".
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "UserQuota" SET "whiteLabel" = 0 WHERE "userId" = ?1"#,
            [OWNER],
        )?;
        Ok(())
    })
    .await;
    assert_eq!(
        tenant::reconcile_owner_entitlement(&db, OWNER)
            .await
            .unwrap(),
        Reconciled::Downgraded
    );
    assert_eq!(status(&db).await.as_deref(), Some("suspended"));
    assert_eq!(
        common::scalar_i64(
            &db,
            r#"SELECT "ssoSaml" FROM "UserQuota" WHERE "userId" = ?1"#,
            OWNER
        )
        .await,
        Some(0)
    );
    // "non-white-label owner with an already-suspended tenant → no-op".
    assert_eq!(
        tenant::reconcile_owner_entitlement(&db, OWNER)
            .await
            .unwrap(),
        Reconciled::Unchanged
    );
}

/// "non-white-label owner with no tenant → no-op".
#[tokio::test]
async fn reconcile_without_entitlement_or_tenant_is_a_no_op() {
    let db = fresh_db().await;
    setup(&db, |tx| user(tx, OWNER)).await;
    assert_eq!(
        tenant::reconcile_owner_entitlement(&db, OWNER)
            .await
            .unwrap(),
        Reconciled::Unchanged
    );
    assert_eq!(status(&db).await, None);
}

/// "returns owner ids and drops the null-owner root tenant".
#[tokio::test]
async fn list_active_owner_ids_skips_the_root_tenant() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        for id in [21, 22, 23] {
            user(tx, id)?;
        }
        tenant_row(tx, 2, 21, "active", "[]")?;
        tenant_row(tx, 3, 22, "suspended", "[]")?;
        tenant_row(tx, 4, 23, "active", "[]")
    })
    .await;
    assert_eq!(
        tenant::list_active_owner_ids(&db).await.unwrap(),
        vec![21, 23]
    );
}

async fn visible(db: &chatbotx_db::Db, owner: i64) -> Vec<&'static str> {
    tenant::resolve_visible_channels(db, owner).await.unwrap()
}

#[tokio::test]
async fn visible_channels_union_the_platform_and_active_reseller_hidden_sets() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        user(tx, 31)?;
        user(tx, 32)
    })
    .await;
    // "hides nothing when neither tier has configured anything".
    let all = visible(&db, 31).await;
    assert!(all.contains(&"whatsapp") && all.contains(&"zalo"));
    assert!(!all.contains(&"smtp") && !all.contains(&"omnichannel"));

    // "platform hiding a channel removes it for every owner".
    tenant::upsert_by_id(
        &db,
        ROOT_TENANT_ID,
        vec![("hiddenChannels", SqlValue::Text(r#"["tiktok"]"#.into()))],
    )
    .await
    .unwrap();
    let plain = visible(&db, 31).await;
    assert!(!plain.contains(&"tiktok") && plain.contains(&"whatsapp"));

    // "a reseller can narrow further" / "cannot un-hide".
    setup(&db, |tx| {
        tenant_row(tx, 5, 31, "active", r#"["zalo"]"#)?;
        tenant_row(tx, 6, 32, "suspended", r#"["zalo"]"#)
    })
    .await;
    let reseller = visible(&db, 31).await;
    assert!(!reseller.contains(&"tiktok") && !reseller.contains(&"zalo"));
    assert!(reseller.contains(&"whatsapp"));
    // "ignores a suspended reseller tenant's hiddenChannels entirely".
    let suspended = visible(&db, 32).await;
    assert!(suspended.contains(&"zalo") && !suspended.contains(&"tiktok"));
}

#[tokio::test]
async fn upsert_by_owner_sets_the_branding_of_that_tenant_only() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        user(tx, OWNER)?;
        tenant_row(tx, 7, OWNER, "active", "[]")
    })
    .await;
    tenant::upsert_by_owner(
        &db,
        OWNER,
        vec![
            ("brandName", SqlValue::Text("Acme".into())),
            ("theme", SqlValue::Null),
        ],
    )
    .await
    .unwrap();
    let t = tenant::find_by_owner(&db, OWNER).await.unwrap().unwrap();
    assert_eq!((t.brand_name.as_deref(), t.theme), (Some("Acme"), None));
    let root = tenant::find_by_id(&db, ROOT_TENANT_ID)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(root.brand_name, None);
    // No tenant for the owner: a no-op.
    tenant::upsert_by_owner(&db, 999, vec![("brandName", SqlValue::Text("X".into()))])
        .await
        .unwrap();
}

/// `customDomainService.findActiveByTenantId`: "returns the active domain row
/// for the tenant" / "returns undefined when the tenant has no active domain".
#[tokio::test]
async fn custom_domain_reads_only_active_rows() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        user(tx, OWNER)?;
        tenant_row(tx, 8, OWNER, "active", "[]")?;
        tx.execute(
            r#"INSERT INTO "CustomDomain" ("id", "tenantId", "domain", "status") VALUES (1, 8, 'chat.acme.test', 'active')"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let found = tenant::find_active_domain_by_tenant(&db, 8)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.domain, "chat.acme.test");
    assert!(
        tenant::find_active_domain_by_tenant(&db, ROOT_TENANT_ID)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        tenant::list_active_domains(&db).await.unwrap(),
        vec!["chat.acme.test".to_string()]
    );
    assert_eq!(
        tenant::find_active_domain(&db, "chat.acme.test")
            .await
            .unwrap()
            .map(|d| d.tenant_id),
        Some(8)
    );
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "CustomDomain" SET "status" = 'pending' WHERE "id" = 1"#,
            [],
        )?;
        Ok(())
    })
    .await;
    assert!(
        tenant::find_active_domain_by_tenant(&db, 8)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        tenant::find_domains_by_tenant(&db, 8).await.unwrap().len(),
        1
    );
}
