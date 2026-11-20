//! `missed-comments-scanners.test.ts` (Q.P6-U6): `scan_post_comments` per
//! channel against a scripted upstream.

mod common;

use std::collections::VecDeque;
use std::sync::Mutex;

use chatbotx_business::comment_automation::{ScanError, scan_post_comments};
use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use common::{fresh_db, setup};
use serde_json::{Value, json};

type Replies = VecDeque<(u16, Value)>;

/// Answers the scripted replies in order (the last one repeats); records
/// `url` and headers.
struct Script(Mutex<(Replies, Vec<UpstreamRequest>)>);

impl Script {
    fn new(replies: Vec<(u16, Value)>) -> Self {
        Self(Mutex::new((replies.into(), Vec::new())))
    }
    fn urls(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .1
            .iter()
            .map(|r| r.url.clone())
            .collect()
    }
    fn requests(&self) -> usize {
        self.0.lock().unwrap().1.len()
    }
}

impl Upstream for Script {
    async fn send(&self, request: UpstreamRequest) -> UpstreamReply {
        let mut state = self.0.lock().unwrap();
        state.1.push(request);
        let (status, body) = if state.0.len() > 1 {
            state.0.pop_front().unwrap()
        } else {
            state.0.front().cloned().unwrap()
        };
        Ok((status, body.to_string()))
    }
}

const SINCE: &str = "2026-09-21T12:00:00Z";
const RECENT: &str = "2026-09-27T10:00:00+0000";
const OLD: &str = "2026-09-10T10:00:00+0000";

fn ms(iso: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(iso)
        .unwrap()
        .timestamp_millis()
}

fn recent_seconds() -> i64 {
    ms("2026-09-27T10:00:00Z") / 1000
}

async fn db_with(rows: &'static str) -> common::TestDb {
    let db = fresh_db().await;
    setup(&db, move |tx| {
        tx.execute_batch(
            r#"INSERT INTO "User" ("id", "email", "name", "emailVerified") VALUES (1, 'a@x.test', 'A', 1);
               INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (10, 'ws', 1);
               INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES
                 (30, 10, 'A', 'messenger', 's30'), (31, 10, 'B', 'messenger', 's31');"#,
        )?;
        Ok(tx.execute_batch(rows)?)
    })
    .await;
    db
}

fn comments(scanned: &[chatbotx_business::comment_automation::Scanned]) -> Vec<Value> {
    scanned.iter().map(|s| s.comment.clone()).collect()
}

const PAGES: &str = r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId") VALUES
    (1, '{}', '999', 'Other', '[]', '[]', '[]', 10, 30),
    (2, '{"tokens":{"accessToken":"token"}}', '111', 'Page', '[]', '[]', '[]', 10, 31);"#;

#[tokio::test]
async fn messenger_maps_the_webhook_shape_and_skips_the_page_and_authorless() {
    let db = db_with(
        r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId") VALUES
           (2, '{"tokens":{"accessToken":"token"}}', '111', 'Page', '[]', '[]', '[]', 10, 31);"#,
    )
    .await;
    let up = Script::new(vec![(
        200,
        json!({ "data": [
            { "id": "222_1", "message": "price?", "created_time": RECENT, "from": { "id": "user-1", "name": "Ann" },
              "message_tags": [{ "id": "user-9", "name": "Bob" }, { "name": "no id" }] },
            { "id": "222_2", "created_time": RECENT, "from": { "id": "user-2" }, "parent": { "id": "222_1" } },
            { "id": "222_3", "created_time": RECENT, "from": { "id": "111" } },
            { "id": "222_4", "created_time": RECENT },
        ] }),
    )]);
    let out = scan_post_comments(&db, &up, 10, "messenger", "222", ms(SINCE), None)
        .await
        .unwrap();
    assert!(
        up.urls()[0].starts_with("https://graph.facebook.com/v23.0/111_222/comments?"),
        "{:?}",
        up.urls()
    );
    assert_eq!(out[0].integration_identifier, "111");
    assert_eq!(
        comments(&out),
        vec![
            json!({ "commentId": "222_1", "postId": "111_222", "parentId": "111_222", "fromId": "user-1",
                    "fromName": "Ann", "message": "price?", "tags": [{ "id": "user-9", "name": "Bob" }],
                    "createdTime": recent_seconds() }),
            json!({ "commentId": "222_2", "postId": "111_222", "parentId": "222_1", "fromId": "user-2",
                    "createdTime": recent_seconds() }),
        ]
    );
}

