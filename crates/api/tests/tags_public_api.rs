//! Ports of the `/v1/tags` part of
//! `apps/builder/__tests__/tags-custom-fields-public-api.test.ts` and the tag
//! part of `contacts-tags-and-fields-public-api.test.ts` (P3.6-D2).
//! Adapted: the captured oRPC handlers become `handlers::dispatch` calls over
//! a real migrated database; "the service was called with" becomes the
//! response and the stored rows.

mod handlers_common;

use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, WS, call, fixture};
use serde_json::{Value, json};

/// Folder 7 (`tag`); tags 1 "vip" (root, oldest), 2 "VIP gold" (folder 7),
/// 3 "basic" (root, newest); contact 10 "Ada" with tag 1.
async fn seed(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Folder" ("id", "workspaceId", "name", "folderType", "paths")
                   VALUES (7, ?1, 'Tags', 'tag', '[]')"#,
                params![WS],
            )?;
            for (id, name, folder, at) in [
                (1_i64, "vip", None, 1_000_i64),
                (2, "VIP gold", Some(7_i64), 2_000),
                (3, "basic", None, 3_000),
            ] {
                tx.execute(
                    r#"INSERT INTO "Tag" ("id", "workspaceId", "name", "folderId", "createdAt", "updatedAt")
                       VALUES (?1, ?2, ?3, ?4, ?5, ?5)"#,
                    params![id, WS, name, folder, at],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (10, 'Ada', ?1)"#,
                params![WS],
            )?;
            tx.execute(r#"INSERT INTO "ContactToTag" ("contactId", "tagId") VALUES (10, 1)"#, [])?;
            Ok(())
        })
        .await
        .expect("seed");
}

fn names(body: &Value) -> Vec<String> {
    body["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|row| row["name"].as_str().expect("name").to_string())
        .collect()
}

async fn contact_tags(f: &Fixture) -> Vec<String> {
    let (status, body) = call(
        f,
        "contacts.listTags",
        WS,
        &[("identifier", "id:10")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    names(&body)
}

// ---- tags-custom-fields-public-api.test.ts ----

#[tokio::test]
async fn get_v1_tags_defaults_to_newest_first() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(&f, "tags.list", WS, &[], Some("page=1&perPage=50"), None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(names(&body), ["basic", "VIP gold", "vip"]);
    assert_eq!(body["data"][2]["contactsCount"], json!(1));
    assert_eq!(body["pageCount"], json!(1));
}

#[tokio::test]
async fn get_v1_tags_forwards_name_folder_id_and_sort() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "tags.list",
        WS,
        &[],
        Some("page=1&perPage=50&name=vip&folderId=0&sort[0][id]=name&sort[0][desc]=false"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(names(&body), ["vip"]);
}

#[tokio::test]
async fn post_v1_tags_passes_folder_id_to_tag_service_create() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "tags.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "VIP", "folderId": "7" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["name"], json!("VIP"));
    assert_eq!(body["folderId"], json!("7"));
    let keys: Vec<&String> = body.as_object().expect("object").keys().collect();
    assert_eq!(keys.len(), 3, "{body}");
}

#[tokio::test]
async fn put_v1_tags_id_renames_and_moves_the_tag() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "tags.update",
        WS,
        &[("id", "2")],
        None,
        Some(json!({ "name": "VIP", "folderId": "0" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["name"], json!("VIP"));
    assert_eq!(body["folderId"], Value::Null);
}

// ---- contacts-tags-and-fields-public-api.test.ts (tags) ----

#[tokio::test]
async fn get_v1_contacts_identifier_tags_resolves_the_contact_id_then_returns_list_contact_tags() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "contacts.listTags",
        WS,
        &[("identifier", "id:10")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "data": [{ "id": "1", "name": "vip", "folderId": null }] })
    );
}

#[tokio::test]
async fn post_v1_contacts_identifier_tags_resolves_the_contact_id_before_attaching_tags() {
    let f = fixture().await;
    seed(&f).await;
    let (status, _) = call(
        &f,
        "contacts.addTags",
        WS,
        &[("identifier", "id:10")],
        None,
        Some(json!({ "tagIds": ["2"] })),
    )
    .await;
    assert_eq!(status, 204);
    assert_eq!(contact_tags(&f).await, ["VIP gold", "vip"]);
}

#[tokio::test]
async fn delete_v1_contacts_identifier_tags_resolves_the_contact_id_before_detaching_tags() {
    let f = fixture().await;
    seed(&f).await;
    let (status, _) = call(
        &f,
        "contacts.removeTags",
        WS,
        &[("identifier", "id:10")],
        None,
        Some(json!({ "tagIds": ["1"] })),
    )
    .await;
    assert_eq!(status, 204);
    assert!(contact_tags(&f).await.is_empty());
}

#[tokio::test]
async fn post_v1_contacts_identifier_tags_by_name_delegates_to_add_contact_tags_with_the_resolved_contact_id()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, _) = call(
        &f,
        "contacts.addTagsByName",
        WS,
        &[("identifier", "id:10")],
        None,
        Some(json!({ "tags": ["basic", "new"] })),
    )
    .await;
    assert_eq!(status, 204);
    assert_eq!(contact_tags(&f).await, ["basic", "new", "vip"]);
}

#[tokio::test]
async fn put_v1_contacts_identifier_tags_replaces_all_tags_on_the_contact_by_name_via_update_contact_tags()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, _) = call(
        &f,
        "contacts.setTags",
        WS,
        &[("identifier", "id:10")],
        None,
        Some(json!({ "tags": ["basic"] })),
    )
    .await;
    assert_eq!(status, 204);
    assert_eq!(contact_tags(&f).await, ["basic"]);
}
