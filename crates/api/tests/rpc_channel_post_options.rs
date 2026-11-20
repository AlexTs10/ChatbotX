//! Port of `apps/builder/__tests__/channel-post-options-api.test.ts`
//! (Q.P6-U1): the private `/rpc` option procedures answer what the public
//! `channelPosts.list` / `listSelected` routes answer, scoped to the
//! caller's workspace. The schema cases (`channelPostId`, cursor, option
//! contract) are the public routes' and are tested in
//! `channel_posts_public_api`.

mod rpc_common;

use axum::http::StatusCode;
use rpc_common::{OUTSIDER_TOKEN, OWNER, OWNER_TOKEN, WS, call, fixture};
use serde_json::{Value, json};

const OTHER: i64 = 202;

async fn seeded() -> rpc_common::Fixture {
    let fx = fixture().await;
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, 'Other', ?2)"#,
                [OTHER, OWNER],
            )?;
            for (inbox, ws) in [(40_i64, WS), (41, OTHER)] {
                tx.execute(
                    r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (?1, ?2, 'IG', 'instagram', ?3)"#,
                    chatbotx_db::rusqlite::params![inbox, ws, format!("ig{inbox}")],
                )?;
            }
            for (id, ws, inbox, caption) in [
                (101_i64, WS, 40_i64, "Summer sale"),
                (102, WS, 40, "Winter"),
                (104, OTHER, 41, "Other summer sale"),
            ] {
                tx.execute(
                    r#"INSERT INTO "ChannelPost" ("id", "workspaceId", "inboxId", "channel", "integrationId", "sourceAccountId",
                         "externalPostId", "caption", "publishedAt")
                       VALUES (?1, ?2, ?3, 'instagram', 1, 'acct', ?4, ?5, ?1)"#,
                    chatbotx_db::rusqlite::params![id, ws, inbox, format!("post-{id}"), caption],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
    fx
}

fn ids(items: &Value) -> Vec<String> {
    items
        .as_array()
        .expect("array")
        .iter()
        .map(|item| item["id"].as_str().expect("id").to_owned())
        .collect()
}

#[tokio::test]
async fn uses_the_same_scoped_service_method_for_private_and_public_lists() {
    let fx = seeded().await;
    let (status, body) = call(
        &fx,
        "POST",
        "/rpc/channelPostAPIs/privateListChannelPostOptionsAPI",
        Some(OWNER_TOKEN),
        json!({ "workspaceId": WS.to_string(), "limit": 30, "search": "summer" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(ids(&body["json"]["items"]), ["101"]);

    // A caller outside the workspace reads nothing.
    let (status, _) = call(
        &fx,
        "POST",
        "/rpc/channelPostAPIs/privateListChannelPostOptionsAPI",
        Some(OUTSIDER_TOKEN),
        json!({ "workspaceId": WS.to_string(), "limit": 30 }),
    )
    .await;
    assert_ne!(status, StatusCode::OK);
}

#[tokio::test]
async fn resolves_saved_filter_labels_through_the_same_workspace_scoped_service() {
    let fx = seeded().await;
    let (status, body) = call(
        &fx,
        "POST",
        "/rpc/channelPostAPIs/privateGetChannelPostOptionsByIdsAPI",
        Some(OWNER_TOKEN),
        json!({ "workspaceId": WS.to_string(), "ids": ["101", "104", "999"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // 104 belongs to another workspace.
    assert_eq!(ids(&body["json"]["data"]), ["101"]);
}

#[tokio::test]
async fn rejects_malformed_ids_without_throwing() {
    let fx = seeded().await;
    let (status, body) = call(
        &fx,
        "POST",
        "/rpc/channelPostAPIs/privateGetChannelPostOptionsByIdsAPI",
        Some(OWNER_TOKEN),
        json!({ "workspaceId": WS.to_string(), "ids": ["bad", "1.5"] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