#[tokio::test]
async fn messenger_uses_the_named_page_and_stops_at_the_first_old_comment() {
    let db = db_with(PAGES).await;
    let up = Script::new(vec![(
        200,
        json!({ "data": [
            { "id": "222_1", "created_time": RECENT, "from": { "id": "user-1" } },
            { "id": "222_2", "created_time": OLD, "from": { "id": "user-2" } },
        ], "paging": { "cursors": { "after": "next" }, "next": "https://graph/next" } }),
    )]);
    let out = scan_post_comments(&db, &up, 10, "messenger", "111_222", ms(SINCE), None)
        .await
        .unwrap();
    assert_eq!(up.requests(), 1);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].comment_id, "222_1");
}

#[tokio::test]
async fn messenger_tries_the_next_page_and_keeps_to_a_scoped_account() {
    let db = db_with(PAGES).await;
    let up = Script::new(vec![
        (
            400,
            json!({ "error": { "message": "Unsupported get request" } }),
        ),
        (
            200,
            json!({ "data": [{ "id": "222_1", "created_time": RECENT, "from": { "id": "user-1" } }] }),
        ),
    ]);
    let out = scan_post_comments(&db, &up, 10, "messenger", "222", ms(SINCE), None)
        .await
        .unwrap();
    let urls = up.urls();
    assert!(urls[0].contains("/999_222/") && urls[1].contains("/111_222/"));
    assert_eq!(out[0].integration_identifier, "111");

    let up = Script::new(vec![(200, json!({ "data": [] }))]);
    scan_post_comments(&db, &up, 10, "messenger", "222", ms(SINCE), Some(31))
        .await
        .unwrap();
    assert_eq!(up.urls().len(), 1);
    assert!(up.urls()[0].contains("/111_222/"));
}

#[tokio::test]
async fn no_connected_or_scoped_account_is_integration_not_found() {
    let db = db_with(PAGES).await;
    let up = Script::new(vec![(200, json!({ "data": [] }))]);
    for (kind, inbox) in [
        ("messenger", Some(99)),
        ("instagram", None),
        ("instagramFacebook", None),
        ("threads", None),
        ("tiktok", None),
    ] {
        let out = scan_post_comments(&db, &up, 10, kind, "111_222", ms(SINCE), inbox).await;
        assert!(
            matches!(out, Err(ScanError::IntegrationNotFound)),
            "{kind}: {out:?}"
        );
    }
    assert_eq!(up.requests(), 0);
}

#[tokio::test]
async fn instagram_flattens_replies_filters_each_by_its_time_and_skips_the_account() {
    let db = db_with(
        r#"INSERT INTO "IntegrationInstagram" ("id", "auth", "igId", "pageId", "name", "username", "conversationStarters", "persistentMenus", "workspaceId", "inboxId", "type") VALUES
           (1, '{"tokens":{"accessToken":"tok"},"metadata":{}}', 'ig-0', 'p0', 'Native', 'n', '[]', '[]', 10, 30, 'instagram'),
           (2, '{"tokens":{"accessToken":"tok"},"metadata":{}}', 'ig-1', 'p1', 'Shop', 'shop', '[]', '[]', 10, 31, 'facebook');"#,
    )
    .await;
    let up = Script::new(vec![(
        200,
        json!({ "data": [{
            "id": "c-1", "text": "old comment, new reply", "timestamp": OLD,
            "from": { "id": "user-1", "username": "ann" },
            "replies": { "data": [
                { "id": "c-2", "text": "reply", "timestamp": RECENT, "from": { "id": "user-2", "username": "bob" }, "parent_id": "c-1" },
                { "id": "c-3", "timestamp": RECENT, "from": { "id": "ig-1", "username": "shop" }, "parent_id": "c-1" },
            ] },
        }] }),
    )]);
    let out = scan_post_comments(
        &db,
        &up,
        10,
        "instagramFacebook",
        "media-1",
        ms(SINCE),
        None,
    )
    .await
    .unwrap();
    let urls = up.urls();
    assert_eq!(urls.len(), 1, "only the Facebook-login account");
    assert!(urls[0].starts_with("https://graph.facebook.com/v23.0/media-1/comments?"));
    assert_eq!(out[0].integration_identifier, "ig-1");
    assert_eq!(
        comments(&out),
        vec![
            json!({ "commentId": "c-2", "postId": "media-1", "parentId": "c-1", "fromId": "user-2",
                    "fromName": "bob", "fromUsername": "bob", "message": "reply",
                    "createdTime": recent_seconds() })
        ]
    );

    // Instagram Login reads graph.instagram.com.
    let up = Script::new(vec![(200, json!({ "data": [] }))]);
    scan_post_comments(&db, &up, 10, "instagram", "media-1", ms(SINCE), None)
        .await
        .unwrap();
    assert!(up.urls()[0].starts_with("https://graph.instagram.com/v22.0/media-1/comments?"));
}

