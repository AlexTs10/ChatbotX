//! Port of `packages/database/__tests__/integration/integration-whatsapp-lifecycle-targets.test.ts`
//! (P3.2b). Runs on a migrated temp file instead of a real Postgres (ported); each case has
//! its own database file instead of a rolled-back transaction.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo11.rs"]
mod repo11;

use chatbotx_db::Db;
use chatbotx_db::repo::integration_whatsapp::{self as iw, ProviderAccountRouteScope as Scope};
use repo11::{read, seed_number, seed_platform_credential, seed_tenant, seed_user};

async fn empty_db() -> common::TestDb {
    common::fresh_db().await
}

fn platform(app: &str) -> Scope {
    Scope::Platform {
        app_id: app.to_owned(),
    }
}

fn owner(owner_id: i64, app: &str) -> Scope {
    Scope::CredentialOwner {
        owner_id,
        app_id: app.to_owned(),
    }
}

/// The TS `phoneNumberIdsFor`: the phone number ids of the targets, in target order.
async fn phone_number_ids_for(db: &Db, waba: &str, scope: Scope) -> Vec<String> {
    let waba = waba.to_owned();
    read(db, move |c| {
        let targets = iw::list_account_targets_by_waba_id(c, &waba, &scope, None)?;
        let mut out = Vec::new();
        for t in targets {
            out.push(c.query_row(
                r#"SELECT "phoneNumberId" FROM "IntegrationWhatsapp" WHERE "id" = ?1"#,
                [t.integration_id],
                |r| r.get::<_, String>(0),
            )?);
        }
        Ok(out)
    })
    .await
}

