//! Port of `packages/business/__tests__/import-peek-headers.test.ts`
//! (P3.6-D2 slice contact imports, part one). Adapted: the mocked
//! `importService.findFile` and `uploader.getObjectStream` become a stored
//! `File` row and an object in an in-memory store; "getObjectStream not
//! called" becomes "the stored object is not needed" (no object stored).
//! Not applicable: "other storage failures still surface" (the in-memory
//! store cannot fail except for a missing key), "an oversized stream is
//! fileTooLarge when no content length is reported" and "still releases its
//! stream" (the store always reports the length; the size check is the
//! "real content length" case), "a connection error on the storage stream"
//! (no stream). XLSX: "bytes that are not a valid XLSX workbook are
//! unreadable" holds (XLSX header reading is in the product import slice).

mod common;

use chatbotx_business::import::{ImportType, peek_import_headers};
use chatbotx_db::rusqlite::params;
use chatbotx_files::{PutOptions, Uploader};
use common::{TestDb, WS, setup, two_workspaces};
use serde_json::json;

const PATH: &str = "workspaces/2001/imports/contacts/a.csv";
const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

/// File 7 of `WS` (`subType`, name, MIME, size), at [`PATH`].
async fn file(db: &TestDb, sub_type: &str, file_name: &str, mime: &str, size: Option<i64>) {
    let (sub_type, file_name, mime) = (
        sub_type.to_string(),
        file_name.to_string(),
        mime.to_string(),
    );
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "File" ("id", "workspaceId", "contextType", "subType", "path", "fileName", "mimeType", "fileSize")
               VALUES (7, ?1, 'import', ?2, ?3, ?4, ?5, ?6)"#,
            params![WS, sub_type, PATH, file_name, mime, size],
        )?;
        Ok(())
    })
    .await;
}

async fn csv_file(db: &TestDb) {
    file(db, "contacts", "people.csv", "text/csv", Some(100)).await;
}

async fn stored(body: &[u8]) -> Uploader {
    let uploader = Uploader::in_memory();
    uploader
        .put_object(PATH, body.to_vec(), PutOptions::default(), None)
        .await
        .expect("put");
    uploader
}

