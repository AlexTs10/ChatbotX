//! Port of `apps/builder/__tests__/folders-public-api.test.ts` (P3.6-D2
//! slice folders). Adapted: the mocked `folderService` becomes a migrated
//! database; the asserted service calls become the stored rows and the
//! responses; the scope registration becomes the route table; the
//! `contactsFolderTypes` schema becomes the 422 of the routes.

mod handlers_common;

use chatbotx_api::routes::scopes_of_module;
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, WS, call, fixture, scalar};
use serde_json::{Value, json};

/// Folders 11 (tag), 12 (tag, child of 11), 13 (customField), 14 (flow),
/// 15 (webhook), all in `WS`.
async fn seeded() -> Fixture {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            for (id, kind, parent, paths) in [
                (11, "tag", None, "[]"),
                (12, "tag", Some(11), "[\"11\"]"),
                (13, "customField", None, "[]"),
                (14, "flow", None, "[]"),
                (15, "webhook", None, "[]"),
            ] {
                tx.execute(
                    r#"INSERT INTO "Folder" ("id", "workspaceId", "name", "folderType", "parentId", "paths") VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
                    params![id, WS, format!("Folder {id}"), kind, parent, paths],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
    f
}

async fn post(f: &Fixture, body: Value) -> (u16, Value) {
    call(f, "folders.create", WS, &[], None, Some(body)).await
}

async fn folder_exists(f: &Fixture, id: i64) -> bool {
    scalar(f, r#"SELECT "id" FROM "Folder" WHERE "id" = ?1"#, id)
        .await
        .is_some()
}

fn ids(body: &Value) -> Vec<String> {
    body["data"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|row| row["id"].as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn registers_the_folders_public_router_under_the_contacts_scope() {
    assert_eq!(
        scopes_of_module("features/folders/api/public"),
        vec!["contacts"]
    );
}

#[tokio::test]
async fn contacts_folder_types_schema_accepts_tag_and_custom_field() {
    let f = seeded().await;
    for kind in ["tag", "customField"] {
        let (status, body) = post(&f, json!({"name": "New", "folderType": kind})).await;
        assert_eq!(status, 200, "{kind}: {body}");
        assert_eq!(body["folderType"], json!(kind));
    }
}

#[tokio::test]
async fn contacts_folder_types_schema_rejects_folder_type_flow_a_public_post_v1_folders_with_it_fails_validation()
 {
    let f = seeded().await;
    let (status, body) = post(&f, json!({"name": "New", "folderType": "flow"})).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["code"], json!("invalidRequestData"));
}

#[tokio::test]
async fn get_v1_folders_lists_folders_for_the_given_type_at_the_root_when_parent_id_is_omitted() {
    let f = seeded().await;
    let (status, body) = call(&f, "folders.list", WS, &[], Some("folderType=tag"), None).await;
    assert_eq!(status, 200, "{body}");
    // `parentId: null` reaches the service, which then does not narrow by
    // parent: the sub-folder 12 is listed too.
    assert_eq!(ids(&body), vec!["11".to_string(), "12".to_string()]);
    assert_eq!(body["data"][0]["name"], json!("Folder 11"));
}

#[tokio::test]
async fn post_v1_folders_creates_a_top_level_folder_when_parent_id_is_the_root_sentinel() {
    let f = seeded().await;
    let (status, body) = post(
        &f,
        json!({"name": "New", "folderType": "tag", "parentId": "0"}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["parentId"], Value::Null);
    assert_eq!(body["paths"], json!([]));
    assert_eq!(body["name"], json!("New"));
}

#[tokio::test]
async fn post_v1_folders_creates_a_nested_folder_when_a_real_parent_id_is_given() {
    let f = seeded().await;
    let (status, body) = post(
        &f,
        json!({"name": "Child", "folderType": "tag", "parentId": "11"}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["parentId"], json!("11"));
    assert_eq!(body["paths"], json!(["11"]));
}

async fn put(f: &Fixture, id: &str, body: Value) -> (u16, Value) {
    call(f, "folders.update", WS, &[("id", id)], None, Some(body)).await
}

#[tokio::test]
async fn put_v1_folders_id_renames_the_folder_when_it_is_a_contacts_owned_folder_type() {
    let f = seeded().await;
    let (status, body) = put(&f, "11", json!({"name": "Renamed"})).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("11"));
    assert_eq!(body["name"], json!("Renamed"));
}

#[tokio::test]
async fn put_v1_folders_id_404s_does_not_call_update_when_the_folder_s_type_is_outside_contacts_scope()
 {
    let f = seeded().await;
    let (status, body) = put(&f, "14", json!({"name": "Renamed"})).await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Folder not found"));
    let name = scalar(&f, r#"SELECT "name" FROM "Folder" WHERE "id" = ?1"#, 14).await;
    assert_eq!(name, Some(json!("Folder 14")));
}

#[tokio::test]
async fn put_v1_folders_id_404s_when_the_folder_does_not_exist() {
    let f = seeded().await;
    let (status, body) = put(&f, "999", json!({"name": "Renamed"})).await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Folder not found"));
}

async fn delete(f: &Fixture, id: &str) -> (u16, Value) {
    call(f, "folders.delete", WS, &[("id", id)], None, None).await
}

#[tokio::test]
async fn delete_v1_folders_id_deletes_the_folder_when_it_is_a_contacts_owned_folder_type() {
    let f = seeded().await;
    let (status, body) = delete(&f, "13").await;
    assert_eq!(status, 204, "{body}");
    assert!(!folder_exists(&f, 13).await);
    // A parent goes with its sub-folders.
    let (status, _) = delete(&f, "11").await;
    assert_eq!(status, 204);
    assert!(!folder_exists(&f, 11).await);
    assert!(!folder_exists(&f, 12).await);
}

#[tokio::test]
async fn delete_v1_folders_id_404s_does_not_call_bulk_delete_for_a_nonexistent_id() {
    let f = seeded().await;
    let (status, body) = delete(&f, "999").await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Folder not found"));
}

#[tokio::test]
async fn delete_v1_folders_id_404s_does_not_call_bulk_delete_when_the_folder_s_type_is_outside_contacts_scope()
 {
    let f = seeded().await;
    let (status, body) = delete(&f, "15").await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Folder not found"));
    assert!(folder_exists(&f, 15).await);
}
