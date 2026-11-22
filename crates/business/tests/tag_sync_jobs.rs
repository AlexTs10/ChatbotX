//! `syncTag` (Q.P7-V5): `sync-tag.test.ts`, `sync-tag-create-attach.test.ts`,
//! `sync-tag-detach.test.ts` and `sync-tag-delete.test.ts` against a scripted
//! Graph / Zalo upstream. The mock-call assertions of the originals are
//! checked on the requests sent and the rows left behind.

mod channel_fake;
mod common;

use channel_fake::{FakeHttp, channel_row, graph_error, ok, text};
use chatbotx_business::tag_sync::sync_tag;
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};
use common::{WS, contact, contact_inbox, inbox, rows, setup, tag, tag_link, two_workspaces};
use serde_json::{Value, json};

const TAG: i64 = 1;
const PAGE: i64 = 100;
const PAGE_2: i64 = 101;
const OA: i64 = 200;
const ADA: i64 = 10;
const BEN: i64 = 11;
const ADA_FB: i64 = 500;
const ADA_ZALO: i64 = 501;
const BEN_FB: i64 = 502;

fn auth() -> Value {
    json!({ "tokens": { "accessToken": "tok" }, "version": "v23.0" })
}

fn enable(tx: &Connection, table: &str, id: i64) -> Result<(), DbError> {
    tx.execute(
        &format!(r#"UPDATE "{table}" SET "syncTagEnabledAt" = 1 WHERE "id" = ?1"#),
        params![id],
    )?;
    Ok(())
}

fn tag_channel(
    tx: &Connection,
    id: i64,
    channel: &str,
    integration: i64,
    label: &str,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "TagChannel" ("id", "workspaceId", "tagId", "channelType", "integrationId", "externalLabelId")
           VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
        params![id, WS, TAG, channel, integration, label],
    )?;
    Ok(())
}

fn channel_link(tx: &Connection, tag_channel: i64, contact_inbox: i64) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "ContactToTagChannel" ("tagId", "tagChannelId", "contactInboxId") VALUES (?1, ?2, ?3)"#,
        params![TAG, tag_channel, contact_inbox],
    )?;
    Ok(())
}

/// Tag 1 "VIP"; Ada (Messenger page 100 + Zalo OA 200) and Ben (page 100).
/// `enabled` turns tag sync on for page 100 and OA 200.
async fn seeded(enabled: bool) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        tag(tx, TAG, WS, "VIP")?;
        contact(tx, ADA, WS)?;
        contact(tx, BEN, WS)?;
        channel_row(tx, "IntegrationMessenger", PAGE, WS, "page-1", &auth())?;
        channel_row(tx, "IntegrationZalo", OA, WS, "oa-1", &auth())?;
        contact_inbox(tx, ADA_FB, ADA, PAGE, "messenger", "psid-ada")?;
        contact_inbox(tx, ADA_ZALO, ADA, OA, "zalo", "zuid-ada")?;
        contact_inbox(tx, BEN_FB, BEN, PAGE, "messenger", "psid-ben")?;
        if enabled {
            enable(tx, "IntegrationMessenger", PAGE)?;
            enable(tx, "IntegrationZalo", OA)?;
        }
        Ok(())
    })
    .await;
    db
}

/// Graph and Zalo succeed: a new label is `lbl-new`.
fn happy() -> FakeHttp {
    FakeHttp::new(|r| {
        if r.url.contains("/custom_labels") {
            ok(json!({ "id": "lbl-new" }))
        } else if r.url.contains("openapi.zalo.me") {
            ok(json!({ "error": 0 }))
        } else {
            ok(json!({ "success": true }))
        }
    })
}

fn job(action: &str, contact: Option<i64>) -> Value {
    let mut data =
        json!({ "action": action, "workspaceId": WS.to_string(), "tagId": TAG.to_string() });
    if let Some(contact) = contact {
        data["contactId"] = json!(contact.to_string());
    }
    data
}

async fn run(db: &Db, http: &FakeHttp, data: Value) -> bool {
    sync_tag(db, http, None, &data).await.is_ok()
}

fn calls(http: &FakeHttp) -> Vec<String> {
    http.seen()
        .iter()
        .map(|r| format!("{} {}", r.method, text(r)))
        .collect()
}

