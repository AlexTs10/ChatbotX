//! Port of `packages/business/__tests__/mini-app-upload-service.test.ts`.
//! Adapted: the mocked `putObject` and repositories become an in-memory
//! uploader and real rows; `claimForSubmission` is reached through
//! `create_submission` (it runs inside the submission transaction).

mod common;

use chatbotx_business::BusinessError;
use chatbotx_business::mini_app::{self as service, DefinitionInput, UploadedFile};
use chatbotx_db::repo::models::MiniApp;
use chatbotx_files::Uploader;
use common::{WS, contact, rows, setup, two_workspaces};
use serde_json::json;

const CONTACT: i64 = 7;
const STORAGE: &str = "https://cdn.example.com/";
const JPEG: [u8; 6] = [0xff, 0xd8, 0xff, 0xe0, 0, 0];

fn jpeg() -> UploadedFile {
    UploadedFile {
        name: "CMND mặt trước.JPG".into(),
        mime_type: "image/jpeg".into(),
        bytes: JPEG.to_vec(),
    }
}

async fn app(db: &chatbotx_db::Db) -> MiniApp {
    setup(db, |tx| {
        contact(tx, CONTACT, WS)?;
        Ok(())
    })
    .await;
    let definition = json!({ "screens": [{ "key": "s1", "id": "WELCOME", "title": "", "children": [
        { "id": "p", "type": "PhotoPicker", "props": { "name": "photo", "label": "Photo", "max-uploaded-photos": 2 } }
    ] }] });
    let input = DefinitionInput {
        definition: Some(definition),
        flow_json: None,
        custom_field_mappings: None,
    };
    service::create(db, WS, "Form".into(), input).await.unwrap()
}

fn is_key(text: &str) -> bool {
    text.len() == 22
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[tokio::test]
async fn create_stores_a_contacts_photo_in_the_contact_folder_with_an_unguessable_key() {
    let db = two_workspaces().await;
    let app = app(&db).await;
    let uploader = Uploader::in_memory();
    let out = service::create_upload(
        &db,
        &uploader,
        STORAGE,
        &app,
        Some(CONTACT),
        "photo".into(),
        jpeg(),
    )
    .await
    .unwrap();
    let url = out["url"].as_str().unwrap();
    let path = url.strip_prefix(STORAGE).expect("storage url");
    let prefix = format!("public/space/{WS}/contacts/{CONTACT}/mini-apps/");
    let rest = path.strip_prefix(&prefix).expect("contact folder");
    let (key, name) = rest.split_once('/').expect("key/name");
    assert!(is_key(key), "{path}");
    assert_eq!(name, "cmnd-mat-truoc.jpg");
    assert!(uploader.head_object(path).await.is_ok());
    assert_eq!(out["name"], "CMND mặt trước.JPG");
    assert_eq!(out["mimeType"], "image/jpeg");
    let upload_id = out["uploadId"].as_str().unwrap();
    assert!(is_key(upload_id));
    let stored = rows(
        &db,
        r#"SELECT "contactId", "status", "uploadToken" FROM "MiniAppUpload" WHERE "miniAppId" = ?1"#,
        app.id,
    )
    .await;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0]["contactId"], CONTACT);
    assert_eq!(stored[0]["status"], "pending");
    assert_eq!(stored[0]["uploadToken"], upload_id);
}

#[tokio::test]
async fn create_stores_an_anonymous_upload_under_the_mini_app() {
    let db = two_workspaces().await;
    let app = app(&db).await;
    let out = service::create_upload(
        &db,
        &Uploader::in_memory(),
        STORAGE,
        &app,
        None,
        "photo".into(),
        jpeg(),
    )
    .await
    .unwrap();
    let prefix = format!("{STORAGE}public/space/{WS}/mini-apps/{}/anonymous/", app.id);
    assert!(out["url"].as_str().unwrap().starts_with(&prefix), "{out}");
}

fn reason(err: BusinessError) -> String {
    match err {
        BusinessError::Exception { data, .. } => data
            .and_then(|d| d["reason"].as_str().map(str::to_owned))
            .unwrap_or_default(),
        other => panic!("expected an upload rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn create_rejects_unknown_inputs_and_files_that_are_not_what_they_claim() {
    let db = two_workspaces().await;
    let app = app(&db).await;
    let uploader = Uploader::in_memory();
    let err = service::create_upload(&db, &uploader, STORAGE, &app, None, "nope".into(), jpeg())
        .await
        .unwrap_err();
    assert_eq!(reason(err), "unknown_input");
    let html = UploadedFile {
        name: "x.png".into(),
        mime_type: "image/png".into(),
        bytes: b"<html><script>alert(1)</script>".to_vec(),
    };
    let err = service::create_upload(&db, &uploader, STORAGE, &app, None, "photo".into(), html)
        .await
        .unwrap_err();
    assert!(!reason(err).is_empty());
    let count = rows(
        &db,
        r#"SELECT "id" FROM "MiniAppUpload" WHERE "miniAppId" = ?1"#,
        app.id,
    )
    .await;
    assert!(count.is_empty());
}

#[tokio::test]
async fn claim_rejects_ids_that_are_not_this_visitors_pending_uploads_or_too_many_files() {
    let db = two_workspaces().await;
    let app = app(&db).await;
    let uploader = Uploader::in_memory();
    let mine = service::create_upload(
        &db,
        &uploader,
        STORAGE,
        &app,
        Some(CONTACT),
        "photo".into(),
        jpeg(),
    )
    .await
    .unwrap()["uploadId"]
        .as_str()
        .unwrap()
        .to_owned();
    let submit = |ids: Vec<&str>| {
        service::create_submission(
            &db,
            STORAGE,
            &app,
            Some(CONTACT),
            json!({ "photo": ids }),
            None,
        )
    };
    for ids in [
        vec![mine.as_str(), "someone-else"],
        vec![mine.as_str(), "b", "c"],
    ] {
        match submit(ids).await {
            Err(BusinessError::Exception { field, .. }) => {
                assert_eq!(field.as_deref(), Some("answers"))
            }
            Err(other) => panic!("expected an answers error, got {other:?}"),
            Ok(_) => panic!("expected an answers error"),
        }
    }
    let status = rows(
        &db,
        r#"SELECT "status" FROM "MiniAppUpload" WHERE "miniAppId" = ?1"#,
        app.id,
    )
    .await;
    assert_eq!(status[0]["status"], "pending");
}