#[tokio::test]
async fn threads_keys_the_commenter_by_lowercased_username_and_skips_own_replies() {
    let db = db_with(
        r#"INSERT INTO "IntegrationThreads" ("id", "auth", "threadsUserId", "username", "name", "workspaceId", "inboxId") VALUES
           (1, '{"tokens":{"accessToken":"t"},"metadata":{"username":"Shop"}}', 'th-1', 'Shop', 'Shop', 10, 30);"#,
    )
    .await;
    let up = Script::new(vec![(
        200,
        json!({ "data": [
            { "id": "r-1", "text": "hi", "timestamp": RECENT, "username": "Ann.B", "replied_to": { "id": "post-1" } },
            { "id": "r-2", "timestamp": RECENT, "username": "shop" },
            { "id": "r-3", "timestamp": RECENT, "username": "other", "is_reply_owned_by_me": true },
        ] }),
    )]);
    let out = scan_post_comments(&db, &up, 10, "threads", "post-1", ms(SINCE), None)
        .await
        .unwrap();
    assert!(up.urls()[0].starts_with("https://graph.threads.com/v1.0/post-1/conversation?"));
    assert!(up.urls()[0].contains("reverse=true"));
    assert_eq!(out[0].integration_identifier, "th-1");
    assert_eq!(
        comments(&out),
        vec![
            json!({ "commentId": "r-1", "postId": "post-1", "parentId": "post-1", "fromId": "ann.b",
                    "fromName": "Ann.B", "message": "hi", "createdTime": recent_seconds() })
        ]
    );
}

const TIKTOK: &str = r#"INSERT INTO "IntegrationTiktok" ("id", "auth", "openId", "name", "workspaceId", "inboxId") VALUES
    (1, '{"tokens":{"accessToken":"token"},"metadata":{"openId":"open-1"}}', 'open-1', 'Shop', 10, 30);"#;

#[tokio::test]
async fn tiktok_normalizes_the_parent_flattens_replies_and_skips_the_owner() {
    let db = db_with(TIKTOK).await;
    let s = recent_seconds();
    let up = Script::new(vec![(
        200,
        json!({ "code": 0, "data": { "has_more": false, "comments": [{
            "comment_id": "c-1", "video_id": "video-1", "unique_identifier": "ann", "create_time": s,
            "text": "top", "parent_comment_id": "0",
            "reply_list": [
                { "comment_id": "c-2", "video_id": "video-1", "unique_identifier": "bob", "create_time": s, "parent_comment_id": "c-1" },
                { "comment_id": "c-3", "video_id": "video-1", "unique_identifier": "shop", "create_time": s, "parent_comment_id": "c-1", "owner": true },
            ],
        }] } }),
    )]);
    let out = scan_post_comments(&db, &up, 10, "tiktok", "video-1", ms(SINCE), None)
        .await
        .unwrap();
    let request = up.0.lock().unwrap().1[0].clone();
    assert!(
        request.url.starts_with(
            "https://business-api.tiktok.com/open_api/v1.3/business/comment/list/?business_id=open-1&video_id=video-1&include_replies=true"
        ),
        "{}",
        request.url
    );
    assert!(request.url.contains("sort_type=DESC"));
    assert!(
        request
            .headers
            .contains(&("Access-Token".into(), "token".into()))
    );
    assert_eq!(out[0].integration_identifier, "open-1");
    assert_eq!(
        comments(&out),
        vec![
            json!({ "commentId": "c-1", "postId": "video-1", "fromId": "ann", "message": "top", "createdTime": s }),
            json!({ "commentId": "c-2", "postId": "video-1", "parentId": "c-1", "fromId": "bob", "createdTime": s }),
        ]
    );
}

#[tokio::test]
async fn tiktok_stops_paging_at_the_page_cap_and_a_rejection_fails() {
    let db = db_with(TIKTOK).await;
    let up = Script::new(vec![(
        200,
        json!({ "code": 0, "data": { "comments": [], "cursor": 30, "has_more": true } }),
    )]);
    scan_post_comments(&db, &up, 10, "tiktok", "video-1", ms(SINCE), None)
        .await
        .unwrap();
    assert_eq!(up.requests(), 200);
    assert!(up.urls()[1].contains("cursor=30"));

    let up = Script::new(vec![(200, json!({ "code": 40001, "message": "no scope" }))]);
    let out = scan_post_comments(&db, &up, 10, "tiktok", "video-1", ms(SINCE), None).await;
    assert!(matches!(out, Err(ScanError::Fetch(m)) if m == "no scope"));
}
