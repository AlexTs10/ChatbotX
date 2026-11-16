//! Port of `packages/business/__tests__/import-create-upload.test.ts`
//! (P3.6-D2 slice contact imports, part one). Adapted: the mocked
//! `uploader.getPresignedUpload`, `fileService.createPending` and
//! `resolveTenantSettings` become an in-memory store with the local URL
//! signer, the stored `File` row and a given storage URL; "without signing
//! anything" becomes "no `File` row".

mod common;

use chatbotx_business::import::{CreateImportUpload, ImportType, create_import_upload};
use chatbotx_files::{LocalUrlSigner, Uploader};
use common::{TestDb, WS, two_workspaces};

const STORAGE_URL: &str = "https://cdn.example.com/";

fn uploader() -> Uploader {
    Uploader::in_memory()
        .with_url_signer(LocalUrlSigner::new("secret", "http://app/files").expect("signer"))
}

fn base() -> CreateImportUpload {
    CreateImportUpload {
        workspace_id: WS,
        user_id: None,
        import_type: ImportType::Contacts,
        file_name: "people.csv".into(),
        mime_type: "text/csv".into(),
        file_size: 1024,
    }
}

/// `(workspaceId, userId, contextType, subType, path, fileName, mimeType,
/// status)` of every `File` row.
async fn files(
    db: &TestDb,
) -> Vec<(
    i64,
    Option<i64>,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
)> {
    db.read(|conn| {
        let mut stmt = conn.prepare(
            r#"SELECT "workspaceId", "userId", "contextType", "subType", "path", "fileName", "mimeType", "status" FROM "File" ORDER BY "id""#,
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    })
    .await
    .expect("files")
}

#[tokio::test]
async fn create_import_upload_records_a_pending_import_file_for_the_workspace_and_returns_the_presigned_url()
 {
    let db = two_workspaces().await;
    let result = create_import_upload(&db, &uploader(), STORAGE_URL, base())
        .await
        .expect("upload");
    let prefix = format!("workspaces/{WS}/imports/contacts/import_");
    assert!(result.path.starts_with(&prefix), "{}", result.path);
    assert!(result.path.ends_with(".csv"), "{}", result.path);
    assert!(
        result
            .presigned_post_url
            .starts_with(&format!("http://app/files/{}?", result.path)),
        "{}",
        result.presigned_post_url
    );
    assert!(result.presigned_post_url.contains("X-Amz-Expires=300"));
    assert_eq!(result.public_url, format!("{STORAGE_URL}{}", result.path));
    let rows = files(&db).await;
    assert_eq!(
        rows,
        vec![(
            WS,
            None,
            "import".to_string(),
            Some("contacts".to_string()),
            result.path.clone(),
            "people.csv".to_string(),
            "text/csv".to_string(),
            "pending".to_string(),
        )]
    );
    let file_id: i64 = db
        .read(|conn| Ok(conn.query_row(r#"SELECT "id" FROM "File""#, [], |r| r.get(0))?))
        .await
        .expect("id");
    assert_eq!(result.file_id, file_id);
}

#[tokio::test]
async fn create_import_upload_accepts_xlsx_for_products() {
    let db = two_workspaces().await;
    let input = CreateImportUpload {
        import_type: ImportType::Products,
        file_name: "items.xlsx".into(),
        mime_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into(),
        ..base()
    };
    let result = create_import_upload(&db, &uploader(), STORAGE_URL, input)
        .await
        .expect("upload");
    assert!(result.path.ends_with(".xlsx"));
}

async fn rejected(input: CreateImportUpload, code: &str) {
    let db = two_workspaces().await;
    let error = create_import_upload(&db, &uploader(), STORAGE_URL, input)
        .await
        .expect_err("rejected");
    assert_eq!(error.code(), Some(code));
    assert!(files(&db).await.is_empty());
}

#[tokio::test]
async fn create_import_upload_rejects_a_mime_type_the_import_does_not_accept_without_signing_anything()
 {
    rejected(
        CreateImportUpload {
            mime_type: "image/png".into(),
            ..base()
        },
        "importUnsupportedFileType",
    )
    .await;
}

#[tokio::test]
async fn create_import_upload_rejects_a_file_name_whose_extension_does_not_match_the_mime_type() {
    rejected(
        CreateImportUpload {
            file_name: "people.xlsx".into(),
            ..base()
        },
        "importUnsupportedFileType",
    )
    .await;
}

#[tokio::test]
async fn create_import_upload_rejects_xlsx_for_contacts_csv_only() {
    rejected(
        CreateImportUpload {
            file_name: "people.xlsx".into(),
            mime_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into(),
            ..base()
        },
        "importUnsupportedFileType",
    )
    .await;
}

#[tokio::test]
async fn create_import_upload_rejects_a_declared_size_over_the_import_type_s_limit() {
    rejected(
        CreateImportUpload {
            file_size: 21 * 1024 * 1024,
            ..base()
        },
        "importFileTooLarge",
    )
    .await;
}
