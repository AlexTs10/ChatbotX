//! Ports of `packages/business/__tests__/mini-app-duplicate.test.ts` and
//! `mini-app-submission-custom-fields.test.ts`. Adapted: the mocked
//! repositories and `setValues` become real rows; "inserted with" becomes the
//! stored row, "setValues called with" the stored custom-field values. The
//! parallel-insert retries of `duplicate` are covered by `copy_name.rs`
//! (`insertWithFreeCopyName`; test map).

mod common;

use chatbotx_business::mini_app::{self as service, DefinitionInput};
use chatbotx_core::mini_app::NAME_MAX_LENGTH;
use chatbotx_db::rusqlite::params;
use common::{WS, contact, custom_field, rows, setup, two_workspaces};
use serde_json::{Value, json};

const CONTACT: i64 = 7001;
const CF_NAME: i64 = 3001;
const CF_TOPICS: i64 = 3002;
const CF_PHOTO: i64 = 3003;
const STORAGE: &str = "https://cdn.test/";

fn starter() -> DefinitionInput {
    DefinitionInput {
        definition: Some(chatbotx_core::mini_app::starter_definition().to_value()),
        flow_json: None,
        custom_field_mappings: None,
    }
}

async fn names(db: &chatbotx_db::Db) -> Vec<String> {
    rows(
        db,
        r#"SELECT "name" FROM "MiniApp" WHERE "workspaceId" = ?1 ORDER BY "id""#,
        WS,
    )
    .await
    .iter()
    .map(|r| r["name"].as_str().unwrap_or_default().to_owned())
    .collect()
}

#[tokio::test]
async fn duplicate_picks_the_first_free_copy_name_and_inserts_once() {
    let db = two_workspaces().await;
    let source = service::create(&db, WS, "X".into(), starter())
        .await
        .unwrap();
    for name in ["X - Copy", "X - Copy 2", "X - Copy 3"] {
        service::create(&db, WS, name.into(), starter())
            .await
            .unwrap();
    }
    let copy = service::duplicate(&db, WS, source.id).await.unwrap();
    assert_eq!(copy.name, "X - Copy 4");
    assert_eq!(copy.definition.0, source.definition.0);
    assert_eq!(names(&db).await.len(), 5);
}

#[tokio::test]
async fn duplicate_cuts_a_long_name_so_copy_names_fit_the_limit() {
    let db = two_workspaces().await;
    let source = service::create(&db, WS, "a".repeat(NAME_MAX_LENGTH), starter())
        .await
        .unwrap();
    let copy = service::duplicate(&db, WS, source.id).await.unwrap();
    assert_eq!(copy.name.chars().count(), NAME_MAX_LENGTH);
    assert!(copy.name.ends_with(" - Copy"));
}

// ---- submissions ------------------------------------------------------------------

fn definition() -> Value {
    let mut definition = json!({ "screens": [{ "key": "s1", "id": "WELCOME", "title": "", "children": [
        { "id": "n1", "type": "TextInput", "props": { "name": "full_name", "label": "Name" } },
        { "id": "n2", "type": "CheckboxGroup", "props": { "name": "topics", "label": "Topics", "data-source": [] } },
        { "id": "n3", "type": "TextInput", "props": { "name": "note", "label": "Note" } },
        { "id": "n4", "type": "PhotoPicker", "props": { "name": "photo", "label": "Photo", "max-uploaded-photos": 2 } }
    ] }] });
    for (index, id) in [(0, CF_NAME), (1, CF_TOPICS), (3, CF_PHOTO)] {
        definition["screens"][0]["children"][index]["customFieldId"] = json!(id.to_string());
    }
    definition
}

/// A Mini App with mapped inputs; `name_kind` is the type of the name field.
async fn submission_db(
    name_kind: &'static str,
) -> (common::TestDb, chatbotx_db::repo::models::MiniApp) {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        contact(tx, CONTACT, WS)?;
        custom_field(tx, CF_NAME, WS, "name", name_kind)?;
        custom_field(tx, CF_TOPICS, WS, "topics", "shortText")?;
        custom_field(tx, CF_PHOTO, WS, "photo", "shortText")?;
        Ok(())
    })
    .await;
    let input = DefinitionInput {
        definition: Some(definition()),
        flow_json: None,
        custom_field_mappings: None,
    };
    let app = service::create(&db, WS, "Form".into(), input)
        .await
        .unwrap();
    (db, app)
}

async fn values(db: &chatbotx_db::Db) -> Vec<(i64, String)> {
    rows(
        db,
        r#"SELECT "customFieldId", "value" FROM "ContactCustomField" WHERE "contactId" = ?1 ORDER BY "customFieldId""#,
        CONTACT,
    )
    .await
    .iter()
    .map(|r| (r["customFieldId"].as_i64().unwrap_or_default(), r["value"].as_str().unwrap_or_default().to_owned()))
    .collect()
}