async fn tag_channels(db: &Db) -> Vec<(String, i64, String)> {
    rows(
        db,
        r#"SELECT "channelType", "integrationId", "externalLabelId" FROM "TagChannel" WHERE "workspaceId" = ?1 ORDER BY "id""#,
        WS,
    )
    .await
    .into_iter()
    .map(|r| {
        (
            r["channelType"].as_str().unwrap().to_owned(),
            r["integrationId"].as_i64().unwrap(),
            r["externalLabelId"].as_str().unwrap().to_owned(),
        )
    })
    .collect()
}

async fn channel_links(db: &Db) -> Vec<i64> {
    rows(
        db,
        r#"SELECT "contactInboxId" FROM "ContactToTagChannel" WHERE "tagId" = ?1 ORDER BY "contactInboxId""#,
        TAG,
    )
    .await
    .into_iter()
    .map(|r| r["contactInboxId"].as_i64().unwrap())
    .collect()
}

async fn tag_rows(db: &Db) -> usize {
    rows(db, r#"SELECT "id" FROM "Tag" WHERE "id" = ?1"#, TAG)
        .await
        .len()
}

async fn tagged_contacts(db: &Db) -> Vec<i64> {
    rows(
        db,
        r#"SELECT "contactId" FROM "ContactToTag" WHERE "tagId" = ?1 ORDER BY "contactId""#,
        TAG,
    )
    .await
    .into_iter()
    .map(|r| r["contactId"].as_i64().unwrap())
    .collect()
}

// ---- dispatch ------------------------------------------------------------------

#[tokio::test]
async fn unknown_action_logs_warn_and_does_not_throw() {
    let db = seeded(true).await;
    let http = happy();
    assert!(run(&db, &http, job("explode", None)).await);
    assert!(http.seen().is_empty());
}

// ---- create --------------------------------------------------------------------

#[tokio::test]
async fn create_tag_not_found_makes_no_sdk_calls() {
    let db = seeded(true).await;
    let http = happy();
    let mut data = job("create", None);
    data["tagId"] = json!("999");
    assert!(run(&db, &http, data).await);
    assert!(http.seen().is_empty());
    assert!(tag_channels(&db).await.is_empty());
}

#[tokio::test]
async fn create_skips_integrations_with_sync_disabled() {
    let db = seeded(false).await;
    let http = happy();
    assert!(run(&db, &http, job("create", None)).await);
    assert!(http.seen().is_empty());
    assert!(tag_channels(&db).await.is_empty());
}

#[tokio::test]
async fn create_calls_create_label_with_page_and_name_and_inserts_the_mapping() {
    let db = seeded(true).await;
    let http = happy();
    assert!(run(&db, &http, job("create", None)).await);
    let calls = calls(&http);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].starts_with("POST "), "{calls:?}");
    assert!(
        calls[0].contains("/v23.0/page-1/custom_labels?page_label_name=VIP"),
        "{calls:?}"
    );
    assert_eq!(
        tag_channels(&db).await,
        vec![
            ("messenger".into(), PAGE, "lbl-new".into()),
            ("zalo".into(), OA, "VIP".into()),
        ]
    );
}

#[tokio::test]
async fn create_always_calls_create_label_and_updates_an_existing_mapping() {
    let db = seeded(true).await;
    setup(&db, |tx| tag_channel(tx, 900, "messenger", PAGE, "lbl-old")).await;
    let http = happy();
    assert!(run(&db, &http, job("create", None)).await);
    assert_eq!(http.seen().len(), 1);
    let channels = tag_channels(&db).await;
    assert_eq!(channels[0], ("messenger".into(), PAGE, "lbl-new".into()));
    assert_eq!(
        channels.iter().filter(|c| c.0 == "messenger").count(),
        1,
        "updated in place, not inserted"
    );
}

