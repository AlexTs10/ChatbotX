//! `fbComments.listPosts` / `threadsComments.listPosts` (Q.D10): fixed Graph
//! hosts, the ads filter, reel post ids, Threads reposts left out, a failing
//! account left out of the posts but kept in the pages.

mod common;

use std::sync::Mutex;

use chatbotx_business::comment_automation_posts::{list_facebook_posts, list_threads_posts};
use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use common::{fresh_db, setup};
use serde_json::json;

/// Answers by URL path; records every URL.
struct Fake(Mutex<Vec<String>>);

impl Upstream for Fake {
    async fn send(&self, request: UpstreamRequest) -> UpstreamReply {
        self.0.lock().unwrap().push(request.url.clone());
        let url = request.url;
        let body = if url.contains("/p2/") {
            return Ok((400, "{}".into()));
        } else if url.contains("/posts?") {
            json!({ "data": [{ "id": "p1_1", "message": "hi", "created_time": "t" }] })
        } else if url.contains("/feed?") {
            json!({ "data": [
                { "id": "p1_2", "created_time": "t", "is_eligible_for_promotion": true },
                { "id": "p1_3", "created_time": "t", "is_eligible_for_promotion": false },
            ] })
        } else if url.contains("/video_reels?") {
            json!({ "data": [{ "id": "v9", "post_id": "p1_9", "description": "reel", "picture": "r.jpg", "created_time": "t" }] })
        } else {
            json!({ "data": [
                { "id": "t1", "text": "post", "media_type": "VIDEO", "media_url": "v.mp4", "thumbnail_url": "t.jpg", "timestamp": "t" },
                { "id": "t2", "media_type": "REPOST_FACADE", "timestamp": "t" },
            ] })
        };
        Ok((200, body.to_string()))
    }
}

#[tokio::test]
async fn lists_facebook_and_threads_posts_per_account() {
    let db = fresh_db().await;
    setup(&db, |tx| {
        Ok(tx.execute_batch(
            r#"INSERT INTO "User" ("id", "email", "name", "emailVerified") VALUES (1, 'a@x.test', 'A', 1);
               INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (10, 'ws', 1);
               INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES
                 (30, 10, 'P1', 'messenger', 's30'), (31, 10, 'P2', 'messenger', 's31'), (32, 10, 'T', 'threads', 's32');
               INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId") VALUES
                 (1, '{"tokens":{"accessToken":"tok1"},"version":"v21.0"}', 'p1', 'Page 1', '[]', '[]', '[]', 10, 30),
                 (2, '{"tokens":{"accessToken":"tok2"}}', 'p2', 'Page 2', '[]', '[]', '[]', 10, 31);
               INSERT INTO "IntegrationThreads" ("id", "auth", "threadsUserId", "username", "name", "workspaceId", "inboxId") VALUES
                 (1, '{"tokens":{"accessToken":"t&ok"},"metadata":{}}', 'u1', 'ann', 'Ann', 10, 32);"#,
        )?)
    })
    .await;
    let fake = Fake(Mutex::new(Vec::new()));

    let fb = list_facebook_posts(&db, &fake, 10, None).await.unwrap();
    assert_eq!(
        fb["pages"],
        json!([{ "id": "p1", "name": "Page 1" }, { "id": "p2", "name": "Page 2" }])
    );
    assert_eq!(
        fb["published"],
        json!([{ "id": "p1_1", "message": "hi", "created_time": "t", "pageId": "p1", "inboxId": "30" }])
    );
    assert_eq!(fb["ads"].as_array().unwrap().len(), 1);
    assert_eq!(fb["ads"][0]["id"], "p1_2");
    assert_eq!(fb["reels"][0]["id"], "p1_9");
    assert_eq!(fb["reels"][0]["full_picture"], "r.jpg");
    let urls = fake.0.lock().unwrap().clone();
    assert!(
        urls.iter()
            .all(|u| u.starts_with("https://graph.facebook.com/"))
    );
    assert!(
        urls[0].starts_with("https://graph.facebook.com/v21.0/p1/posts?"),
        "{urls:?}"
    );
    assert!(
        urls.iter()
            .any(|u| u.starts_with("https://graph.facebook.com/v23.0/p2/"))
    );

    // `inboxId` keeps one account.
    let one = list_facebook_posts(&db, &fake, 10, Some(31)).await.unwrap();
    assert_eq!(one["pages"], json!([{ "id": "p2", "name": "Page 2" }]));
    assert_eq!(one["published"], json!([]));
    // Another workspace sees nothing.
    let other = list_facebook_posts(&db, &fake, 11, None).await.unwrap();
    assert_eq!(other["pages"], json!([]));

    fake.0.lock().unwrap().clear();
    let threads = list_threads_posts(&db, &fake, 10, None).await.unwrap();
    assert_eq!(threads["accounts"], json!([{ "id": "u1", "name": "@ann" }]));
    assert_eq!(
        threads["posts"],
        json!([{ "id": "t1", "message": "post", "full_picture": "t.jpg", "created_time": "t", "accountId": "u1", "inboxId": "32" }])
    );
    let urls = fake.0.lock().unwrap().clone();
    assert!(
        urls[0].starts_with("https://graph.threads.com/v1.0/me/threads?"),
        "{urls:?}"
    );
    assert!(urls[0].contains("access_token=t%26ok"), "{urls:?}");
}