#[tokio::test]
async fn integration_whatsapp_repository_list_account_targets_by_waba_id_each_route_only_reaches_the_numbers_that_connected_through_its_own_app()
 {
    let db = empty_db().await;
    let waba = "waba-1";
    let platform_user = seed_user(&db).await;
    let own_credential_user = seed_user(&db).await;
    let reseller_owner = seed_user(&db).await;
    let reseller_customer = seed_user(&db).await;
    let tenant = seed_tenant(&db, reseller_owner).await;
    seed_platform_credential(&db, own_credential_user, false).await;
    seed_platform_credential(&db, reseller_owner, false).await;
    seed_number(
        &db,
        platform_user,
        None,
        waba,
        "p-platform",
        false,
        false,
        None,
    )
    .await;
    seed_number(
        &db,
        own_credential_user,
        None,
        waba,
        "p-own-credential",
        false,
        false,
        None,
    )
    .await;
    seed_number(
        &db,
        reseller_customer,
        Some(tenant),
        waba,
        "p-reseller-customer",
        false,
        false,
        None,
    )
    .await;
    seed_number(
        &db,
        platform_user,
        None,
        waba,
        "p-manual",
        true,
        false,
        None,
    )
    .await;
    seed_number(
        &db,
        platform_user,
        None,
        "waba-1-other",
        "p-other-waba",
        false,
        false,
        None,
    )
    .await;
    assert_eq!(
        phone_number_ids_for(&db, waba, platform("app-platform")).await,
        vec!["p-platform"]
    );
    assert_eq!(
        phone_number_ids_for(&db, waba, owner(own_credential_user, "app-owner")).await,
        vec!["p-own-credential"]
    );
    // A reseller's customers own their workspaces, but the reseller owns the credential.
    assert_eq!(
        phone_number_ids_for(&db, waba, owner(reseller_owner, "app-owner")).await,
        vec!["p-reseller-customer"]
    );
    // ... and the customer's own user id must not.
    assert!(
        phone_number_ids_for(&db, waba, owner(reseller_customer, "app-owner"))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn integration_whatsapp_repository_list_account_targets_by_waba_id_a_number_follows_the_app_it_connected_through_not_the_owner_s_current_credential()
 {
    let db = empty_db().await;
    let waba = "waba-2";
    let user = seed_user(&db).await;
    seed_platform_credential(&db, user, false).await;
    seed_number(
        &db,
        user,
        None,
        waba,
        "p-via-platform",
        false,
        false,
        Some("app-platform"),
    )
    .await;
    seed_number(
        &db,
        user,
        None,
        waba,
        "p-via-own-app",
        false,
        false,
        Some("app-owner"),
    )
    .await;
    assert_eq!(
        phone_number_ids_for(&db, waba, platform("app-platform")).await,
        vec!["p-via-platform"]
    );
    assert_eq!(
        phone_number_ids_for(&db, waba, owner(user, "app-owner")).await,
        vec!["p-via-own-app"]
    );
    // An unrelated app matches neither.
    assert!(
        phone_number_ids_for(&db, waba, platform("app-unrelated"))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn integration_whatsapp_repository_list_account_targets_by_waba_id_a_reseller_without_an_app_of_its_own_falls_under_the_platform_app()
 {
    let db = empty_db().await;
    let reseller_owner = seed_user(&db).await;
    let customer = seed_user(&db).await;
    let tenant = seed_tenant(&db, reseller_owner).await;
    seed_number(
        &db,
        customer,
        Some(tenant),
        "waba-3",
        "p-inherited",
        false,
        false,
        None,
    )
    .await;
    assert_eq!(
        phone_number_ids_for(&db, "waba-3", platform("app-platform")).await,
        vec!["p-inherited"]
    );
}

#[tokio::test]
async fn integration_whatsapp_repository_list_account_targets_by_waba_id_an_owner_whose_credential_row_only_delegates_to_the_platform_stays_on_the_platform_route()
 {
    let db = empty_db().await;
    let delegating = seed_user(&db).await;
    seed_platform_credential(&db, delegating, true).await;
    seed_number(
        &db,
        delegating,
        None,
        "waba-4",
        "p-delegating",
        false,
        false,
        None,
    )
    .await;
    assert_eq!(
        phone_number_ids_for(&db, "waba-4", platform("app-platform")).await,
        vec!["p-delegating"]
    );
}

#[tokio::test]
async fn integration_whatsapp_repository_list_account_targets_by_waba_id_a_manual_integration_is_reached_only_by_its_own_route_and_only_for_its_own_waba()
 {
    let db = empty_db().await;
    let user = seed_user(&db).await;
    let manual = seed_number(
        &db,
        user,
        None,
        "waba-5",
        "p-manual-only",
        true,
        false,
        None,
    )
    .await;
    let sibling = seed_number(&db, user, None, "waba-5", "p-sibling", false, false, None).await;
    let route = |workspace_id| Scope::ManualIntegration {
        integration_id: manual.integration_id,
        workspace_id,
    };
    assert_eq!(
        phone_number_ids_for(&db, "waba-5", route(manual.workspace_id)).await,
        vec!["p-manual-only"]
    );
    // The route pins one integration: a forged workspace id matches nothing.
    assert!(
        phone_number_ids_for(&db, "waba-5", route(sibling.workspace_id))
            .await
            .is_empty()
    );
    // A different WABA never matches even for the right integration.
    assert!(
        phone_number_ids_for(&db, "waba-5-x", route(manual.workspace_id))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn integration_whatsapp_repository_list_account_targets_by_waba_id_the_channel_list_flags_manual_integrations_without_exposing_auth()
 {
    let db = empty_db().await;
    let user = seed_user(&db).await;
    let manual = seed_number(
        &db,
        user,
        None,
        "waba-6a",
        "p-flag-manual",
        true,
        false,
        None,
    )
    .await;
    let embedded = seed_number(
        &db,
        user,
        None,
        "waba-6b",
        "p-flag-embedded",
        false,
        false,
        None,
    )
    .await;
    let manual_rows = read(&db, move |c| {
        iw::list_client_resources_by_workspace_id(c, manual.workspace_id)
    })
    .await;
    let embedded_rows = read(&db, move |c| {
        iw::list_client_resources_by_workspace_id(c, embedded.workspace_id)
    })
    .await;
    assert!(manual_rows[0].is_manual);
    assert!(!embedded_rows[0].is_manual);
    // `IntegrationWhatsappClientResource` has no `auth` field (checked by the type).
    assert_eq!(manual_rows[0].phone_number_id, "p-flag-manual");
}

#[tokio::test]
async fn integration_whatsapp_repository_list_account_targets_by_waba_id_pages_by_integration_id_so_every_number_is_reached()
 {
    let db = empty_db().await;
    let user = seed_user(&db).await;
    let mut ids = Vec::new();
    for phone in ["p-page-1", "p-page-2", "p-page-3"] {
        ids.push(
            seed_number(&db, user, None, "waba-7", phone, false, false, None)
                .await
                .integration_id,
        );
    }
    ids.sort_unstable();
    let after = ids[0];
    let rest = read(&db, move |c| {
        iw::list_account_targets_by_waba_id(c, "waba-7", &platform("app-platform"), Some(after))
    })
    .await;
    assert_eq!(
        rest.iter().map(|t| t.integration_id).collect::<Vec<_>>(),
        ids[1..].to_vec()
    );
}

#[tokio::test]
async fn integration_whatsapp_repository_list_account_targets_by_waba_id_returns_the_connection_id_the_workspace_owner_and_null_for_a_legacy_row()
 {
    let db = empty_db().await;
    let user = seed_user(&db).await;
    let with_connection = seed_number(
        &db,
        user,
        None,
        "waba-8",
        "p-with-connection",
        false,
        true,
        None,
    )
    .await;
    let legacy = seed_number(&db, user, None, "waba-8", "p-legacy", false, false, None).await;
    let targets = read(&db, |c| {
        iw::list_account_targets_by_waba_id(c, "waba-8", &platform("app-platform"), None)
    })
    .await;
    let find = |id: i64| {
        targets
            .iter()
            .find(|t| t.integration_id == id)
            .copied()
            .expect("target")
    };
    let t = find(with_connection.integration_id);
    assert_eq!(
        (t.connection_id, t.workspace_owner_id, t.integration_id),
        (
            with_connection.connection_id,
            user,
            with_connection.integration_id
        )
    );
    assert_eq!(find(legacy.integration_id).connection_id, None);
}
