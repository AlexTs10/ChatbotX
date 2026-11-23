//! `syncChannelLabels` (Q.P7-V5): `sync-channel-labels.test.ts`, the import of
//! the labels already on a Messenger page / Zalo OA as workspace tags. The
//! one-error-log-per-scan rule needs the event buses (none in this test).

mod channel_fake;
mod common;

use std::time::Duration;

use channel_fake::{FakeHttp, channel_row, graph_error, ok, status};
use chatbotx_business::tag_sync::sync_channel_labels;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::Connection;
use common::{WS, contact, contact_inbox, inbox, rows, setup, two_workspaces};
use serde_json::{Value, json};

const PAGE: i64 = 100;
const OA: i64 = 200;

fn auth() -> Value {
    json!({ "tokens": { "accessToken": "tok" } })
}

/// Contacts `1..=n` with a contact inbox `1000 + i` (source `user-i`) on
/// `integration`.
fn contacts_on(
    tx: &Connection,
    integration: i64,
    channel: &str,
    n: i64,
) -> Result<(), chatbotx_db::DbError> {
    for i in 1..=n {
        contact(tx, i, WS)?;
        contact_inbox(tx, 1000 + i, i, integration, channel, &format!("user-{i}"))?;
    }
    Ok(())
}

async fn seeded(n: i64) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        channel_row(tx, "IntegrationMessenger", PAGE, WS, "page-1", &auth())?;
        channel_row(tx, "IntegrationZalo", OA, WS, "oa-1", &auth())?;
        contacts_on(tx, PAGE, "messenger", n)
    })
    .await;
    db
}

async fn scan(db: &Db, http: &FakeHttp, channel: &str, integration: i64) {
    let data = json!({
        "workspaceId": WS.to_string(),
        "channelType": channel,
        "integrationId": integration.to_string(),
    });
    sync_channel_labels(db, http, None, &data, Duration::ZERO)
        .await
        .expect("scan resolves");
}

/// `(tag name, channel, integration, externalLabelId, contactInboxId, contactId)`.
async fn mappings(db: &Db) -> Vec<(String, String, i64, String, i64, i64)> {
    rows(
        db,
        r#"SELECT t."name", tc."channelType", tc."integrationId", tc."externalLabelId", l."contactInboxId", ct."contactId"
           FROM "ContactToTagChannel" l
           JOIN "TagChannel" tc ON tc."id" = l."tagChannelId"
           JOIN "Tag" t ON t."id" = l."tagId"
           JOIN "ContactInbox" ci ON ci."id" = l."contactInboxId"
           JOIN "ContactToTag" ct ON ct."tagId" = t."id" AND ct."contactId" = ci."contactId"
           WHERE t."workspaceId" = ?1 AND tc."workspaceId" = ?1
           ORDER BY l."contactInboxId", t."name""#,
        WS,
    )
    .await
    .into_iter()
    .map(|r| {
        (
            r["name"].as_str().unwrap().to_owned(),
            r["channelType"].as_str().unwrap().to_owned(),
            r["integrationId"].as_i64().unwrap(),
            r["externalLabelId"].as_str().unwrap().to_owned(),
            r["contactInboxId"].as_i64().unwrap(),
            r["contactId"].as_i64().unwrap(),
        )
    })
    .collect()
}

fn labels(pairs: &[(&str, &str)]) -> Value {
    let data: Vec<Value> = pairs
        .iter()
        .map(|(id, name)| json!({ "id": id, "page_label_name": name }))
        .collect();
    json!({ "data": data })
}

// ---- routing -------------------------------------------------------------------

#[tokio::test]
async fn a_missing_integration_returns_without_scanning() {
    let db = seeded(2).await;
    let http = FakeHttp::new(|_| ok(labels(&[("l1", "VIP")])));
    scan(&db, &http, "messenger", 999).await;
    scan(&db, &http, "zalo", 999).await;
    assert!(http.seen().is_empty());
    assert!(mappings(&db).await.is_empty());
}

