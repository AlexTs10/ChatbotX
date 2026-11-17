//! `messengerChannels.*` / `instagramChannels.*` (Q.I1b slice 6) on a migrated
//! database: summaries, settings reads, and the settings-write checks that run
//! before any Meta call. The Meta calls are covered by parity p1-11 `50` and
//! p1-10 `20`.

mod handlers_common;

use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

async fn seeded() -> handlers_common::Fixture {
    let fx = fixture().await;
    fx.state
        .db
        .write(|tx| {
            for (inbox, channel) in [(30_i64, "messenger"), (31, "instagram")] {
                tx.execute(
                    r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (?1, ?2, 'Inbox', ?3, ?4)"#,
                    chatbotx_db::rusqlite::params![inbox, WS, channel, format!("s{inbox}")],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters",
                     "persistentMenus", "personas", "workspaceId", "inboxId", "datasetId", "hasCapiScope")
                   VALUES (3, '{"tokens":{"accessToken":"page-token-secret"}}', 'p3', 'Page', '[]', '[]', '[]', ?1, 30, 'ds-1', 1)"#,
                chatbotx_db::rusqlite::params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationInstagram" ("id", "auth", "igId", "pageId", "name", "username", "workspaceId", "inboxId", "type",
                     "conversationStarters", "persistentMenus")
                   VALUES (4, '{}', 'ig4', 'p4', 'Account', 'acct', ?1, 31, 'instagram', '[]', '[]')"#,
                chatbotx_db::rusqlite::params![WS],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    fx
}

async fn run(
    fx: &handlers_common::Fixture,
    route: &str,
    ws: i64,
    id: &str,
    body: Option<Value>,
) -> (u16, Value) {
    call(fx, route, ws, &[("id", id)], None, body).await
}

#[tokio::test]
async fn summaries_and_settings_reads() {
    let fx = seeded().await;
    let (status, out) = call(&fx, "messengerChannels.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out[0]["id"], json!("3"));
    assert_eq!(out[0]["capiConnected"], json!(true));
    assert!(!out.to_string().contains("page-token"), "{out}");
    let (status, out) = run(&fx, "instagramChannels.get", WS, "4", None).await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(
        (out["username"].clone(), out["adsEligible"].clone()),
        (json!("acct"), json!(false))
    );
    let (status, out) = run(&fx, "messengerChannels.get", OTHER_WS, "3", None).await;
    assert_eq!(
        (status, out["message"].clone()),
        (404, json!("Channel integration not found"))
    );
    let (status, out) = run(&fx, "messengerChannels.getSettings", WS, "3", None).await;
    assert_eq!(
        (status, out),
        (
            200,
            json!({ "welcomeFlowId": null, "persistentMenus": [], "personas": [], "conversationStarters": [] })
        )
    );
    let (status, out) = run(&fx, "instagramChannels.getSettings", OTHER_WS, "4", None).await;
    assert_eq!(
        (status, out["message"].clone()),
        (404, json!("Record not found"))
    );
}

#[tokio::test]
async fn settings_writes_refuse_bad_input_before_meta() {
    let fx = seeded().await;
    let empty = json!({ "welcomeFlowId": null, "persistentMenus": [], "personas": [], "conversationStarters": [] });
    let with = |patch: Value| {
        let mut body = empty.clone();
        for (k, v) in patch.as_object().unwrap() {
            body[k] = v.clone();
        }
        body
    };
    let picture = "https://parity.test/a.png";
    for (body, path) in [
        (
            with(json!({ "personas": [
                { "name": "A", "isDefault": true, "profilePictureUrl": picture },
                { "name": "B", "isDefault": true, "profilePictureUrl": picture },
            ] })),
            "\"personas\"",
        ),
        (
            with(json!({ "personas": [{ "name": "A", "isDefault": false }] })),
            "profilePictureUrl",
        ),
        (
            with(
                json!({ "persistentMenus": [{ "type": "url", "label": "X", "url": "not-a-url" }] }),
            ),
            "url",
        ),
        (json!({ "welcomeFlowId": null }), "persistentMenus"),
    ] {
        let (status, out) = run(&fx, "messengerChannels.updateSettings", WS, "3", Some(body)).await;
        assert_eq!(status, 422, "{path}: {out}");
        assert!(out.to_string().contains(path), "{path}: {out}");
    }
    let (status, out) = run(
        &fx,
        "messengerChannels.updateSettings",
        WS,
        "3",
        Some(with(json!({ "welcomeFlowId": "999" }))),
    )
    .await;
    assert_eq!(
        (status, out["message"].clone()),
        (404, json!("Flow does not exists."))
    );
    // PUT on an unknown page: the lookup runs inside the catch-all (400).
    let (status, out) = run(
        &fx,
        "messengerChannels.updateSettings",
        WS,
        "999",
        Some(empty.clone()),
    )
    .await;
    assert_eq!(
        (status, out["code"].clone(), out["message"].clone()),
        (
            400,
            json!("systemError"),
            json!("Failed to update Facebook page")
        )
    );
    let (status, out) = run(
        &fx,
        "messengerChannels.patchSettings",
        WS,
        "999",
        Some(json!({})),
    )
    .await;
    assert_eq!(
        (status, out["message"].clone()),
        (404, json!("Record not found"))
    );

    let ig_empty =
        json!({ "welcomeFlowId": null, "persistentMenus": [], "conversationStarters": [] });
    let (status, out) = run(
        &fx,
        "instagramChannels.updateSettings",
        WS,
        "999",
        Some(ig_empty),
    )
    .await;
    assert_eq!(
        (status, out["message"].clone()),
        (400, json!("Failed to update Instagram integration"))
    );
    let (status, out) = run(
        &fx,
        "instagramChannels.patchSettings",
        WS,
        "4",
        Some(json!({ "persistentMenus": [{ "type": "phone" }] })),
    )
    .await;
    assert_eq!(status, 422, "{out}");
    assert!(out.to_string().contains("type"), "{out}");
    let (status, _) = run(
        &fx,
        "instagramChannels.patchSettings",
        WS,
        "4",
        Some(json!({ "markReadOnOutbound": 1 })),
    )
    .await;
    assert_eq!(status, 422);
}

#[tokio::test]
async fn disconnect_refuses_unknown_or_foreign_channels() {
    let fx = seeded().await;
    let (status, out) = run(&fx, "messengerChannels.disconnect", OTHER_WS, "3", None).await;
    assert_eq!(
        (status, out["message"].clone()),
        (404, json!("Messenger channel not found"))
    );
    let (status, out) = run(&fx, "instagramChannels.disconnect", WS, "999", None).await;
    assert_eq!(
        (status, out["message"].clone()),
        (404, json!("Instagram channel not found"))
    );
    let (status, _) = run(&fx, "messengerChannels.disconnect", WS, "abc", None).await;
    assert_eq!(status, 422);
    let (status, _) = run(&fx, "messengerChannels.get", WS, "3", None).await;
    assert_eq!(status, 200);
}
