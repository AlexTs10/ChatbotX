//! Port of `apps/builder/__tests__/media-library-public-api.test.ts`.
//! Adapted: the captured oRPC procedures and mocked services become
//! `handlers::dispatch` calls over a real database and an in-memory store;
//! "service called with" becomes the stored rows / the answer.

mod handlers_common;

use std::sync::Arc;

use chatbotx_api::routes::{routes_of_module, scopes_of_module};
use chatbotx_files::{LocalUrlSigner, Uploader};
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

const MODULE: &str = "features/media-library/api/public";

async fn with_files() -> Fixture {
    let mut f = fixture().await;
    let signer = LocalUrlSigner::new("secret", "http://app/files").expect("signer");
    f.state = f
        .state
        .clone()
        .with_files(Arc::new(Uploader::in_memory().with_url_signer(signer)));
    f
}

async fn folder(f: &Fixture, name: &str) -> String {
    let (status, body) = call(
        f,
        "mediaLibrary.createFolder",
        WS,
        &[],
        None,
        Some(json!({ "name": name })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    body["id"].as_str().expect("id").to_string()
}

async fn file(f: &Fixture, name: &str, folder_id: Option<&str>) -> Value {
    let (status, body) = call(
        f,
        "mediaLibrary.createFile",
        WS,
        &[],
        None,
        Some(json!({
            "folderId": folder_id,
            "name": name,
            "path": format!("public/space/{WS}/media-library/{name}"),
            "mimeType": "image/png",
            "size": 100,
        })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    body
}

#[test]
fn registers_the_media_library_public_router_under_the_media_scope() {
    assert_eq!(scopes_of_module(MODULE), vec!["media"]);
}

#[test]
fn registers_each_media_library_route() {
    let routes: Vec<(&str, &str)> = routes_of_module(MODULE)
        .iter()
        .map(|r| (r.method, r.path))
        .collect();
    let mut expected = vec![
        ("GET", "/v1/media-library/folders"),
        ("POST", "/v1/media-library/folders"),
        ("PUT", "/v1/media-library/folders/{folderId}"),
        ("DELETE", "/v1/media-library/folders/{folderId}"),
        ("POST", "/v1/media-library/files/upload-url"),
        ("GET", "/v1/media-library/files"),
        ("GET", "/v1/media-library/files/{fileId}"),
        ("POST", "/v1/media-library/files"),
        ("DELETE", "/v1/media-library/files/{fileId}"),
        ("PUT", "/v1/media-library/files/{fileId}/favourite"),
        ("POST", "/v1/media-library/files/{fileId}/access"),
        ("PATCH", "/v1/media-library/files/move"),
    ];
    let mut got = routes;
    // Order-free: the Rust route table sorts by module, not by declaration.
    got.sort_unstable();
    expected.sort_unstable();
    assert_eq!(got, expected);
}

// ---- folder routes ---------------------------------------------------------

#[tokio::test]
async fn folder_routes_lists_workspace_folders_with_in_memory_pagination() {
    let f = fixture().await;
    let id = folder(&f, "Images").await;
    file(&f, "a.png", Some(&id)).await;
    file(&f, "b.png", Some(&id)).await;
    let (status, body) = call(
        &f,
        "mediaLibrary.listFolders",
        WS,
        &[],
        Some("page=1&perPage=50"),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["pageCount"], 1);
    assert_eq!(body["data"][0]["id"], id.as_str());
    assert_eq!(body["data"][0]["name"], "Images");
    assert_eq!(body["data"][0]["fileCount"], 2);
    let (_, other) = call(&f, "mediaLibrary.listFolders", OTHER_WS, &[], None, None).await;
    assert_eq!(other["data"], json!([]));
}

#[tokio::test]
async fn folder_routes_creates_a_workspace_folder() {
    let f = fixture().await;
    let (status, body) = call(
        &f,
        "mediaLibrary.createFolder",
        WS,
        &[],
        None,
        Some(json!({ "name": "Images" })),
    )
    .await;
    assert_eq!(status, 201);
    assert_eq!(body["name"], "Images");
    let id: i64 = body["id"].as_str().expect("id").parse().expect("digits");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "workspaceId" FROM "MediaLibraryFolder" WHERE "id" = ?1"#,
            id
        )
        .await,
        Some(json!(WS))
    );
}

#[tokio::test]
async fn folder_routes_renames_a_workspace_folder() {
    let f = fixture().await;
    let id = folder(&f, "Images").await;
    let (status, body) = call(
        &f,
        "mediaLibrary.renameFolder",
        WS,
        &[("folderId", &id)],
        None,
        Some(json!({ "name": "Renamed" })),
    )
    .await;
    assert_eq!((status, body), (204, Value::Null));
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "name" FROM "MediaLibraryFolder" WHERE "id" = ?1"#,
            id.parse().expect("id")
        )
        .await,
        Some(json!("Renamed"))
    );
}

