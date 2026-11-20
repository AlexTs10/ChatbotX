//! Port of `apps/builder/__tests__/get-post-details-query.test.ts`
//! (Q.P6-U1). Adapted: the mocked services become integration rows and a
//! fake upstream. Not ported: the degraded-TTL and cache-tag cases (the
//! cache is in-process, see `post_details` module docs).

mod common;

use std::sync::Mutex;

use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use chatbotx_business::post_details::{cache_key, get_post_details};
use common::{OTHER_WS, WS, setup, two_workspaces};
use serde_json::json;

struct Fake(Mutex<Vec<String>>);

impl Upstream for Fake {
    async fn send(&self, request: UpstreamRequest) -> UpstreamReply {
        self.0.lock().unwrap().push(request.url.clone());
        let body = if request.url.contains("graph.threads.com") {
            json!({
                "id": "17841400000000001",
                "text": "video post",
                "media_type": "VIDEO",
                "media_url": "https://cdn.example.com/video.mp4",
                "thumbnail_url": "https://cdn.example.com/thumb.jpg",
                "timestamp": "2026-09-01T00:00:00Z",
            })
        } else {
            json!({ "error": { "message": "unexpected" } })
        };
        Ok((200, body.to_string()))
    }
}

fn fake() -> Fake {
    Fake(Mutex::new(Vec::new()))
}

/// Inbox 30 tiktok and 31 threads of `WS`; inbox 40 tiktok of `OTHER_WS`.
async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        Ok(tx.execute_batch(&format!(
            r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES
                 (30, {WS}, 'Tik', 'tiktok', 's30'), (31, {WS}, 'Thr', 'threads', 's31'), (40, {OTHER_WS}, 'Tik2', 'tiktok', 's40');
               INSERT INTO "IntegrationTiktok" ("id", "auth", "openId", "name", "workspaceId", "inboxId") VALUES
                 (1, '{{"tokens":{{"accessToken":"tok"}},"metadata":{{"username":"acme.studio"}}}}', 'open_1', 'Acme Studio', {WS}, 30),
                 (2, '{{"tokens":{{"accessToken":"tok"}},"metadata":{{"username":"other"}}}}', 'open_2', 'Other', {OTHER_WS}, 40);
               INSERT INTO "IntegrationThreads" ("id", "auth", "threadsUserId", "username", "name", "workspaceId", "inboxId") VALUES
                 (1, '{{"tokens":{{"accessToken":"token"}},"metadata":{{}}}}', 'u1', 'ann', 'Ann', {WS}, 31);"#
        ))?)
    })
    .await;
    db
}

#[tokio::test]
async fn rejects_a_channel_that_has_no_post_concept_instead_of_asking_messenger() {
    let db = seeded().await;
    let upstream = fake();
    let error = get_post_details(&db, &upstream, WS, "30", "7123", "whatsapp")
        .await
        .expect_err("unsupported");
    assert!(error.to_string().contains("whatsapp"), "{error}");
    assert!(upstream.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn routes_a_tiktok_comment_conversation_to_the_tiktok_service() {
    let db = seeded().await;
    let upstream = fake();
    let result = get_post_details(&db, &upstream, WS, "30", "7123001", "tiktok")
        .await
        .expect("details");
    assert_eq!(
        result,
        json!({
            "from": { "id": "open_1", "name": "Acme Studio" },
            "link": "https://www.tiktok.com/@acme.studio/video/7123001",
        })
    );
    // No `video.list` scope: no TikTok or Graph call.
    assert!(upstream.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn shows_a_threads_videos_thumbnail_not_its_mp4_media_url() {
    let db = seeded().await;
    let upstream = fake();
    let result = get_post_details(&db, &upstream, WS, "31", "17841400000000001", "threads")
        .await
        .expect("details");
    assert_eq!(
        result["picture"],
        json!("https://cdn.example.com/thumb.jpg")
    );
}

#[tokio::test]
async fn scopes_the_lookup_by_workspace() {
    let db = seeded().await;
    for channel in ["tiktok", "instagram", "threads", "messenger"] {
        // Inbox 40 belongs to `OTHER_WS`.
        let error = get_post_details(&db, &fake(), WS, "40", "7123002", channel)
            .await
            .expect_err(channel);
        assert!(
            error.to_string().contains("Record not found"),
            "{channel}: {error}"
        );
    }
}

#[test]
fn keys_the_cache_by_workspace() {
    assert_eq!(
        cache_key(42, "inbox_1", "7123"),
        "post-details:42:inbox_1:7123"
    );
}