fn changed(changes: &[chatbotx_business::contact_custom_field::FieldChange]) -> Vec<i64> {
    changes.iter().map(|c| c.custom_field_id).collect()
}

#[tokio::test]
async fn create_writes_mapped_answers_to_the_contacts_custom_fields() {
    let (db, app) = submission_db("shortText").await;
    let answers = json!({ "full_name": "Ana", "topics": ["a", "b"], "note": "unmapped" });
    let submitted = service::create_submission(
        &db,
        STORAGE,
        &app,
        Some(CONTACT),
        answers,
        Some("Asia/Ho_Chi_Minh".into()),
    )
    .await
    .unwrap();
    assert_eq!(
        values(&db).await,
        [(CF_NAME, "Ana".to_owned()), (CF_TOPICS, "a, b".to_owned())]
    );
    assert_eq!(changed(&submitted.changes), [CF_NAME, CF_TOPICS]);
}

#[tokio::test]
async fn create_one_rejected_value_does_not_drop_the_other_fields_or_fail_the_submission() {
    let (db, app) = submission_db("number").await;
    let answers = json!({ "full_name": "Ana", "topics": ["a"] });
    let submitted = service::create_submission(&db, STORAGE, &app, Some(CONTACT), answers, None)
        .await
        .unwrap();
    assert_eq!(values(&db).await, [(CF_TOPICS, "a".to_owned())]);
    assert_eq!(changed(&submitted.changes), [CF_TOPICS]);
    assert_eq!(submitted.submission.mini_app_id, app.id);
}

#[tokio::test]
async fn create_does_not_write_anything_for_an_anonymous_submission() {
    let (db, app) = submission_db("shortText").await;
    let submitted = service::create_submission(
        &db,
        STORAGE,
        &app,
        None,
        json!({ "full_name": "Ana" }),
        None,
    )
    .await
    .unwrap();
    assert!(values(&db).await.is_empty());
    assert!(submitted.changes.is_empty());
}

#[tokio::test]
async fn create_swaps_upload_ids_for_files_and_saves_their_public_urls() {
    let (db, app) = submission_db("shortText").await;
    let (a, b) = ("AAAAAAAAAAAAAAAAAAAAAA", "BBBBBBBBBBBBBBBBBBBBBB");
    let app_id = app.id;
    setup(&db, move |tx| {
        for (id, token, path) in [(9001, a, "public/a.jpg"), (9002, b, "public/b.jpg")] {
            tx.execute(
                r#"INSERT INTO "MiniAppUpload" ("id", "workspaceId", "miniAppId", "contactId", "inputName", "uploadToken",
                     "path", "fileName", "mimeType", "size", "status")
                   VALUES (?1, ?2, ?3, ?4, 'photo', ?5, ?6, 'x.jpg', 'image/jpeg', 1, 'pending')"#,
                params![id, WS, app_id, CONTACT, token, path],
            )?;
        }
        Ok(())
    })
    .await;
    let submitted = service::create_submission(
        &db,
        STORAGE,
        &app,
        Some(CONTACT),
        json!({ "photo": [a, b] }),
        None,
    )
    .await
    .unwrap();
    let files = submitted.submission.answers.0["photo"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let urls: Vec<&str> = files.iter().filter_map(|f| f["url"].as_str()).collect();
    assert_eq!(urls.len(), 2);
    assert!(
        urls[0].ends_with("a.jpg") && urls[1].ends_with("b.jpg"),
        "{urls:?}"
    );
    assert_eq!(values(&db).await, [(CF_PHOTO, urls.join(", "))]);
    let status = rows(
        &db,
        r#"SELECT "status" FROM "MiniAppUpload" WHERE "miniAppId" = ?1"#,
        app.id,
    )
    .await;
    assert!(status.iter().all(|r| r["status"] == "submitted"));
}

#[tokio::test]
async fn save_screen_answers_saves_a_screens_mapped_answers_right_away_without_files() {
    let (db, app) = submission_db("shortText").await;
    let answers = json!({ "full_name": "Ana", "photo": ["AAAAAAAAAAAAAAAAAAAAAA"] });
    let saved = service::save_screen_answers(&db, &app, Some(CONTACT), "s1".into(), answers, None)
        .await
        .unwrap();
    assert_eq!(saved.map(|c| changed(&c)), Some(vec![CF_NAME]));
    assert_eq!(values(&db).await, [(CF_NAME, "Ana".to_owned())]);
}

#[tokio::test]
async fn save_screen_answers_saves_nothing_for_an_anonymous_visitor_or_an_unknown_screen() {
    let (db, app) = submission_db("shortText").await;
    let answers = || json!({ "full_name": "Ana" });
    assert!(
        service::save_screen_answers(&db, &app, None, "s1".into(), answers(), None)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        service::save_screen_answers(&db, &app, Some(CONTACT), "missing".into(), answers(), None)
            .await
            .unwrap()
            .is_none()
    );
    assert!(values(&db).await.is_empty());
}