#[tokio::test]
async fn create_label_failure_is_caught_and_the_next_integration_still_runs() {
    let db = seeded(true).await;
    setup(&db, |tx| {
        channel_row(tx, "IntegrationMessenger", PAGE_2, WS, "page-2", &auth())?;
        enable(tx, "IntegrationMessenger", PAGE_2)
    })
    .await;
    let http = FakeHttp::new(|r| {
        if r.url.contains("page-1") {
            graph_error(400, "boom", 100, None)
        } else {
            ok(json!({ "id": "lbl-2" }))
        }
    });
    assert!(run(&db, &http, job("create", None)).await);
    assert_eq!(http.seen().len(), 2, "createLabel once per enabled page");
    let channels = tag_channels(&db).await;
    assert!(channels.contains(&("messenger".into(), PAGE_2, "lbl-2".into())));
    assert!(!channels.iter().any(|c| c.1 == PAGE));
}

#[tokio::test]
async fn create_on_zalo_inserts_the_tag_name_as_label_without_an_api_call() {
    let db = seeded(false).await;
    setup(&db, |tx| {
        enable(tx, "IntegrationZalo", OA)?;
        channel_row(tx, "IntegrationZalo", 201, WS, "oa-2", &auth())?;
        enable(tx, "IntegrationZalo", 201)
    })
    .await;
    let http = happy();
    assert!(run(&db, &http, job("create", None)).await);
    assert!(http.seen().is_empty());
    assert_eq!(
        tag_channels(&db).await,
        vec![
            ("zalo".into(), OA, "VIP".into()),
            ("zalo".into(), 201, "VIP".into()),
        ]
    );
}

// ---- attach --------------------------------------------------------------------

#[tokio::test]
async fn attach_with_an_existing_mapping_skips_create_label_and_assigns_it() {
    let db = seeded(true).await;
    setup(&db, |tx| tag_channel(tx, 900, "messenger", PAGE, "lbl-1")).await;
    let http = happy();
    assert!(run(&db, &http, job("attach", Some(BEN))).await);
    let calls = calls(&http);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        calls[0].starts_with("POST ") && calls[0].contains("/v23.0/lbl-1/label?user=psid-ben"),
        "{calls:?}"
    );
    assert_eq!(channel_links(&db).await, vec![BEN_FB]);
}

#[tokio::test]
async fn attach_without_a_mapping_creates_the_label_then_inserts_then_assigns() {
    let db = seeded(true).await;
    let http = happy();
    assert!(run(&db, &http, job("attach", Some(BEN))).await);
    let calls = calls(&http);
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(calls[0].contains("page-1/custom_labels?page_label_name=VIP"));
    assert!(calls[1].contains("lbl-new/label?user=psid-ben"));
    assert_eq!(
        tag_channels(&db).await,
        vec![("messenger".into(), PAGE, "lbl-new".into())]
    );
    assert_eq!(channel_links(&db).await, vec![BEN_FB]);
}

#[tokio::test]
async fn attach_skips_integrations_with_sync_disabled() {
    let db = seeded(false).await;
    let http = happy();
    assert!(run(&db, &http, job("attach", Some(ADA))).await);
    assert!(http.seen().is_empty());
    assert!(channel_links(&db).await.is_empty());
}

#[tokio::test]
async fn attach_on_zalo_calls_tag_follower_and_upserts_the_mapping() {
    let db = seeded(false).await;
    setup(&db, |tx| enable(tx, "IntegrationZalo", OA)).await;
    let http = happy();
    assert!(run(&db, &http, job("attach", Some(ADA))).await);
    let seen = http.seen();
    assert_eq!(seen.len(), 1);
    assert!(
        seen[0].url.ends_with("/v2.0/oa/tag/tagfollower"),
        "{}",
        seen[0].url
    );
    let body: Value = serde_json::from_str(seen[0].body.as_deref().unwrap()).unwrap();
    assert_eq!(body, json!({ "user_id": "zuid-ada", "tag_name": "VIP" }));
    assert_eq!(
        tag_channels(&db).await,
        vec![("zalo".into(), OA, "VIP".into())]
    );
    assert_eq!(channel_links(&db).await, vec![ADA_ZALO]);
}

