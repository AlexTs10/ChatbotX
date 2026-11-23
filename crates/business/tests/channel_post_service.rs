//! `channelPostService.resolveForComment` and
//! `contactInboxPostService.recordComment` (Q.P7-V5): the
//! `channel-post.service.test.ts` / `contact-inbox-post.service.test.ts`
//! cases of the comment path, on a real database. Not here: the picker
//! listing cases (`listFilterOptions`, `rpc_channel_post_options`), the
//! hanging-fetch timeout (15 s of real time) and the thumbnail upload (no
//! file store in this test: the thumbnail stays `None`).

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};

use chatbotx_business::channel_post::{
    CommentPost, PostDetails, record_comment, resolve_for_comment,
};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use common::{WS, contact, contact_inbox, inbox, rows, setup, two_workspaces};

const AT: i64 = 1_790_000_000_000;

fn post(channel: &str) -> CommentPost {
    CommentPost {
        channel: channel.into(),
        workspace_id: WS,
        inbox_id: 5,
        integration_id: 50,
        source_account_id: "account-1".into(),
        external_post_id: "1780".into(),
    }
}

async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        inbox(tx, 5, WS, "instagram")?;
        inbox(tx, 6, WS, "messenger")?;
        Ok(())
    })
    .await;
    db
}

/// Counts the calls; answers `details` (or fails with "Graph unavailable").
struct Fetch(AtomicUsize, Option<PostDetails>);

impl Fetch {
    fn new(details: Option<PostDetails>) -> Fetch {
        Fetch(AtomicUsize::new(0), details)
    }
    async fn call(&self) -> Result<PostDetails, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        self.1.clone().ok_or_else(|| "Graph unavailable".to_owned())
    }
    fn calls(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

async fn resolve(db: &Db, input: &CommentPost, fetch: &Fetch) -> Option<i64> {
    resolve_for_comment(db, None, input, || fetch.call())
        .await
        .expect("resolve")
}

async fn post_rows(db: &Db) -> Vec<serde_json::Map<String, serde_json::Value>> {
    rows(
        db,
        r#"SELECT "id", "channel", "integrationId", "caption", "mediaType", "permalink", "publishedAt",
                  "thumbnail", "metadataFetchedAt", "metadataAttemptedAt"
           FROM "ChannelPost" WHERE "workspaceId" = ?1 ORDER BY "channel""#,
        WS,
    )
    .await
}

fn details() -> PostDetails {
    PostDetails {
        caption: Some("A post".into()),
        media_type: Some("IMAGE".into()),
        permalink: Some("https://instagram.com/p/1".into()),
        published_at: Some(AT),
        thumbnail_url: Some("https://cdn.example/photo.jpg".into()),
    }
}

#[tokio::test]
async fn records_the_bare_post_even_when_metadata_hydration_fails() {
    let db = seeded().await;
    let fetch = Fetch::new(None);
    let id = resolve(&db, &post("instagram"), &fetch).await;
    assert!(id.is_some());
    assert_eq!(fetch.calls(), 1);
    let rows = post_rows(&db).await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0]["metadataFetchedAt"].is_null());
    assert!(
        rows[0]["metadataAttemptedAt"].is_i64(),
        "the attempt is marked"
    );
}

#[tokio::test]
async fn keeps_text_metadata_when_the_thumbnail_is_not_rehosted() {
    let db = seeded().await;
    let fetch = Fetch::new(Some(details()));
    resolve(&db, &post("instagram"), &fetch).await;
    let row = &post_rows(&db).await[0];
    assert_eq!(row["caption"], "A post");
    assert_eq!(row["mediaType"], "IMAGE");
    assert_eq!(row["permalink"], "https://instagram.com/p/1");
    assert_eq!(row["publishedAt"], AT);
    assert!(row["thumbnail"].is_null());
    assert!(row["metadataFetchedAt"].is_i64(), "not retried again");
}

#[tokio::test]
async fn does_not_record_a_post_after_its_workspace_was_removed() {
    let db = seeded().await;
    let mut gone = post("instagram");
    gone.workspace_id = 9999;
    let fetch = Fetch::new(Some(details()));
    assert_eq!(resolve(&db, &gone, &fetch).await, None);
    assert_eq!(fetch.calls(), 0);
}