#[tokio::test]
async fn folder_routes_deletes_a_workspace_folder() {
    let f = with_files().await;
    let id = folder(&f, "Images").await;
    let (status, _) = call(
        &f,
        "mediaLibrary.deleteFolder",
        WS,
        &[("folderId", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    assert_eq!(
        scalar(
            &f,
            r#"SELECT count(*) FROM "MediaLibraryFolder" WHERE "id" = ?1"#,
            id.parse().expect("id")
        )
        .await,
        Some(json!(0))
    );
}

// ---- file routes -----------------------------------------------------------

#[tokio::test]
async fn file_routes_creates_a_presigned_upload_url() {
    let f = with_files().await;
    let (status, body) = call(
        &f,
        "mediaLibrary.createUploadUrl",
        WS,
        &[],
        None,
        Some(json!({ "fileName": "cat.png", "mimeType": "image/png" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let path = body["path"].as_str().expect("path");
    assert!(path.starts_with(&format!("public/space/{WS}/media-library/")));
    assert_eq!(body["publicUrl"], format!("http://app/storage/{path}"));
    assert!(
        body["uploadUrl"]
            .as_str()
            .expect("url")
            .contains("X-Amz-Expires=300")
    );
}

#[tokio::test]
async fn file_routes_lists_workspace_files_with_page_count() {
    let f = fixture().await;
    for name in ["a.png", "b.png", "c.png"] {
        let created = file(&f, name, None).await;
        if name != "b.png" {
            call(
                &f,
                "mediaLibrary.setFavourite",
                WS,
                &[("fileId", created["id"].as_str().expect("id"))],
                None,
                Some(json!({ "isFavourite": true })),
            )
            .await;
        }
    }
    let (status, body) = call(
        &f,
        "mediaLibrary.listFiles",
        WS,
        &[],
        Some("filter=favourite&page=2&perPage=1"),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["pageCount"], 2);
    assert_eq!(body["data"][0]["name"], "a.png");
    assert!(
        body["data"][0]["url"]
            .as_str()
            .expect("url")
            .starts_with("http://app/storage/")
    );
}

#[tokio::test]
async fn file_routes_gets_a_single_workspace_file() {
    let f = fixture().await;
    let created = file(&f, "a.png", None).await;
    let id = created["id"].as_str().expect("id");
    let (status, body) = call(
        &f,
        "mediaLibrary.getFile",
        WS,
        &[("fileId", id)],
        None,
        None,
    )
    .await;
    assert_eq!((status, &body), (200, &created));
    let (status, _) = call(
        &f,
        "mediaLibrary.getFile",
        OTHER_WS,
        &[("fileId", id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn file_routes_creates_a_workspace_file() {
    let f = fixture().await;
    let id = folder(&f, "F").await;
    let body = file(&f, "image.png", Some(&id)).await;
    assert_eq!(body["name"], "image.png");
    assert_eq!(body["folderId"], id.as_str());
    assert_eq!(body["size"], 100);
    assert_eq!(
        body["url"],
        format!("http://app/storage/public/space/{WS}/media-library/image.png")
    );
}

#[tokio::test]
async fn file_routes_deletes_a_workspace_file() {
    let f = with_files().await;
    let created = file(&f, "a.png", None).await;
    let id = created["id"].as_str().expect("id");
    let (status, _) = call(
        &f,
        "mediaLibrary.deleteFile",
        WS,
        &[("fileId", id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    assert_eq!(
        scalar(
            &f,
            r#"SELECT count(*) FROM "MediaLibraryFile" WHERE "id" = ?1"#,
            id.parse().expect("id")
        )
        .await,
        Some(json!(0))
    );
}

#[tokio::test]
async fn file_routes_sets_a_workspace_file_s_favourite_status_to_the_explicit_value() {
    let f = fixture().await;
    let created = file(&f, "a.png", None).await;
    let id = created["id"].as_str().expect("id");
    let (status, body) = call(
        &f,
        "mediaLibrary.setFavourite",
        WS,
        &[("fileId", id)],
        None,
        Some(json!({ "isFavourite": true })),
    )
    .await;
    assert_eq!(status, 200);
    let mut expected = created.clone();
    expected["isFavourite"] = json!(true);
    assert_eq!(body, expected);
}

#[tokio::test]
async fn file_routes_records_a_workspace_file_s_access() {
    let f = fixture().await;
    let created = file(&f, "a.png", None).await;
    let id = created["id"].as_str().expect("id");
    let (status, _) = call(
        &f,
        "mediaLibrary.recordAccess",
        WS,
        &[("fileId", id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    let (_, body) = call(
        &f,
        "mediaLibrary.getFile",
        WS,
        &[("fileId", id)],
        None,
        None,
    )
    .await;
    assert!(body["lastAccessedAt"].is_string());
}

#[tokio::test]
async fn file_routes_moves_workspace_files_into_another_folder() {
    let f = fixture().await;
    let target = folder(&f, "Target").await;
    let a = file(&f, "a.png", None).await;
    let b = file(&f, "b.png", None).await;
    let (status, _) = call(
        &f,
        "mediaLibrary.moveFiles",
        WS,
        &[],
        None,
        Some(json!({ "fileIds": [a["id"], b["id"]], "folderId": target })),
    )
    .await;
    assert_eq!(status, 204);
    let (_, listed) = call(
        &f,
        "mediaLibrary.listFiles",
        WS,
        &[],
        Some(&format!("folderId={target}")),
        None,
    )
    .await;
    assert_eq!(listed["data"].as_array().map(Vec::len), Some(2));
}

#[tokio::test]
async fn file_routes_surfaces_a_declared_delete_error() {
    let f = with_files().await;
    let (status, body) = call(
        &f,
        "mediaLibrary.deleteFile",
        WS,
        &[("fileId", "999")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(body["code"], "notFound");
    assert_eq!(body["message"], "MediaLibraryFile 999 not found");
    assert_eq!(body["defined"], true);
}