#[tokio::test]
async fn attach_routes_messenger_and_zalo_inboxes_independently() {
    let db = seeded(true).await;
    let http = happy();
    assert!(run(&db, &http, job("attach", Some(ADA))).await);
    let urls = http.urls();
    assert!(
        urls.iter().any(|u| u.contains("label?user=psid-ada")),
        "{urls:?}"
    );
    assert!(urls.iter().any(|u| u.ends_with("tagfollower")), "{urls:?}");
    assert_eq!(channel_links(&db).await, vec![ADA_FB, ADA_ZALO]);
}

#[tokio::test]
async fn attach_without_channel_inboxes_or_tag_makes_no_calls() {
    let db = seeded(true).await;
    setup(&db, |tx| {
        contact(tx, 12, WS)?;
        inbox(tx, 300, WS, "webchat")?;
        contact_inbox(tx, 503, 12, 300, "webchat", "guest-12")?;
        Ok(())
    })
    .await;
    let http = happy();
    assert!(run(&db, &http, job("attach", Some(12))).await);
    assert!(run(&db, &http, job("attach", Some(99))).await);
    let mut missing = job("attach", Some(ADA));
    missing["tagId"] = json!("999");
    assert!(run(&db, &http, missing).await);
    assert!(http.seen().is_empty());
}

#[tokio::test]
async fn attach_rethrows_an_assign_failure_and_leaves_no_link() {
    let db = seeded(true).await;
    setup(&db, |tx| tag_channel(tx, 900, "messenger", PAGE, "lbl-1")).await;
    let http = FakeHttp::new(|_| graph_error(500, "down", 2, None));
    assert!(!run(&db, &http, job("attach", Some(BEN))).await);
    assert!(channel_links(&db).await.is_empty());
}

// ---- detach --------------------------------------------------------------------

#[tokio::test]
async fn detach_without_mapping_rows_makes_no_calls() {
    let db = seeded(true).await;
    let http = happy();
    assert!(run(&db, &http, job("detach", Some(ADA))).await);
    assert!(http.seen().is_empty());
}

