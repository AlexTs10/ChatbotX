//! The help item actions under `/rpc/helpItemActions/<name>` (Q.D11 slice
//! 10), from `packages/business/__tests__/tenant-help-item-service.test.ts`
//! (update / delete only inside the resolved tenant, else 404) plus the
//! scope resolution of `features/help-items/scope.ts`.

mod rpc_common;

use axum::http::StatusCode;
use chatbotx_business::keys::Edition;
use rpc_common::{AGENT_TOKEN, OWNER, OWNER_TOKEN, action, fixture_as, scalar};
use serde_json::json;

const KEY: &str = "helpItemActions";
const OTHER_TENANT: i64 = 7;
const FOREIGN_ITEM: i64 = 900;

async fn seeded(edition: Edition) -> rpc_common::Fixture {
    let fx = fixture_as(edition).await;
    fx.db()
        .write(|tx| {
            tx.execute(r#"INSERT INTO "PlatformAdmin" ("id", "userId") VALUES (1, ?1)"#, [OWNER])?;
            tx.execute(r#"INSERT INTO "Tenant" ("id", "status") VALUES (?1, 'active')"#, [OTHER_TENANT])?;
            tx.execute(
                r#"INSERT INTO "TenantHelpItem" ("id", "tenantId", "name", "url") VALUES (?1, ?2, 'Theirs', 'https://other.test')"#,
                [FOREIGN_ITEM, OTHER_TENANT],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    fx
}

#[tokio::test]
async fn create_update_and_delete_in_the_root_tenant() {
    let fx = seeded(Edition::Community).await;
    let (status, out) = action(
        &fx,
        KEY,
        "createHelpItem",
        OWNER_TOKEN,
        json!({ "scope": "platform", "name": " Docs ", "url": "https://docs.test", "icon": "", "position": "2" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let item = &out["json"];
    assert_eq!(
        (
            item["name"].clone(),
            item["icon"].clone(),
            item["position"].clone(),
            item["tenantId"].clone()
        ),
        (json!("Docs"), json!(null), json!(2), json!("1"))
    );
    let id = item["id"].as_str().unwrap().to_owned();

    let (status, out) = action(
        &fx,
        KEY,
        "updateHelpItem",
        OWNER_TOKEN,
        json!({ "scope": "tenant", "id": id, "name": "Mail us", "url": "mailto:help@docs.test" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["url"], json!("mailto:help@docs.test"));
    // The defaults of `helpItemSchema` apply again.
    assert_eq!(out["json"]["position"], json!(0));

    let (status, _) = action(
        &fx,
        KEY,
        "deleteHelpItem",
        OWNER_TOKEN,
        json!({ "scope": "platform", "id": id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "TenantHelpItem" WHERE "tenantId" = ?1"#,
            1
        )
        .await,
        json!(0)
    );
}

/// "throws not found when updating / deleting an item outside the resolved
/// tenant".
#[tokio::test]
async fn an_item_of_another_tenant_is_not_found() {
    let fx = seeded(Edition::Community).await;
    let foreign = FOREIGN_ITEM.to_string();
    let (status, out) = action(
        &fx,
        KEY,
        "updateHelpItem",
        OWNER_TOKEN,
        json!({ "scope": "platform", "id": foreign, "name": "Mine", "url": "https://mine.test" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{out}");
    assert_eq!(out["json"]["message"], json!("Help item not found"));
    let (status, _) = action(
        &fx,
        KEY,
        "deleteHelpItem",
        OWNER_TOKEN,
        json!({ "scope": "platform", "id": foreign }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "name" FROM "TenantHelpItem" WHERE "id" = ?1"#,
            FOREIGN_ITEM
        )
        .await,
        json!("Theirs")
    );
}

#[tokio::test]
async fn a_non_super_admin_and_a_bad_url_are_refused() {
    let fx = seeded(Edition::Community).await;
    let input = |url: &str| json!({ "scope": "platform", "name": "Docs", "url": url });
    let (status, out) = action(
        &fx,
        KEY,
        "createHelpItem",
        AGENT_TOKEN,
        input("https://docs.test"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{out}");
    let (status, _) = action(
        &fx,
        KEY,
        "createHelpItem",
        OWNER_TOKEN,
        input("ftp://docs.test"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // On cloud the `tenant` scope needs an active tenant owned by the caller.
    let cloud = seeded(Edition::Cloud).await;
    let tenant = json!({ "scope": "tenant", "name": "Docs", "url": "https://docs.test" });
    let (status, _) = action(&cloud, KEY, "createHelpItem", OWNER_TOKEN, tenant.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    cloud
        .db()
        .write(|tx| {
            tx.execute(
                r#"UPDATE "Tenant" SET "ownerId" = ?1 WHERE "id" = ?2"#,
                [OWNER, OTHER_TENANT],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let (status, out) = action(&cloud, KEY, "createHelpItem", OWNER_TOKEN, tenant).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["tenantId"], json!(OTHER_TENANT.to_string()));
}
