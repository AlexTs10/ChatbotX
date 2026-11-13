//! Port of `packages/business/__tests__/contact-note-service.test.ts`.
//! Adapted: the mocked `findByIdOrFail` / database become real rows (a
//! contact of another workspace is "out of scope"); "invalidates the
//! contact-notes cache tag" becomes the stored row (no read cache).

mod common;

use chatbotx_business::contact_note as svc;
use common::{OTHER_WS, WS, contact, scalar_i64, setup, two_workspaces};

/// Contacts 10 (WS) and 20 (OTHER_WS); note 100 of contact 10, note 200 of
/// contact 20.
async fn fixture() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        contact(tx, 10, WS)?;
        contact(tx, 20, OTHER_WS)?;
        tx.execute(
            r#"INSERT INTO "ContactNote" ("id", "text", "contactId", "createdAt", "updatedAt")
               VALUES (100, 'mine', 10, 1000, 1000), (200, 'theirs', 20, 1000, 1000)"#,
            [],
        )?;
        Ok(())
    })
    .await;
    db
}

async fn notes_of(db: &chatbotx_db::Db, contact_id: i64) -> i64 {
    scalar_i64(
        db,
        r#"SELECT count(*) FROM "ContactNote" WHERE "contactId" = ?1"#,
        contact_id,
    )
    .await
    .unwrap_or_default()
}

fn not_found(error: chatbotx_business::BusinessError, message: &str) {
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.message(), message);
}

#[tokio::test]
async fn create_throws_before_writing_when_the_contact_is_out_of_scope() {
    let db = fixture().await;
    let error = svc::create(&db, WS, 20, "x", None)
        .await
        .expect_err("scope");
    not_found(error, "Contact not found");
    assert_eq!(notes_of(&db, 20).await, 1);
}

#[tokio::test]
async fn inserts_the_note_and_invalidates_the_contact_notes_cache_tag() {
    let db = fixture().await;
    let note = svc::create(&db, WS, 10, "hello", None)
        .await
        .expect("create");
    assert_eq!(
        (note.contact_id, note.text.as_str(), note.created_by_id),
        (10, "hello", None)
    );
    assert_eq!(notes_of(&db, 10).await, 2);
}

#[tokio::test]
async fn update_throws_before_writing_when_the_contact_is_out_of_scope() {
    let db = fixture().await;
    let error = svc::update(&db, WS, 20, 200, Some("x"))
        .await
        .expect_err("scope");
    not_found(error, "Contact not found");
}

#[tokio::test]
async fn update_404s_when_the_note_does_not_exist() {
    let db = fixture().await;
    for note_id in [999, 200] {
        let error = svc::update(&db, WS, 10, note_id, Some("x"))
            .await
            .expect_err("missing");
        not_found(error, "Contact note not found");
    }
}

#[tokio::test]
async fn updates_the_note_and_invalidates_the_contact_notes_cache_tag() {
    let db = fixture().await;
    let note = svc::update(&db, WS, 10, 100, Some("edited"))
        .await
        .expect("update");
    assert_eq!(
        (note.id, note.text.as_str(), note.created_at),
        (100, "edited", 1000)
    );
}

#[tokio::test]
async fn list_throws_without_querying_notes_when_the_contact_is_out_of_scope() {
    let db = fixture().await;
    let error = svc::list_by_contact_id(&db, WS, 20)
        .await
        .expect_err("scope");
    not_found(error, "Contact not found");
}

#[tokio::test]
async fn scopes_the_contact_lookup_by_workspace_id_before_listing_notes() {
    let db = fixture().await;
    let notes = svc::list_by_contact_id(&db, WS, 10).await.expect("list");
    assert_eq!(notes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![100]);
    let notes = svc::list_by_contact_id(&db, OTHER_WS, 20)
        .await
        .expect("list");
    assert_eq!(notes.iter().map(|n| n.id).collect::<Vec<_>>(), vec![200]);
}

#[tokio::test]
async fn delete_throws_before_deleting_when_the_contact_is_out_of_scope() {
    let db = fixture().await;
    let error = svc::delete(&db, WS, 20, 200).await.expect_err("scope");
    not_found(error, "Contact not found");
    assert_eq!(notes_of(&db, 20).await, 1);
}

#[tokio::test]
async fn delete_404s_when_zero_rows_were_deleted_missing_note_matching_update_s_behavior() {
    let db = fixture().await;
    let error = svc::delete(&db, WS, 10, 200).await.expect_err("missing");
    not_found(error, "Contact note not found");
    assert_eq!(notes_of(&db, 20).await, 1);
}

#[tokio::test]
async fn deletes_the_note_and_invalidates_the_contact_notes_cache_tag() {
    let db = fixture().await;
    svc::delete(&db, WS, 10, 100).await.expect("delete");
    assert_eq!(notes_of(&db, 10).await, 0);
}