#[tokio::test]
async fn namespaces_post_lookups_and_inserts_by_channel() {
    let db = seeded().await;
    let fetch = Fetch::new(Some(PostDetails::default()));
    let instagram = resolve(&db, &post("instagram"), &fetch).await;
    let messenger = resolve(&db, &post("messenger"), &fetch).await;
    assert_ne!(
        instagram, messenger,
        "the same bare id on another channel is its own row"
    );
    let channels: Vec<String> = post_rows(&db)
        .await
        .iter()
        .map(|r| r["channel"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(channels, vec!["instagram", "messenger"]);
}

#[tokio::test]
async fn a_hydrated_same_integration_post_is_not_fetched_again() {
    let db = seeded().await;
    let fetch = Fetch::new(Some(details()));
    let first = resolve(&db, &post("instagram"), &fetch).await;
    let again = resolve(&db, &post("instagram"), &fetch).await;
    assert_eq!(first, again);
    assert_eq!(fetch.calls(), 1);
}

#[tokio::test]
async fn claims_a_missing_metadata_post_after_reconnecting_its_integration() {
    let db = seeded().await;
    let failing = Fetch::new(None);
    let id = resolve(&db, &post("instagram"), &failing).await.unwrap();
    // Within the 10-minute retry interval: no second fetch.
    resolve(&db, &post("instagram"), &failing).await;
    assert_eq!(failing.calls(), 1);

    setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "ChannelPost" SET "metadataAttemptedAt" = "metadataAttemptedAt" - 11 * 60 * 1000 WHERE "id" = ?1"#,
            params![id],
        )?;
        Ok(())
    })
    .await;
    let mut reconnected = post("instagram");
    reconnected.integration_id = 51;
    let fetch = Fetch::new(Some(PostDetails {
        caption: Some("A post".into()),
        ..PostDetails::default()
    }));
    assert_eq!(resolve(&db, &reconnected, &fetch).await, Some(id));
    assert_eq!(fetch.calls(), 1);
    let row = &post_rows(&db).await[0];
    assert_eq!(row["integrationId"], 51);
    assert_eq!(row["caption"], "A post");
    assert!(row["mediaType"].is_null() && row["permalink"].is_null());
    assert!(row["metadataFetchedAt"].is_i64());
}

#[tokio::test]
async fn a_post_older_than_a_day_is_not_retried() {
    let db = seeded().await;
    let failing = Fetch::new(None);
    let id = resolve(&db, &post("instagram"), &failing).await.unwrap();
    setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "ChannelPost" SET "createdAt" = "createdAt" - 25 * 3600 * 1000,
                   "metadataAttemptedAt" = "metadataAttemptedAt" - 3600 * 1000 WHERE "id" = ?1"#,
            params![id],
        )?;
        Ok(())
    })
    .await;
    resolve(&db, &post("instagram"), &failing).await;
    assert_eq!(failing.calls(), 1);
}

// ---- recordComment -------------------------------------------------------------------

async fn links(db: &Db) -> Vec<(i64, i64)> {
    rows(
        db,
        r#"SELECT "contactInboxId", "postId" FROM "ContactInboxPost" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await
    .iter()
    .map(|r| {
        (
            r["contactInboxId"].as_i64().unwrap(),
            r["postId"].as_i64().unwrap(),
        )
    })
    .collect()
}

#[tokio::test]
async fn records_a_comment_once_and_only_on_the_posts_own_channel() {
    let db = seeded().await;
    setup(&db, |tx| {
        contact(tx, 10, WS)?;
        contact_inbox(tx, 100, 10, 5, "instagram", "ig-user")?;
        contact_inbox(tx, 101, 10, 6, "messenger", "psid")?;
        Ok(())
    })
    .await;
    let fetch = Fetch::new(Some(PostDetails::default()));
    let ig_post = resolve(&db, &post("instagram"), &fetch).await.unwrap();

    assert!(record_comment(&db, WS, 5, 100, ig_post, AT).await.unwrap());
    assert!(
        !record_comment(&db, WS, 5, 100, ig_post, AT).await.unwrap(),
        "ON CONFLICT DO NOTHING"
    );
    assert!(
        !record_comment(&db, WS, 6, 101, ig_post, AT).await.unwrap(),
        "a Messenger contact inbox never links to an Instagram post"
    );
    assert!(
        !record_comment(&db, common::OTHER_WS, 5, 100, ig_post, AT)
            .await
            .unwrap(),
        "another workspace's inbox"
    );
    assert_eq!(links(&db).await, vec![(100, ig_post)]);
}
