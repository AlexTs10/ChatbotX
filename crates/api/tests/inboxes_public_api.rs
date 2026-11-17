//! `apps/builder/__tests__/inboxes-public-api.test.ts` (Q.A1): the TS test
//! mocks `inboxService`; here the routes run on a migrated database and the
//! responses and rows are checked. Plus `InboxService.list` of
//! `packages/business/src/inbox/__tests__/service.test.ts` (connected only).

mod handlers_common;

use handlers_common::{OTHER_WS, WS, call, fixture, scalar};
use serde_json::json;

async fn seeded() -> handlers_common::Fixture {
    let fx = fixture().await;
    fx.state
        .db
        .write(|tx| {
            for (id, ws, name, source, status) in [
                (5, WS, "Shop", "page-1", "connected"),
                (6, WS, "Old", "page-2", "disconnected"),
                (7, WS, "TikTok", "shop_user", "connected"),
                (8, OTHER_WS, "Other", "page-9", "connected"),
            ] {
                tx.execute(
                    r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status") VALUES (?1, ?2, 'messenger', ?3, ?4, ?5)"#,
                    chatbotx_db::rusqlite::params![id, name, source, ws, status],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    fx
}

#[tokio::test]
async fn updates_mark_read_on_outbound_in_the_tokens_workspace() {
    let fx = seeded().await;
    let (status, out) = call(
        &fx,
        "inboxes.update",
        WS,
        &[("id", "5")],
        None,
        Some(json!({ "markReadOnOutbound": true })),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(
        out,
        json!({ "id": "5", "name": "Shop", "channel": "messenger", "status": "connected", "sourceId": "page-1", "markReadOnOutbound": true })
    );
    let stored = scalar(&fx, r#"SELECT "markReadOnOutbound" FROM "Inbox" WHERE "id" = ?1"#, 5).await;
    assert_eq!(stored, Some(json!(1)));
    // Another workspace cannot change it.
    let (status, _) = call(
        &fx,
        "inboxes.update",
        OTHER_WS,
        &[("id", "5")],
        None,
        Some(json!({ "markReadOnOutbound": false })),
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn with_no_field_it_returns_the_inbox_unchanged_and_writes_nothing() {
    let fx = seeded().await;
    let before = scalar(&fx, r#"SELECT "updatedAt" FROM "Inbox" WHERE "id" = ?1"#, 5).await;
    let (status, out) = call(&fx, "inboxes.update", WS, &[("id", "5")], None, Some(json!({}))).await;
    assert_eq!((status, out["markReadOnOutbound"].clone()), (200, json!(false)));
    let after = scalar(&fx, r#"SELECT "updatedAt" FROM "Inbox" WHERE "id" = ?1"#, 5).await;
    assert_eq!(before, after);
}

#[tokio::test]
async fn a_missing_inbox_is_a_404() {
    let fx = seeded().await;
    let (status, out) = call(&fx, "inboxes.update", WS, &[("id", "404")], None, Some(json!({}))).await;
    assert_eq!(status, 404);
    assert_eq!(out["message"], json!("Inbox not found"));
    let (status, _) = call(&fx, "inboxes.update", WS, &[("id", "x")], None, None).await;
    assert_eq!(status, 422);
}

#[tokio::test]
async fn the_public_inbox_resource_exposes_the_setting() {
    let fx = seeded().await;
    let (status, out) = call(&fx, "inboxes.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["data"][0]["markReadOnOutbound"], json!(false));
}

#[tokio::test]
async fn list_includes_connected_inboxes_and_excludes_disconnected_inboxes() {
    let fx = seeded().await;
    let (status, out) = call(&fx, "inboxes.list", WS, &[], Some("includes[]=integration"), None).await;
    assert_eq!(status, 200, "{out}");
    let ids: Vec<_> = out["data"].as_array().unwrap().iter().map(|i| i["id"].clone()).collect();
    assert_eq!(ids, vec![json!("5"), json!("7")]);
    // `ceil(2 / (perPage ?? 10))`.
    assert_eq!(out["pageCount"], json!(1));
    let (_, page2) = call(&fx, "inboxes.list", WS, &[], Some("page=2&perPage=1"), None).await;
    assert_eq!((page2["data"][0]["id"].clone(), page2["pageCount"].clone()), (json!("7"), json!(2)));
    let (status, _) = call(&fx, "inboxes.list", WS, &[], Some("perPage=0"), None).await;
    assert_eq!(status, 422);
    let (status, _) = call(&fx, "inboxes.list", WS, &[], Some("includes[]=tags"), None).await;
    assert_eq!(status, 422);
}

#[tokio::test]
async fn deprecated_list_channels_returns_the_source_id_as_id() {
    let fx = seeded().await;
    let (status, out) = call(&fx, "inboxes.listChannels", WS, &[], None, None).await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(
        out["data"][1],
        json!({ "id": "shop_user", "name": "TikTok", "channel": "messenger", "status": "connected" })
    );
}
