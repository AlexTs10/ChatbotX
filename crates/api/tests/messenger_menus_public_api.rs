//! `apps/builder/__tests__/user-persistent-menus-public-api.test.ts` and
//! `packages/business/__tests__/user-persistent-menu-service.test.ts`
//! (Q.I1b slice 5): the TS tests check the delegation to
//! `userPersistentMenuService`; here the routes run on a migrated database.
//! Plus `messengerPersonas.list` (`listMessengerPersonaOptions`).

mod handlers_common;

use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

fn menus() -> Value {
    json!([
        { "label": "Start", "type": "flow", "flowId": "42", "extra": 1 },
        { "label": "Site", "type": "url", "url": "https://example.com/a" },
    ])
}

#[tokio::test]
async fn create_get_update_list_and_delete_a_menu() {
    let fx = fixture().await;
    let (status, out) = call(
        &fx,
        "userPersistentMenus.create",
        WS,
        &[],
        None,
        Some(json!({ "name": " Main menu ", "persistentMenus": menus() })),
    )
    .await;
    assert_eq!(status, 201, "{out}");
    assert_eq!(out["name"], json!("Main menu"));
    assert!(out.get("workspaceId").is_none(), "{out}");
    // Unknown keys are stripped, as `z.object`.
    assert_eq!(
        out["menus"],
        json!([
            { "label": "Start", "type": "flow", "flowId": "42" },
            { "label": "Site", "type": "url", "url": "https://example.com/a" },
        ])
    );
    let id = out["id"].as_str().unwrap().to_owned();

    let (status, got) = call(
        &fx,
        "userPersistentMenus.get",
        WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!((status, got["id"].clone()), (200, json!(id)));
    let (status, out) = call(
        &fx,
        "userPersistentMenus.get",
        OTHER_WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404, "{out}");
    assert_eq!(out["message"], json!("User persistent menu not found"));

    let (status, out) = call(
        &fx,
        "userPersistentMenus.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": "Updated menu", "persistentMenus": [] })),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(
        (out["name"].clone(), out["menus"].clone()),
        (json!("Updated menu"), json!([]))
    );

    let (status, list) = call(&fx, "userPersistentMenus.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(list["pageCount"], json!(1));
    assert_eq!(list["data"][0]["id"], json!(id));

    let (status, out) = call(
        &fx,
        "userPersistentMenus.delete",
        WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!((status, out), (204, Value::Null));
    // A single-id delete 404s on a missing id.
    let (status, out) = call(
        &fx,
        "userPersistentMenus.delete",
        WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404, "{out}");
}

#[tokio::test]
async fn update_404s_on_a_missing_menu_and_bad_items_are_refused() {
    let fx = fixture().await;
    let (status, out) = call(
        &fx,
        "userPersistentMenus.update",
        WS,
        &[("id", "999")],
        None,
        Some(json!({ "name": "Updated menu", "persistentMenus": [] })),
    )
    .await;
    assert_eq!(status, 404, "{out}");
    assert_eq!(out["message"], json!("User persistent menu not found"));

    for menus in [
        json!([{ "label": "x", "type": "postback" }]),
        json!([{ "label": "", "type": "flow", "flowId": "1" }]),
        json!([{ "label": "x", "type": "url", "url": "not a url" }]),
        json!(["x"]),
        Value::Array(vec![
            json!({ "label": "x", "type": "flow", "flowId": "1" });
            21
        ]),
    ] {
        let (status, out) = call(
            &fx,
            "userPersistentMenus.create",
            WS,
            &[],
            None,
            Some(json!({ "name": "Menu", "persistentMenus": menus })),
        )
        .await;
        assert_eq!(status, 422, "{menus}: {out}");
    }
}

#[tokio::test]
async fn personas_list_only_registered_personas_with_the_page_name() {
    let fx = fixture().await;
    fx.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (30, ?1, 'Page', 'messenger', 'p')"#,
                [WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters",
                     "persistentMenus", "personas", "workspaceId", "inboxId")
                   VALUES (3, '{}', 'p', 'Page', '[]', '[]',
                     '[{"id":"a","name":"Ann","facebookPersonaId":"fb1","isDefault":false},
                       {"id":"b","name":"Bob","isDefault":false}]', ?1, 30)"#,
                [WS],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let (status, out) = call(&fx, "messengerPersonas.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(
        out,
        json!({ "data": [{ "id": "a", "name": "Ann", "pageName": "Page" }] })
    );
    let (_, other) = call(&fx, "messengerPersonas.list", OTHER_WS, &[], None, None).await;
    assert_eq!(other, json!({ "data": [] }));
}