#[tokio::test]
async fn detach_removes_the_label_on_both_channels_then_unlinks() {
    let db = seeded(true).await;
    setup(&db, |tx| {
        tag_channel(tx, 900, "messenger", PAGE, "lbl-1")?;
        tag_channel(tx, 901, "zalo", OA, "VIP")?;
        channel_link(tx, 900, ADA_FB)?;
        channel_link(tx, 901, ADA_ZALO)?;
        channel_link(tx, 900, BEN_FB)
    })
    .await;
    let http = happy();
    assert!(run(&db, &http, job("detach", Some(ADA))).await);
    let calls = calls(&http);
    assert_eq!(calls.len(), 2, "one call per row: {calls:?}");
    assert!(
        calls
            .iter()
            .any(|c| c.starts_with("DELETE ") && c.contains("/lbl-1/label?user=psid-ada"))
    );
    assert!(calls.iter().any(|c| c.contains("rmfollowerfromtag")
        && c.contains(r#""user_id":"zuid-ada""#)
        && c.contains(r#""tag_name":"VIP""#)));
    // Scoped by tagChannelId AND contactInboxId: Ben keeps his link.
    assert_eq!(channel_links(&db).await, vec![BEN_FB]);
}

#[tokio::test]
async fn detach_unlinks_even_when_the_api_throws_and_continues_to_the_next_row() {
    let db = seeded(true).await;
    setup(&db, |tx| {
        tag_channel(tx, 900, "messenger", PAGE, "lbl-1")?;
        tag_channel(tx, 901, "zalo", OA, "VIP")?;
        channel_link(tx, 900, ADA_FB)?;
        channel_link(tx, 901, ADA_ZALO)
    })
    .await;
    let http = FakeHttp::new(|r| {
        if r.url.contains("graph.facebook.com") {
            graph_error(500, "down", 2, None)
        } else {
            ok(json!({ "error": 0 }))
        }
    });
    assert!(run(&db, &http, job("detach", Some(ADA))).await);
    assert_eq!(http.seen().len(), 2);
    assert!(channel_links(&db).await.is_empty());
}

#[tokio::test]
async fn detach_with_sync_disabled_skips_the_api_but_still_unlinks() {
    let db = seeded(false).await;
    setup(&db, |tx| {
        tag_channel(tx, 900, "messenger", PAGE, "lbl-1")?;
        tag_channel(tx, 901, "zalo", OA, "VIP")?;
        channel_link(tx, 900, ADA_FB)?;
        channel_link(tx, 901, ADA_ZALO)
    })
    .await;
    let http = happy();
    assert!(run(&db, &http, job("detach", Some(ADA))).await);
    assert!(http.seen().is_empty());
    assert!(channel_links(&db).await.is_empty());
}

// ---- delete --------------------------------------------------------------------

async fn mapped_and_soft_deleted(db: &Db) {
    setup(db, |tx| {
        tag_channel(tx, 900, "messenger", PAGE, "lbl-1")?;
        tag_channel(tx, 901, "zalo", OA, "VIP")?;
        channel_link(tx, 900, ADA_FB)?;
        channel_link(tx, 901, ADA_ZALO)?;
        channel_link(tx, 900, BEN_FB)?;
        tag_link(tx, ADA, TAG)?;
        tag_link(tx, BEN, TAG)?;
        tx.execute(
            r#"UPDATE "Tag" SET "deletedAt" = 1 WHERE "id" = ?1"#,
            params![TAG],
        )?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn delete_calls_no_channel_api_and_removes_every_channel_then_the_tag() {
    let db = seeded(true).await;
    mapped_and_soft_deleted(&db).await;
    let http = happy();
    assert!(run(&db, &http, job("delete", None)).await);
    assert!(http.seen().is_empty(), "label API temporarily disabled");
    assert!(channel_links(&db).await.is_empty());
    assert!(tag_channels(&db).await.is_empty());
    assert!(tagged_contacts(&db).await.is_empty());
    assert_eq!(tag_rows(&db).await, 0, "hard-deleted last");
}

#[tokio::test]
async fn delete_catch_all_removes_manually_applied_contact_tags() {
    let db = seeded(false).await;
    setup(&db, |tx| {
        contact(tx, 12, WS)?;
        tag_link(tx, 12, TAG)?;
        tx.execute(
            r#"UPDATE "Tag" SET "deletedAt" = 1 WHERE "id" = ?1"#,
            params![TAG],
        )?;
        Ok(())
    })
    .await;
    let http = happy();
    assert!(run(&db, &http, job("delete", None)).await);
    assert!(tagged_contacts(&db).await.is_empty());
    assert_eq!(tag_rows(&db).await, 0);
}

#[tokio::test]
async fn delete_without_channels_or_links_only_deletes_the_tag() {
    let db = seeded(false).await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "Tag" SET "deletedAt" = 1 WHERE "id" = ?1"#,
            params![TAG],
        )?;
        Ok(())
    })
    .await;
    let http = happy();
    assert!(run(&db, &http, job("delete", None)).await);
    assert!(http.seen().is_empty());
    assert_eq!(tag_rows(&db).await, 0);
}

#[tokio::test]
async fn channel_scoped_delete_removes_only_that_channel_and_keeps_the_tag() {
    let db = seeded(true).await;
    mapped_and_soft_deleted(&db).await;
    let http = happy();
    let mut data = job("delete", None);
    data["channelType"] = json!("messenger");
    data["integrationId"] = json!(PAGE.to_string());
    assert!(run(&db, &http, data).await);
    assert!(http.seen().is_empty());
    assert_eq!(
        tag_channels(&db).await,
        vec![("zalo".into(), OA, "VIP".into())]
    );
    assert_eq!(channel_links(&db).await, vec![ADA_ZALO]);
    assert!(
        tagged_contacts(&db).await.is_empty(),
        "the contacts tagged through the page lose the tag"
    );
    assert_eq!(tag_rows(&db).await, 1, "the Tag row stays");
}

#[tokio::test]
async fn channel_scoped_delete_is_a_no_op_when_the_tag_is_not_mapped_there() {
    let db = seeded(true).await;
    setup(&db, |tx| tag_link(tx, ADA, TAG)).await;
    let http = happy();
    let mut data = job("delete", None);
    data["channelType"] = json!("zalo");
    data["integrationId"] = json!(OA.to_string());
    assert!(run(&db, &http, data).await);
    assert_eq!(tagged_contacts(&db).await, vec![ADA]);
    assert_eq!(tag_rows(&db).await, 1);
}
