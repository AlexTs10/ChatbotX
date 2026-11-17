//! `messengerTemplates.*` (Q.I1b slice 5) on a migrated database: the local
//! routes and the validation that runs before any Meta call. The Meta calls
//! (sync, create, clone) are covered by parity p1-11 `60`.

mod handlers_common;

use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

async fn seeded() -> handlers_common::Fixture {
    let fx = fixture().await;
    fx.state
        .db
        .write(|tx| {
            for (inbox, page, name) in [(30_i64, 3_i64, "Page A"), (31, 4, "Page B")] {
                tx.execute(
                    r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (?1, ?2, ?3, 'messenger', ?4)"#,
                    chatbotx_db::rusqlite::params![inbox, WS, name, format!("p{page}")],
                )?;
                tx.execute(
                    r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters",
                         "persistentMenus", "personas", "workspaceId", "inboxId")
                       VALUES (?1, '{}', ?2, ?3, '[]', '[]', '[]', ?4, ?5)"#,
                    chatbotx_db::rusqlite::params![page, format!("p{page}"), name, WS, inbox],
                )?;
            }
            for (id, page, status) in [(10_i64, 3_i64, "APPROVED"), (11, 4, "PENDING")] {
                tx.execute(
                    r#"INSERT INTO "MessengerMessageTemplate" ("id", "name", "integrationMessengerId", "sourceId",
                         "language", "category", "status", "components")
                       VALUES (?1, ?2, ?3, ?4, 'en_US', 'UTILITY', ?5, '[{"type":"BODY","text":"Hi {{1}}"}]')"#,
                    chatbotx_db::rusqlite::params![id, format!("t{id}"), page, format!("s{id}"), status],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();
    fx
}

fn ids(out: &Value) -> Vec<&str> {
    out.as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn list_filters_newest_first_with_the_page() {
    let fx = seeded().await;
    let list = |query: &'static str, ws: i64| {
        let fx = &fx;
        async move {
            let (status, out) =
                call(fx, "messengerTemplates.list", ws, &[], Some(query), None).await;
            assert_eq!(status, 200, "{out}");
            out
        }
    };
    let all = list("", WS).await;
    assert_eq!(ids(&all), vec!["11", "10"]);
    assert_eq!(
        all[0]["integrationMessenger"],
        json!({ "id": "4", "name": "Page B", "inboxId": "31" })
    );
    assert_eq!(all[0]["parameterFormat"], json!("POSITIONAL"));
    assert_eq!(ids(&list("integrationMessengerId=3", WS).await), vec!["10"]);
    assert_eq!(ids(&list("inboxId=31", WS).await), vec!["11"]);
    assert_eq!(ids(&list("status=PENDING", WS).await), vec!["11"]);
    assert_eq!(list("inboxId=999", WS).await, json!([]));
    assert_eq!(list("", OTHER_WS).await, json!([]));
    let (status, _) = call(
        &fx,
        "messengerTemplates.list",
        WS,
        &[],
        Some("status=DRAFT"),
        None,
    )
    .await;
    assert_eq!(status, 422);
}

#[tokio::test]
async fn get_lists_the_parameters_and_delete_is_local() {
    let fx = seeded().await;
    let (status, out) = call(
        &fx,
        "messengerTemplates.get",
        WS,
        &[("id", "10")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert!(out.get("integrationMessenger").is_none());
    assert_eq!(out["parameters"][0]["key"], json!("body.1"));
    assert_eq!(out["parameters"][0]["required"], json!(true));
    let (status, out) = call(
        &fx,
        "messengerTemplates.get",
        OTHER_WS,
        &[("id", "10")],
        None,
        None,
    )
    .await;
    assert_eq!(
        (status, out["message"].clone()),
        (404, json!("Template not found"))
    );

    let (status, out) = call(
        &fx,
        "messengerTemplates.delete",
        WS,
        &[("id", "10")],
        None,
        None,
    )
    .await;
    assert_eq!((status, out), (204, Value::Null));
    let (status, _) = call(
        &fx,
        "messengerTemplates.delete",
        WS,
        &[("id", "10")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn create_and_clone_refuse_bad_input_before_meta() {
    let fx = seeded().await;
    let base =
        json!({ "name": "booking", "language": "en_US", "headerType": "none", "body": "Hello" });
    let with = |patch: Value| {
        let mut body = base.clone();
        for (k, v) in patch.as_object().unwrap() {
            body[k] = v.clone();
        }
        body
    };
    for (body, path) in [
        (with(json!({ "name": "Has Caps" })), "name"),
        (with(json!({ "body": "Hi {{1}}" })), "bodyVariables"),
        (with(json!({ "headerType": "text" })), "headerText"),
        (
            with(json!({ "headerType": "text_and_image", "headerText": "H" })),
            "headerImageUrl",
        ),
        (
            with(json!({ "buttons": [{ "type": "POSTBACK", "title": "x".repeat(21) }] })),
            "buttons",
        ),
    ] {
        let (status, out) = call(
            &fx,
            "messengerTemplates.create",
            WS,
            &[("id", "3")],
            None,
            Some(body),
        )
        .await;
        assert_eq!(status, 422, "{path}: {out}");
        assert!(out.to_string().contains(path), "{path}: {out}");
    }
    let (status, out) = call(
        &fx,
        "messengerTemplates.create",
        WS,
        &[("id", "999")],
        None,
        Some(base),
    )
    .await;
    assert_eq!(
        (status, out["message"].clone()),
        (404, json!("Messenger channel not found"))
    );

    let clone = |targets: Value| {
        let fx = &fx;
        async move {
            call(
                fx,
                "messengerTemplates.clone",
                WS,
                &[("id", "10")],
                None,
                Some(json!({ "targetIntegrationMessengerIds": targets })),
            )
            .await
        }
    };
    let (status, out) = clone(json!(["3"])).await;
    assert_eq!(
        (status, out["message"].clone()),
        (404, json!("No target channels found in this workspace"))
    );
    let (status, _) = clone(json!([])).await;
    assert_eq!(status, 422);
    let (status, out) = call(
        &fx,
        "messengerTemplates.sync",
        OTHER_WS,
        &[("id", "3")],
        None,
        None,
    )
    .await;
    assert_eq!(
        (status, out["message"].clone()),
        (404, json!("Messenger channel not found"))
    );
}
