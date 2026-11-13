//! Port of `packages/business/__tests__/contact-resolve-id-by-identifier.test.ts`
//! (P3.6-D2). Adapted: the spied `contactRepository.findIdByIdentityWhere`
//! becomes stored contacts; "before ever querying" is the parser error.

mod common;

use chatbotx_business::BusinessError;
use chatbotx_business::contact::resolve_id_by_identifier;
use common::{OTHER_WS, TestDb, WS, contact_with, setup, two_workspaces};

async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        contact_with(tx, 123, WS, Some("Ada"), Some("ada@example.com"), None)?;
        contact_with(tx, 124, WS, Some("Grace"), None, Some("+841234567890"))?;
        // The same email in another workspace is never matched.
        contact_with(
            tx,
            125,
            OTHER_WS,
            Some("Ada"),
            Some("ada@example.com"),
            None,
        )?;
        Ok(())
    })
    .await;
    db
}

fn code_status_message(error: BusinessError) -> (String, u16, String) {
    match error {
        BusinessError::Exception {
            code,
            status,
            message,
            ..
        } => (code, status, message),
        other => panic!("expected a ChatbotXException, got {other:?}"),
    }
}

#[tokio::test]
async fn resolves_an_id_identifier_by_its_numeric_id() {
    let db = db().await;
    assert_eq!(
        resolve_id_by_identifier(&db, WS, "id:123")
            .await
            .expect("id"),
        123
    );
}

#[tokio::test]
async fn resolves_an_email_identifier_by_email() {
    let db = db().await;
    assert_eq!(
        resolve_id_by_identifier(&db, WS, "email:ada@example.com")
            .await
            .expect("id"),
        123
    );
}

#[tokio::test]
async fn resolves_a_phone_identifier_by_phone_number() {
    let db = db().await;
    assert_eq!(
        resolve_id_by_identifier(&db, WS, "phone:+841234567890")
            .await
            .expect("id"),
        124
    );
}

#[tokio::test]
async fn throws_not_found_404_when_no_contact_matches() {
    let db = db().await;
    let error = resolve_id_by_identifier(&db, WS, "id:999")
        .await
        .expect_err("404");
    let (code, status, message) = code_status_message(error);
    assert_eq!((code.as_str(), status), ("notFound", 404));
    assert_eq!(message, "Contact not found");
    // A contact of another workspace is not found either.
    let error = resolve_id_by_identifier(&db, WS, "id:125")
        .await
        .expect_err("404");
    assert_eq!(code_status_message(error).0, "notFound");
}

#[tokio::test]
async fn rejects_an_id_identifier_with_a_non_numeric_value_before_ever_querying() {
    let db = db().await;
    let error = resolve_id_by_identifier(&db, WS, "id:not-a-number")
        .await
        .expect_err("404");
    let (code, _, message) = code_status_message(error);
    assert_eq!(code, "notFound");
    assert_eq!(message, "Contact not found");
}

#[tokio::test]
async fn rejects_an_identifier_with_an_unrecognized_prefix() {
    let db = db().await;
    let error = resolve_id_by_identifier(&db, WS, "username:ada")
        .await
        .expect_err("404");
    let (_, _, message) = code_status_message(error);
    assert!(message.contains("Invalid identifier format"), "{message}");
}
