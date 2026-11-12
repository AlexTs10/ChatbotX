//! Port of `packages/database/__tests__/platform-admin-repository.test.ts` (P3.3). The
//! mocked upsert chain becomes real `PlatformAdmin` rows: the asserted `values` /
//! `onConflictDoUpdate` arguments are read back as the stored role, and "no insert" is
//! "no row".

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::platform_admin::{
    bootstrap_root_admin, delete_by_user_id, find_for_auth, upsert_role,
};
use chatbotx_db::repo::{self};
use common::fresh_db;
use repo_common::{ok, scalar_i64, scalar_text, setup, text};

/// Root-tenant user 11 (`admin@example.com`) and user 12 of tenant 7 with the same email.
async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    setup(&db, |tx| {
        ins!(tx, "Tenant", { "id" => 7_i64 })?;
        ins!(tx, "User", { "id" => 11_i64, "email" => "admin@example.com" })?;
        ins!(tx, "User", { "id" => 12_i64, "email" => "admin@example.com", "tenantId" => 7_i64 })?;
        Ok(())
    })
    .await;
    db
}

async fn role_of(db: &common::TestDb, user_id: i64) -> Option<String> {
    scalar_text(
        db,
        "SELECT \"role\" FROM \"PlatformAdmin\" WHERE \"userId\" = ?1",
        vec![repo_common::int(user_id)],
    )
    .await
}

/// Port of: packages/database/__tests__/platform-admin-repository.test.ts:51 "platformAdminRepository.bootstrapRootAdmin > returns true when it creates the configured root administrator"
#[tokio::test]
async fn platform_admin_repository_bootstrap_root_admin_returns_true_when_it_creates_the_configured_root_administrator()
 {
    let db = seeded().await;
    let created = ok(repo::write(&db, |tx| bootstrap_root_admin(tx, "admin@example.com")).await);
    assert!(created);
    // The root-tenant user (not the tenant-7 user with the same email) is super-admin.
    assert_eq!(role_of(&db, 11).await.as_deref(), Some("super-admin"));
    assert_eq!(role_of(&db, 12).await, None);
}

/// Port of: packages/database/__tests__/platform-admin-repository.test.ts:73 "platformAdminRepository.bootstrapRootAdmin > returns true when the conflict update promotes an administrator"
#[tokio::test]
async fn platform_admin_repository_bootstrap_root_admin_returns_true_when_the_conflict_update_promotes_an_administrator()
 {
    let db = seeded().await;
    setup(&db, |tx| {
        ins!(tx, "PlatformAdmin", { "id" => 1_i64, "userId" => 11_i64, "role" => "support" })?;
        Ok(())
    })
    .await;
    let promoted = ok(repo::write(&db, |tx| bootstrap_root_admin(tx, "admin@example.com")).await);
    assert!(promoted);
    assert_eq!(role_of(&db, 11).await.as_deref(), Some("super-admin"));
    let count = scalar_i64(&db, "SELECT count(*) FROM \"PlatformAdmin\"", vec![]).await;
    assert_eq!(count, Some(1));
}

/// Port of: packages/database/__tests__/platform-admin-repository.test.ts:84 "platformAdminRepository.bootstrapRootAdmin > returns false when the administrator is already super-admin"
#[tokio::test]
async fn platform_admin_repository_bootstrap_root_admin_returns_false_when_the_administrator_is_already_super_admin()
 {
    let db = seeded().await;
    setup(&db, |tx| {
        ins!(tx, "PlatformAdmin", { "id" => 1_i64, "userId" => 11_i64, "role" => "super-admin" })?;
        Ok(())
    })
    .await;
    let changed = ok(repo::write(&db, |tx| bootstrap_root_admin(tx, "admin@example.com")).await);
    assert!(!changed);
    assert_eq!(role_of(&db, 11).await.as_deref(), Some("super-admin"));
}

/// Port of: packages/database/__tests__/platform-admin-repository.test.ts:95 "platformAdminRepository.bootstrapRootAdmin > does not insert when the configured email has no root-tenant user"
#[tokio::test]
async fn platform_admin_repository_bootstrap_root_admin_does_not_insert_when_the_configured_email_has_no_root_tenant_user()
 {
    let db = seeded().await;
    // Only a tenant-7 user has this email.
    setup(&db, |tx| {
        ins!(tx, "User", { "id" => 13_i64, "email" => "tenant-only@example.com", "tenantId" => 7_i64 })?;
        Ok(())
    })
    .await;
    let created = ok(repo::write(&db, |tx| {
        bootstrap_root_admin(tx, "tenant-only@example.com")
    })
    .await);
    assert!(!created);
    let count = scalar_i64(&db, "SELECT count(*) FROM \"PlatformAdmin\"", vec![]).await;
    assert_eq!(count, Some(0));
}

/// Port of: packages/database/__tests__/platform-admin-repository.test.ts:109 "platformAdminRepository.upsertRole > inserts or updates the requested role"
#[tokio::test]
async fn platform_admin_repository_upsert_role_inserts_or_updates_the_requested_role() {
    let db = seeded().await;
    let inserted =
        ok(repo::write(&db, |tx| upsert_role(tx, "support", 11)).await).expect("inserted row");
    assert_eq!(inserted.role, "support");
    assert_eq!(inserted.user_id, 11);
    let updated =
        ok(repo::write(&db, |tx| upsert_role(tx, "super-admin", 11)).await).expect("updated row");
    assert_eq!(updated.id, inserted.id);
    assert_eq!(role_of(&db, 11).await.as_deref(), Some("super-admin"));
}

/// Port of: packages/database/__tests__/platform-admin-repository.test.ts:130 "platformAdminRepository.deleteByUserId > deletes the administrator row"
#[tokio::test]
async fn platform_admin_repository_delete_by_user_id_deletes_the_administrator_row() {
    let db = seeded().await;
    setup(&db, |tx| {
        ins!(tx, "PlatformAdmin", { "id" => 1_i64, "userId" => 11_i64 })?;
        ins!(tx, "PlatformAdmin", { "id" => 2_i64, "userId" => 12_i64 })?;
        Ok(())
    })
    .await;
    ok(repo::write(&db, |tx| delete_by_user_id(tx, 11)).await);
    assert_eq!(role_of(&db, 11).await, None);
    assert!(role_of(&db, 12).await.is_some());
}

/// Port of: packages/database/__tests__/platform-admin-repository.test.ts:144 "platformAdminRepository.findForAuth > reads administrator roles fresh for authorization"
///
/// "Called twice" (no cache) = a role change between two reads is seen by the second one.
#[tokio::test]
async fn platform_admin_repository_find_for_auth_reads_administrator_roles_fresh_for_authorization()
{
    let db = seeded().await;
    setup(&db, |tx| {
        ins!(tx, "PlatformAdmin", { "id" => 1_i64, "userId" => 11_i64, "role" => "super-admin" })?;
        Ok(())
    })
    .await;
    let first = ok(repo::read(&db, |c| find_for_auth(c, 11)).await).expect("admin");
    assert_eq!(first.role, "super-admin");
    setup(&db, |tx| {
        tx.execute(
            "UPDATE \"PlatformAdmin\" SET \"role\" = ?1 WHERE \"userId\" = 11",
            [text("support")],
        )?;
        Ok(())
    })
    .await;
    let second = ok(repo::read(&db, |c| find_for_auth(c, 11)).await).expect("admin");
    assert_eq!(second.role, "support");
}
