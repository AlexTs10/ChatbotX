//! Port of `packages/business/__tests__/contact-list-by-custom-field-value.test.ts`
//! (P3.6-D2). Adapted: the spied `contactRepository.listPublicByCustomField`
//! becomes stored contacts; the asserted `where` / limit / order become the
//! returned contacts.

mod common;

use chatbotx_business::contact::list_by_custom_field_value;
use common::{OTHER_WS, TestDb, WS, contact_with, custom_field, setup, two_workspaces};

async fn db() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        contact_with(tx, 101, WS, Some("Ada"), Some("ada@example.com"), Some("+15551234567"))?;
        contact_with(tx, 102, WS, Some("Grace"), Some("grace@example.com"), None)?;
        contact_with(tx, 103, OTHER_WS, Some("Other"), Some("ada@example.com"), Some("+15551234567"))?;
        custom_field(tx, 1, WS, "tier", "shortText")?;
        tx.execute(
            r#"INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value") VALUES (1, 102, 1, 'gold')"#,
            [],
        )?;
        Ok(())
    })
    .await;
    db
}

fn ids(contacts: &[chatbotx_db::repo::contact::PublicContact]) -> Vec<i64> {
    contacts.iter().map(|c| c.contact.id).collect()
}

#[tokio::test]
async fn custom_field_id_email_maps_to_the_native_email_column() {
    let db = db().await;
    let found = list_by_custom_field_value(&db, WS, "email", "ada@example.com")
        .await
        .expect("list");
    assert_eq!(ids(&found), vec![101]);
}

#[tokio::test]
async fn custom_field_id_phone_maps_to_the_native_phone_number_column() {
    let db = db().await;
    let found = list_by_custom_field_value(&db, WS, "phone", "+15551234567")
        .await
        .expect("list");
    assert_eq!(ids(&found), vec![101]);
}

#[tokio::test]
async fn any_other_custom_field_id_addresses_a_real_custom_field_row() {
    let db = db().await;
    let found = list_by_custom_field_value(&db, WS, "1", "gold")
        .await
        .expect("list");
    assert_eq!(ids(&found), vec![102]);
    assert_eq!(found[0].contact_custom_fields.len(), 1);
}

#[tokio::test]
async fn returns_an_empty_data_array_when_nothing_matches() {
    let db = db().await;
    let found = list_by_custom_field_value(&db, WS, "cf-1", "no-such-value")
        .await
        .expect("list");
    assert!(found.is_empty());
    let found = list_by_custom_field_value(&db, WS, "1", "no-such-value")
        .await
        .expect("list");
    assert!(found.is_empty());
}

#[tokio::test]
async fn passes_through_whatever_the_repository_returns_capped_at_100_rows() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        for i in 0..105_i64 {
            contact_with(tx, 1000 + i, WS, Some("C"), Some("same@example.com"), None)?;
            tx.execute(
                r#"UPDATE "Contact" SET "updatedAt" = ?2 WHERE "id" = ?1"#,
                chatbotx_db::rusqlite::params![1000 + i, 1_000_000 + i],
            )?;
        }
        Ok(())
    })
    .await;
    let found = list_by_custom_field_value(&db, WS, "email", "same@example.com")
        .await
        .expect("list");
    assert_eq!(found.len(), 100);
    // `updatedAt desc`: the latest updated contact first.
    assert_eq!(found[0].contact.id, 1104);
    assert_eq!(found[99].contact.id, 1005);
}
