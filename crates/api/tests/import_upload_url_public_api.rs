//! Port of the contact parts of
//! `apps/builder/__tests__/import-upload-url-public-api.test.ts` (P3.6-D2
//! slice contact imports, part one). Adapted: the mocked
//! `createImportUpload` / `peekImportHeaders` / `importService.list` become
//! the real services over an in-memory store with the local URL signer and
//! seeded rows. Moved to the product import slice (P3.6-D1): the products
//! upload, products headers, products template and `POST /v1/products/imports`
//! cases.

mod handlers_common;

use std::sync::Arc;

use chatbotx_api::routes::match_route;
use chatbotx_db::rusqlite::params;
use chatbotx_files::{LocalUrlSigner, PutOptions, Uploader};
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::json;

/// The fixture with a file store.
async fn with_files() -> (Fixture, Arc<Uploader>) {
    let mut f = fixture().await;
    let files = Arc::new(
        Uploader::in_memory()
            .with_url_signer(LocalUrlSigner::new("secret", "http://app/files").expect("signer")),
    );
    f.state = f.state.clone().with_files(Arc::clone(&files));
    (f, files)
}

async fn exec(f: &Fixture, sql: &'static str, values: Vec<i64>) {
    f.state
        .db
        .write(move |tx| {
            tx.execute(sql, chatbotx_db::rusqlite::params_from_iter(values.iter()))?;
            Ok(())
        })
        .await
        .expect("exec");
}

#[tokio::test]
async fn post_v1_contacts_imports_upload_url_mints_an_upload_for_the_token_s_workspace_as_a_contacts_import()
 {
    let (f, _) = with_files().await;
    let (status, body) = call(
        &f,
        "contacts.createImportUpload",
        WS,
        &[],
        None,
        Some(json!({ "fileName": "people.csv", "mimeType": "text/csv", "fileSize": 10 })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let path = body["path"].as_str().expect("path");
    assert!(path.starts_with(&format!("workspaces/{WS}/imports/contacts/import_")));
    assert_eq!(
        body["publicUrl"],
        json!(format!("http://app/storage/{path}"))
    );
    let keys: Vec<&str> = body
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        vec!["fileId", "presignedPostUrl", "publicUrl", "path"]
    );
    let stored = f
        .state
        .db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "workspaceId", "userId", "subType" FROM "File""#,
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )?)
        })
        .await
        .expect("file");
    assert_eq!(stored, (WS, None, "contacts".to_string()));
    assert_eq!(body["fileId"], json!(stored_id(&f).await.to_string()));
}

async fn stored_id(f: &Fixture) -> i64 {
    f.state
        .db
        .read(|conn| Ok(conn.query_row(r#"SELECT "id" FROM "File""#, [], |r| r.get(0))?))
        .await
        .expect("id")
}

#[tokio::test]
async fn post_v1_contacts_imports_upload_url_is_gated_by_the_contacts_scope_and_returns_201() {
    let matched = match_route("POST", "/v1/contacts/imports/upload-url").expect("route");
    assert_eq!(matched.route.id, "contacts.createImportUpload");
    assert_eq!(
        format!("{:?}", matched.route.auth),
        format!(
            "{:?}",
            chatbotx_api::routes::RouteAuth::WorkspaceToken(&["contacts"])
        )
    );
}

#[tokio::test]
async fn get_v1_contacts_imports_files_file_id_headers_reads_the_headers_of_a_file_in_the_token_s_workspace_and_suggests_a_column_map()
 {
    let (f, files) = with_files().await;
    exec(
        &f,
        r#"INSERT INTO "File" ("id", "workspaceId", "contextType", "subType", "path", "fileName", "mimeType")
           VALUES (9, ?1, 'import', 'contacts', 'workspaces/2001/imports/contacts/a.csv', 'a.csv', 'text/csv')"#,
        vec![WS],
    )
    .await;
    files
        .put_object(
            "workspaces/2001/imports/contacts/a.csv",
            b"Phone,Name\n1,2\n".to_vec(),
            PutOptions::default(),
            None,
        )
        .await
        .expect("put");
    let (status, body) = call(
        &f,
        "contacts.peekImportHeaders",
        WS,
        &[("fileId", "9")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "headers": ["Phone", "Name"], "suggestedColumnMap": { "phoneNumber": "Phone" } })
    );
    // Another workspace's file is unreadable.
    let (status, body) = call(
        &f,
        "contacts.peekImportHeaders",
        OTHER_WS,
        &[("fileId", "9")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["code"], json!("importUnableToReadHeaders"));
}

#[tokio::test]
async fn get_v1_contacts_import_template_returns_the_csv_in_the_workspace_language_by_default() {
    let (f, _) = with_files().await;
    exec(
        &f,
        r#"UPDATE "Workspace" SET "language" = 'vi' WHERE "id" = ?1"#,
        vec![WS],
    )
    .await;
    let (status, body) = call(&f, "contacts.getImportTemplate", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["fileName"], json!("contacts-import-template.csv"));
    assert_eq!(body["mimeType"], json!("text/csv"));
    assert!(
        body["content"]
            .as_str()
            .expect("content")
            .contains("Số điện thoại")
    );
}

#[tokio::test]
async fn get_v1_contacts_import_template_an_explicit_language_wins_over_the_workspace_language() {
    let (f, _) = with_files().await;
    exec(
        &f,
        r#"UPDATE "Workspace" SET "language" = 'vi' WHERE "id" = ?1"#,
        vec![WS],
    )
    .await;
    let (status, body) = call(
        &f,
        "contacts.getImportTemplate",
        WS,
        &[],
        Some("language=en"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body["content"]
            .as_str()
            .expect("content")
            .contains("Phone number")
    );
}

#[tokio::test]
async fn get_v1_contacts_imports_forwards_sort_to_the_service_and_scopes_to_contact_imports() {
    let (f, _) = with_files().await;
    f.state
        .db
        .write(|tx| {
            for (file, name) in [(1_i64, "b.csv"), (2, "a.csv"), (3, "p.csv")] {
                tx.execute(
                    r#"INSERT INTO "File" ("id", "workspaceId", "contextType", "subType", "path", "fileName", "mimeType")
                       VALUES (?1, ?2, 'import', 'contacts', ?3, ?3, 'text/csv')"#,
                    params![file, WS, name],
                )?;
            }
            for (id, file, kind, status) in [
                (11_i64, 1_i64, "contacts", "pending"),
                (12, 2, "contacts", "completed"),
                (13, 3, "products", "completed"),
            ] {
                tx.execute(
                    r#"INSERT INTO "Import" ("id", "workspaceId", "fileId", "type", "format", "status", "meta")
                       VALUES (?1, ?2, ?3, ?4, 'csv', ?5, '{}')"#,
                    params![id, WS, file, kind, status],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
    let (status, body) = call(
        &f,
        "contacts.listImports",
        WS,
        &[],
        Some("page=1&perPage=10&sort[0][id]=status&sort[0][desc]=false"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let ids: Vec<&str> = body["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|row| row["id"].as_str().expect("id"))
        .collect();
    // `completed` < `pending`; the products import is not listed.
    assert_eq!(ids, vec!["12", "11"]);
    assert_eq!(body["pageCount"], json!(1));
    assert_eq!(body["data"][0]["type"], json!("contacts"));
    assert_eq!(body["data"][0]["fileName"], json!("a.csv"));
}
