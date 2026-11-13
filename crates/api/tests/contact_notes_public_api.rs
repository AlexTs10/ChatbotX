//! Port of `apps/builder/__tests__/contacts-notes-public-api.test.ts`
//! (P3.6-D2). Adapted: the captured oRPC handlers become `handlers::dispatch`
//! calls over a real migrated database; "the service was called with" becomes
//! the response and the stored rows.

mod handlers_common;

use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, WS, call, fixture, scalar};
use serde_json::{Value, json};

/// Contact 123 (`a@b.com`) with note 7 "hello".
async fn seed(f: &Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "email", "workspaceId") VALUES (123, 'Ada', 'a@b.com', ?1)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactNote" ("id", "text", "contactId") VALUES (7, 'hello', 123)"#,
                [],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
}

#[tokio::test]
async fn get_v1_contacts_identifier_notes_resolves_the_identifier_then_lists_notes_by_contact_id() {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "contacts.listNotes",
        WS,
        &[("identifier", "email:a@b.com")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["data"][0]["id"], json!("7"));
    assert_eq!(body["data"][0]["text"], json!("hello"));
    assert_eq!(body["data"][0]["contactId"], json!("123"));
}

#[tokio::test]
async fn post_v1_contacts_identifier_notes_creates_a_note_with_a_null_user_id_no_session_for_a_token_caller()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "contacts.createNote",
        WS,
        &[("identifier", "id:123")],
        None,
        Some(json!({ "text": "hi" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["text"], json!("hi"));
    assert_eq!(body["createdById"], Value::Null);
}

#[tokio::test]
async fn put_v1_contacts_identifier_notes_note_id_delegates_to_edit_contact_note_with_the_resolved_contact_id()
 {
    let f = fixture().await;
    seed(&f).await;
    let (status, body) = call(
        &f,
        "contacts.updateNote",
        WS,
        &[("identifier", "id:123"), ("noteId", "7")],
        None,
        Some(json!({ "text": "updated" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        (&body["id"], &body["text"]),
        (&json!("7"), &json!("updated"))
    );
}

#[tokio::test]
async fn delete_v1_contacts_identifier_notes_note_id_delegates_to_delete_contact_note() {
    let f = fixture().await;
    seed(&f).await;
    let (status, _) = call(
        &f,
        "contacts.deleteNote",
        WS,
        &[("identifier", "id:123"), ("noteId", "7")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    let left = scalar(
        &f,
        r#"SELECT count(*) FROM "ContactNote" WHERE "contactId" = ?1"#,
        123,
    )
    .await;
    assert_eq!(left, Some(json!(0)));
}
