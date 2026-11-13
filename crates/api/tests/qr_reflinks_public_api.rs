//! Ports of `apps/builder/__tests__/qr-codes-public-api.test.ts` and
//! `reflinks-public-api.test.ts` (CRUD and open-chat links). Adapted: the
//! captured oRPC handlers become `handlers::dispatch` calls over a real
//! migrated database; "the service was called with" becomes the stored rows /
//! the response. The chat-widget routes are not ported yet (n-a rows).

mod handlers_common;

use chatbotx_api::routes::scopes_of_module;
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

async fn seed(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            for (id, ws) in [(1_i64, WS), (2, WS), (9, OTHER_WS)] {
                tx.execute(
                    r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (?1, ?2, ?3)"#,
                    params![id, ws, format!("Flow {id}")],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed flows");
}

async fn new_qr(f: &Fixture, name: &str) -> String {
    let (status, body) = call(
        f,
        "qrCodes.create",
        WS,
        &[],
        None,
        Some(json!({ "name": name, "flowId": "1" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_str().expect("id").to_string()
}

async fn new_reflink(f: &Fixture, ws: i64, name: &str, flow: &str) -> Value {
    let (status, body) = call(
        f,
        "reflinks.create",
        ws,
        &[],
        None,
        Some(json!({ "name": name, "flowId": flow, "customFieldId": null })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    body
}

// --- QR codes ---------------------------------------------------------------

#[tokio::test]
async fn registers_the_qr_codes_public_router_under_the_automation_scope() {
    assert_eq!(
        scopes_of_module("features/qr-codes/api/public"),
        vec!["automation"]
    );
}

#[tokio::test]
async fn get_v1_qr_codes_lists_qr_codes_in_the_token_workspace() {
    let f = fixture().await;
    seed(&f).await;
    let id = new_qr(&f, "promo").await;
    let (status, body) = call(&f, "qrCodes.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (&body["data"][0]["id"], &body["data"][0]["flow"]),
        (&json!(id), &json!({ "id": "1", "name": "Flow 1" }))
    );
    assert_eq!(body["pageCount"], json!(1));
    let (_, other) = call(&f, "qrCodes.list", OTHER_WS, &[], None, None).await;
    assert_eq!(other["data"], json!([]));
}

#[tokio::test]
async fn get_v1_qr_codes_id_gets_a_qr_code_from_the_token_workspace() {
    let f = fixture().await;
    seed(&f).await;
    let id = new_qr(&f, "promo").await;
    let (status, body) = call(&f, "qrCodes.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (
            &body["name"],
            &body["type"],
            &body["qrStyles"],
            &body["flowId"]
        ),
        (
            &json!("qr_promo"),
            &json!("qrCode"),
            &json!({ "size": 600 }),
            &json!("1")
        )
    );
    assert!(body.get("workspaceId").is_none());
}

#[tokio::test]
async fn get_v1_qr_codes_id_surfaces_the_service_not_found_error() {
    let f = fixture().await;
    seed(&f).await;
    let id = new_qr(&f, "promo").await;
    let (status, body) = call(&f, "qrCodes.get", OTHER_WS, &[("id", &id)], None, None).await;
    assert_eq!(
        (status, &body["message"], &body["defined"]),
        (404, &json!("QR Code not found"), &json!(true))
    );
}

#[tokio::test]
async fn post_v1_qr_codes_creates_a_qr_code_in_the_token_workspace() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "qrCodes.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "promo", "flowId": "1", "size": 256 })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let id = body["id"].as_str().expect("id").to_string();
    let (_, got) = call(&f, "qrCodes.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(got["qrStyles"], json!({ "size": 256 }));
}

#[tokio::test]
async fn patch_v1_qr_codes_id_updates_a_qr_code_in_the_token_workspace() {
    let f = fixture().await;
    seed(&f).await;
    let id = new_qr(&f, "promo").await;
    let (status, body) = call(
        &f,
        "qrCodes.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": "renamed", "size": 128, "flowId": "2" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (&body["name"], &body["qrStyles"], &body["flowId"]),
        (&json!("qr_renamed"), &json!({ "size": 128 }), &json!("2"))
    );
}

#[tokio::test]
async fn delete_v1_qr_codes_id_deletes_a_qr_code_in_the_token_workspace() {
    let f = fixture().await;
    seed(&f).await;
    let id = new_qr(&f, "promo").await;
    let (status, _) = call(&f, "qrCodes.delete", OTHER_WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204);
    let (status, _) = call(&f, "qrCodes.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200);
    let (status, _) = call(&f, "qrCodes.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204);
    let (status, _) = call(&f, "qrCodes.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
}

// --- ref links ------------------------------------------------------------------

#[tokio::test]
async fn registers_the_reflinks_public_router_under_the_automation_scope() {
    assert_eq!(
        scopes_of_module("features/reflinks/api/public"),
        vec!["automation"]
    );
}

#[tokio::test]
async fn get_v1_ref_links_delegates_to_reflink_service_list() {
    let f = fixture().await;
    seed(&f).await;
    let own = new_reflink(&f, WS, "welcome", "1").await;
    new_reflink(&f, OTHER_WS, "foreign", "9").await;
    let (status, body) = call(&f, "reflinks.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (
            body["data"].as_array().map(Vec::len),
            &body["data"][0]["id"],
            &body["pageCount"]
        ),
        (Some(1), &own["id"], &json!(1))
    );
}

#[tokio::test]
async fn get_v1_ref_links_forwards_keyword_and_sort() {
    let f = fixture().await;
    seed(&f).await;
    new_reflink(&f, WS, "alpha", "1").await;
    new_reflink(&f, WS, "promoB", "1").await;
    new_reflink(&f, WS, "promoA", "1").await;
    let query = "keyword=PROMO&sort[0][id]=name&sort[0][desc]=false";
    let (status, body) = call(&f, "reflinks.list", WS, &[], Some(query), None).await;
    assert_eq!(status, 200, "{body}");
    let names: Vec<&str> = body["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|r| r["name"].as_str().expect("name"))
        .collect();
    assert_eq!(names, vec!["promoA", "promoB"]);
}

#[tokio::test]
async fn get_v1_ref_links_id_delegates_to_reflink_service_find_or_fail() {
    let f = fixture().await;
    seed(&f).await;
    let created = new_reflink(&f, WS, "welcome", "1").await;
    let id = created["id"].as_str().expect("id").to_string();
    let (status, body) = call(&f, "reflinks.get", WS, &[("id", &id)], None, None).await;
    assert_eq!((status, &body), (200, &created));
    let (status, _) = call(&f, "reflinks.get", OTHER_WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn post_v1_ref_links_delegates_to_reflink_service_create() {
    let f = fixture().await;
    seed(&f).await;
    let created = new_reflink(&f, WS, "welcome", "1").await;
    assert_eq!(
        (
            &created["name"],
            &created["type"],
            &created["flowId"],
            &created["customFieldId"],
            &created["links"]
        ),
        (
            &json!("welcome"),
            &json!("refLink"),
            &json!("1"),
            &Value::Null,
            &json!([])
        )
    );
}

#[tokio::test]
async fn patch_v1_ref_links_id_delegates_to_reflink_service_update() {
    let f = fixture().await;
    seed(&f).await;
    let created = new_reflink(&f, WS, "welcome", "1").await;
    let id = created["id"].as_str().expect("id").to_string();
    let (status, body) = call(
        &f,
        "reflinks.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": "hello" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (&body["name"], &body["flowId"]),
        (&json!("hello"), &json!("1"))
    );
}

#[tokio::test]
async fn delete_v1_ref_links_id_delegates_to_reflink_service_delete_many() {
    let f = fixture().await;
    seed(&f).await;
    let created = new_reflink(&f, WS, "welcome", "1").await;
    let id = created["id"].as_str().expect("id").to_string();
    let (status, _) = call(&f, "reflinks.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204);
    let (status, _) = call(&f, "reflinks.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
}

async fn seed_inboxes(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            let rows: [(i64, &str, &str, &str, &str); 6] = [
                (101, "messenger", "page-1", "My Page", "connected"),
                (102, "zalo", "oa-1", "OA", "connected"),
                (103, "smtp", "mail-1", "Mail", "connected"),
                (104, "tiktok", "acme.shop", "TT", "connected"),
                (105, "threads", "1789", "Threads", "connected"),
                (106, "messenger", "page-2", "Gone", "disconnected"),
            ];
            for (id, channel, source, name, status) in rows {
                tx.execute(
                    r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId", "status") VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
                    params![id, WS, name, channel, source, status],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "IntegrationThreads" ("id", "auth", "threadsUserId", "username", "name", "workspaceId", "inboxId")
                   VALUES (1, '{}', '1789', 'acme', 'Threads', ?1, 105)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed inboxes");
}

#[tokio::test]
async fn open_chat_links_returns_one_link_per_linkable_channel_of_the_token_workspace() {
    let f = fixture().await;
    seed(&f).await;
    seed_inboxes(&f).await;
    let created = new_reflink(&f, WS, "welcome", "1").await;
    let id = created["id"].as_str().expect("id").to_string();
    let (status, body) = call(&f, "reflinks.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["links"],
        json!([
            { "inboxId": "101", "inboxName": "My Page", "channel": "messenger", "url": "https://m.me/page-1?ref=welcome", "receivesRef": true },
            { "inboxId": "102", "inboxName": "OA", "channel": "zalo", "url": "https://zalo.me/oa-1?ref=welcome", "receivesRef": false },
            { "inboxId": "104", "inboxName": "TT", "channel": "tiktok", "url": "https://www.tiktok.com/@acme.shop", "receivesRef": false },
            { "inboxId": "105", "inboxName": "Threads", "channel": "threads", "url": "https://www.threads.com/@acme", "receivesRef": false },
        ])
    );
}

#[tokio::test]
async fn open_chat_links_list_loads_inboxes_once_for_the_whole_page() {
    let f = fixture().await;
    seed(&f).await;
    seed_inboxes(&f).await;
    new_reflink(&f, WS, "first", "1").await;
    new_reflink(&f, WS, "second", "1").await;
    let (status, body) = call(&f, "reflinks.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    for row in body["data"].as_array().expect("data") {
        let name = row["name"].as_str().expect("name");
        assert_eq!(
            row["links"][0]["url"],
            json!(format!("https://m.me/page-1?ref={name}"))
        );
    }
}
