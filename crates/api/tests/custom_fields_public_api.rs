//! Port of the `/v1/custom-fields` part of
//! `apps/builder/__tests__/tags-custom-fields-public-api.test.ts` (P3.6-D2).
//! Adapted: the captured oRPC handlers become `handlers::dispatch` calls over
//! a real migrated database; "the service was called with" becomes the
//! response and the stored rows.

mod handlers_common;

use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, WS, call, fixture};
use serde_json::{Value, json};

/// Folder 3 (`customField`); fields 11 "city" (folder 3, oldest), 12
/// "home city" (folder 3), 13 "city root" (root), 14 "other" (folder 3).
async fn seed(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Folder" ("id", "workspaceId", "name", "folderType", "paths")
                   VALUES (3, ?1, 'Fields', 'customField', '[]')"#,
                params![WS],
            )?;
            for (id, name, folder, at) in [
                (11_i64, "city", Some(3_i64), 1_000_i64),
                (12, "home city", Some(3), 2_000),
                (13, "city root", None, 3_000),
                (14, "other", Some(3), 4_000),
            ] {
                tx.execute(
                    r#"INSERT INTO "CustomField" ("id", "workspaceId", "name", "type", "folderId", "createdAt", "updatedAt")
                       VALUES (?1, ?2, ?3, 'shortText', ?4, ?5, ?5)"#,
                    params![id, WS, name, folder, at],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
}

fn ids(body: &Value) -> Vec<String> {
    body["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|row| row["id"].as_str().expect("id").to_string())
        .collect()
}

#[tokio::test]
async fn get_v1_custom_fields_forwards_filters_and_defaults_to_newest_first() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "customFields.list",
        WS,
        &[],
        Some("page=1&perPage=50&name=city&folderId=3"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(ids(&body), ["12", "11"]);
    assert_eq!(body["pageCount"], json!(1));
}

#[tokio::test]
async fn post_v1_custom_fields_passes_description_and_folder_id_to_the_service() {
    let f = fixture().await;
    let (status, body) = call(
        &f,
        "customFields.create",
        WS,
        &[],
        None,
        Some(
            json!({ "name": "city", "type": "shortText", "description": "Home", "folderId": "0" }),
        ),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["name"], json!("city"));
    assert_eq!(body["description"], json!("Home"));
    assert_eq!(body["folderId"], Value::Null);
}

#[tokio::test]
async fn get_v1_custom_fields_id_or_name_uses_find_by_key_or_fail_so_an_unknown_field_is_a_404() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "customFields.get",
        WS,
        &[("idOrName", "missing")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["code"], json!("notFound"));
    assert_eq!(body["message"], json!("Custom field not found"));
}

#[tokio::test]
async fn put_v1_custom_fields_id_leaves_the_folder_alone_when_folder_id_is_omitted() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "customFields.update",
        WS,
        &[("id", "11")],
        None,
        Some(json!({ "name": "town" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["name"], json!("town"));
    assert_eq!(body["folderId"], json!("3"));
}

#[tokio::test]
async fn put_v1_custom_fields_id_passes_a_null_folder_id_to_the_service() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "customFields.update",
        WS,
        &[("id", "11")],
        None,
        Some(json!({ "name": "city", "folderId": null })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["folderId"], Value::Null);
}