#[tokio::test]
async fn messenger_scan_lists_the_labels_of_each_contact_by_source_id() {
    let db = seeded(2).await;
    let http = FakeHttp::new(|_| ok(labels(&[])));
    scan(&db, &http, "messenger", PAGE).await;
    let urls = http.urls();
    assert_eq!(urls.len(), 2, "one listLabels per contact: {urls:?}");
    assert!(
        urls[0].contains("/user-1/custom_labels?fields=id%2Cpage_label_name"),
        "{urls:?}"
    );
    assert!(urls[1].contains("/user-2/custom_labels"), "{urls:?}");
    assert!(urls[0].contains(&format!(
        "/{}/",
        chatbotx_business::channel_api::META_DEFAULT_VERSION
    )));
    assert!(mappings(&db).await.is_empty(), "[] labels → no upsert");
}

#[tokio::test]
async fn each_messenger_label_becomes_a_mapping_with_the_fb_label_id() {
    let db = seeded(1).await;
    let http = FakeHttp::new(|_| ok(labels(&[("lbl-1", "VIP"), ("lbl-2", "Lead")])));
    scan(&db, &http, "messenger", PAGE).await;
    assert_eq!(
        mappings(&db).await,
        vec![
            (
                "Lead".into(),
                "messenger".into(),
                PAGE,
                "lbl-2".into(),
                1001,
                1
            ),
            (
                "VIP".into(),
                "messenger".into(),
                PAGE,
                "lbl-1".into(),
                1001,
                1
            ),
        ]
    );
}

#[tokio::test]
async fn one_failing_user_does_not_stop_the_scan_and_all_failing_still_resolves() {
    let db = seeded(3).await;
    let http = FakeHttp::new(|r| {
        if r.url.contains("user-2") {
            graph_error(500, "down", 2, None)
        } else {
            ok(labels(&[("lbl-1", "VIP")]))
        }
    });
    scan(&db, &http, "messenger", PAGE).await;
    let inboxes: Vec<i64> = mappings(&db).await.iter().map(|m| m.4).collect();
    assert_eq!(inboxes, vec![1001, 1003]);

    let failing = FakeHttp::new(|_| graph_error(500, "down", 2, None));
    scan(&db, &failing, "messenger", PAGE).await;
    assert_eq!(failing.seen().len(), 3);
}

#[tokio::test]
async fn the_scan_pages_by_id_and_stays_on_the_integrations_inbox() {
    let db = seeded(101).await;
    setup(&db, |tx| {
        inbox(tx, 300, WS, "messenger")?;
        contact(tx, 500, WS)?;
        contact_inbox(tx, 5000, 500, 300, "messenger", "other-page-user")?;
        Ok(())
    })
    .await;
    let http = FakeHttp::new(|_| ok(labels(&[])));
    scan(&db, &http, "messenger", PAGE).await;
    let urls = http.urls();
    assert_eq!(urls.len(), 101, "both chunks processed");
    assert!(urls[100].contains("/user-101/"));
    assert!(!urls.iter().any(|u| u.contains("other-page-user")));
}

// ---- zalo ----------------------------------------------------------------------

async fn zalo_db() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        channel_row(tx, "IntegrationZalo", OA, WS, "oa-1", &auth())?;
        contacts_on(tx, OA, "zalo", 4)
    })
    .await;
    db
}

#[tokio::test]
async fn zalo_scan_reads_the_tag_names_of_each_user_detail() {
    let db = zalo_db().await;
    let http = FakeHttp::new(|r| {
        let detail = if r.url.contains("user-1") {
            json!({ "tags_and_notes_info": null })
        } else if r.url.contains("user-2") {
            json!({ "tags_and_notes_info": {} })
        } else if r.url.contains("user-3") {
            json!({ "tags_and_notes_info": { "tag_names": ["VIP", "Lead"] } })
        } else {
            return status(200, json!({ "error": -201, "message": "user not found" }));
        };
        ok(json!({ "error": 0, "data": detail }))
    });
    scan(&db, &http, "zalo", OA).await;
    let urls = http.urls();
    assert_eq!(urls.len(), 4);
    assert!(urls[0].contains("/v3.0/oa/user/detail"), "{urls:?}");
    assert!(urls[0].contains("user_id%22%3A%22user-1"), "{urls:?}");
    assert_eq!(
        mappings(&db).await,
        vec![
            ("Lead".into(), "zalo".into(), OA, "Lead".into(), 1003, 3),
            ("VIP".into(), "zalo".into(), OA, "VIP".into(), 1003, 3),
        ]
    );
}
