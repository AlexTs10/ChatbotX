//! Port of `packages/business/src/place-near-me/__tests__/contact-location.test.ts`.
//! Adapted: the mocked `listValuesByKeys` becomes real `CustomField` /
//! `ContactCustomField` rows; "called with the input" becomes the keys the
//! rows are found by (an id key and a legacy name key).

mod common;

use chatbotx_business::place_near_me::contact_location::ContactLocationInput;
use chatbotx_business::place_near_me::{Coordinates, find_contact_location};
use chatbotx_db::rusqlite::params;
use common::{WS, contact, setup, two_workspaces};

const CONTACT: i64 = 4001;

/// Fields `f1` (id 501, name "Home") and `f2` (id 502, name "Office") with values.
async fn seeded(f2_value: Option<&'static str>, f1_value: Option<&'static str>) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        contact(tx, CONTACT, WS)?;
        for (id, name) in [(501_i64, "Home"), (502, "Office")] {
            tx.execute(
                r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (?1, ?2, 'shortText', ?3)"#,
                params![id, name, WS],
            )?;
        }
        if let Some(value) = f2_value {
            tx.execute(
                r#"INSERT INTO "ContactCustomField" ("id", "value", "contactId", "customFieldId", "updatedAt") VALUES (602, ?1, ?2, 502, 2)"#,
                params![value, CONTACT],
            )?;
        }
        if let Some(value) = f1_value {
            tx.execute(
                r#"INSERT INTO "ContactCustomField" ("id", "value", "contactId", "customFieldId", "updatedAt") VALUES (601, ?1, ?2, 501, 1)"#,
                params![value, CONTACT],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

fn input() -> ContactLocationInput {
    ContactLocationInput {
        workspace_id: WS,
        contact_id: CONTACT,
        keys: vec!["501".to_string(), "Office".to_string()],
    }
}

#[tokio::test]
async fn find_contact_location_returns_the_newest_valid_value() {
    let db = seeded(Some("10.5,20.25"), Some("1,2")).await;
    let found = find_contact_location(&db, input()).await.expect("lookup");
    assert_eq!(
        found,
        Some(Coordinates {
            latitude: 10.5,
            longitude: 20.25
        })
    );
}

#[tokio::test]
async fn find_contact_location_falls_through_invalid_newest_value() {
    let db = seeded(Some("hello"), Some("1,2")).await;
    let found = find_contact_location(&db, input()).await.expect("lookup");
    assert_eq!(
        found,
        Some(Coordinates {
            latitude: 1.0,
            longitude: 2.0
        })
    );
}

#[tokio::test]
async fn find_contact_location_returns_null_when_there_are_no_rows() {
    let db = seeded(None, None).await;
    assert_eq!(
        find_contact_location(&db, input()).await.expect("lookup"),
        None
    );
}