async fn code(db: &TestDb, uploader: &Uploader, import_type: Option<ImportType>) -> String {
    peek_import_headers(db, uploader, WS, 7, import_type)
        .await
        .expect_err("peek fails")
        .code()
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn peek_import_headers_returns_the_first_row_s_column_names_for_a_csv_file() {
    let db = two_workspaces().await;
    csv_file(&db).await;
    let uploader = stored(b"Phone,First name\n+8490,Ann\n").await;
    let headers = peek_import_headers(&db, &uploader, WS, 7, None)
        .await
        .expect("headers");
    assert_eq!(headers, vec!["Phone".to_string(), "First name".to_string()]);
}

#[tokio::test]
async fn peek_import_headers_reports_an_unknown_file_as_unable_to_read() {
    let db = two_workspaces().await;
    let uploader = Uploader::in_memory();
    assert_eq!(
        code(&db, &uploader, None).await,
        "importUnableToReadHeaders"
    );
}

#[tokio::test]
async fn peek_import_headers_rejects_a_file_whose_sub_type_is_not_an_import_type() {
    let db = two_workspaces().await;
    file(&db, "generic", "people.csv", "text/csv", Some(100)).await;
    let uploader = stored(b"a\n1\n").await;
    assert_eq!(
        code(&db, &uploader, None).await,
        "importUnsupportedFileType"
    );
}

#[tokio::test]
async fn peek_import_headers_rejects_a_mime_extension_the_import_type_does_not_accept() {
    let db = two_workspaces().await;
    file(&db, "contacts", "people.csv", "image/png", Some(100)).await;
    assert_eq!(
        code(&db, &Uploader::in_memory(), None).await,
        "importUnsupportedFileType"
    );
}

#[tokio::test]
async fn peek_import_headers_rejects_a_recorded_size_over_the_limit_before_reading_storage() {
    let db = two_workspaces().await;
    file(
        &db,
        "contacts",
        "people.csv",
        "text/csv",
        Some(21 * 1024 * 1024),
    )
    .await;
    let error = peek_import_headers(&db, &Uploader::in_memory(), WS, 7, None)
        .await
        .expect_err("too large");
    assert_eq!(error.code(), Some("importFileTooLarge"));
    let chatbotx_business::BusinessError::Exception { data, .. } = error else {
        panic!("an exception");
    };
    assert_eq!(data, Some(json!({ "size": 20 })));
}

#[tokio::test]
async fn peek_import_headers_rejects_an_object_whose_real_content_length_is_over_the_limit() {
    let db = two_workspaces().await;
    file(&db, "contacts", "people.csv", "text/csv", None).await;
    let mut body = b"a,b\n1,2\n".to_vec();
    body.resize(21 * 1024 * 1024, b' ');
    let uploader = stored(&body).await;
    assert_eq!(code(&db, &uploader, None).await, "importFileTooLarge");
}

#[tokio::test]
async fn peek_import_headers_an_empty_file_has_no_headers_to_read() {
    let db = two_workspaces().await;
    csv_file(&db).await;
    let uploader = stored(b"").await;
    assert_eq!(
        code(&db, &uploader, None).await,
        "importUnableToReadHeaders"
    );
}

#[tokio::test]
async fn peek_import_headers_a_file_of_another_import_type_is_reported_as_unreadable_when_a_type_is_required()
 {
    let db = two_workspaces().await;
    file(&db, "products", "people.csv", "text/csv", Some(100)).await;
    assert_eq!(
        code(&db, &Uploader::in_memory(), Some(ImportType::Contacts)).await,
        "importUnableToReadHeaders"
    );
}

#[tokio::test]
async fn peek_import_headers_a_file_of_the_required_type_is_read() {
    let db = two_workspaces().await;
    csv_file(&db).await;
    let uploader = stored(b"Phone\n+8490\n").await;
    let headers = peek_import_headers(&db, &uploader, WS, 7, Some(ImportType::Contacts))
        .await
        .expect("headers");
    assert_eq!(headers, vec!["Phone".to_string()]);
}

#[tokio::test]
async fn peek_import_headers_an_object_that_was_never_uploaded_is_unreadable_not_a_server_error() {
    let db = two_workspaces().await;
    csv_file(&db).await;
    assert_eq!(
        code(&db, &Uploader::in_memory(), None).await,
        "importUnableToReadHeaders"
    );
}

#[tokio::test]
async fn peek_import_headers_bytes_that_are_not_a_valid_xlsx_workbook_are_unreadable() {
    let db = two_workspaces().await;
    file(&db, "products", "items.xlsx", XLSX, Some(100)).await;
    let uploader = stored(b"not a zip").await;
    assert_eq!(
        code(&db, &uploader, None).await,
        "importUnableToReadHeaders"
    );
}

#[tokio::test]
async fn peek_import_headers_strips_the_bom_trims_and_reads_only_the_header_row() {
    // Parity `10-import` "reads the header row (BOM stripped, trimmed)".
    let db = two_workspaces().await;
    csv_file(&db).await;
    let uploader = stored("\u{feff} Phone ,\"First, name\"\n1,2\n".as_bytes()).await;
    let headers = peek_import_headers(&db, &uploader, WS, 7, None)
        .await
        .expect("headers");
    assert_eq!(
        headers,
        vec!["Phone".to_string(), "First, name".to_string()]
    );
    // A header row without a data row has no headers (`columns: true`).
    let db = two_workspaces().await;
    csv_file(&db).await;
    let uploader = stored(b"Phone,Name\n").await;
    assert_eq!(
        code(&db, &uploader, None).await,
        "importUnableToReadHeaders"
    );
}
