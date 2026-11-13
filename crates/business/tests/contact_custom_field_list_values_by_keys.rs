//! Port of
//! `packages/business/__tests__/contact-custom-field-list-values-by-keys.test.ts`
//! (P3.6-D2 slice contact custom-field values). Adapted: the mocked
//! `listValuesByFieldKeys` rows become real `CustomField` /
//! `ContactCustomField` rows (`updatedAt` set); "skips the query" becomes
//! the empty result for bot-field-only / empty keys.

mod common;

use chatbotx_business::contact_custom_field::{ContactFieldValue, list_values_by_keys};
use common::{TestDb, WS, contact, custom_field, custom_field_value, setup, two_workspaces};

/// Contact 10; fields 5 "Shop location", 7 "Home", 8 "Work", 9 named "7";
/// values with `updatedAt` 2, 1, 5, 9 ms.
async fn fixture() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        contact(tx, 10, WS)?;
        for (id, name, value, at) in [
            (5_i64, "Shop location", "1,2", 2_i64),
            (7, "Home", "3,4", 1),
            (8, "Work", "5,6", 5),
            (9, "7", "old", 9),
        ] {
            custom_field(tx, id, WS, name, "shortText")?;
            custom_field_value(tx, 700 + id, 10, id, value)?;
            tx.execute(
                r#"UPDATE "ContactCustomField" SET "updatedAt" = ?1 WHERE "id" = ?2"#,
                chatbotx_db::rusqlite::params![at, 700 + id],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

fn value(key: &str, value: &str, updated_at: i64) -> ContactFieldValue {
    ContactFieldValue {
        key: key.to_string(),
        value: value.to_string(),
        updated_at,
    }
}

fn keys(list: &[&str]) -> Vec<String> {
    list.iter().map(|k| (*k).to_string()).collect()
}

// ---- contactCustomFieldService.listValuesByKeys ----

#[tokio::test]
async fn contact_custom_field_service_list_values_by_keys_resolves_name_keys() {
    let db = fixture().await;
    let rows = list_values_by_keys(&db, WS, 10, keys(&["Shop location", "bot_field:3"]))
        .await
        .expect("list");
    assert_eq!(rows, vec![value("Shop location", "1,2", 2)]);
}

#[tokio::test]
async fn contact_custom_field_service_list_values_by_keys_prefers_an_id_match_over_a_same_named_field_and_orders_newest_first()
 {
    let db = fixture().await;
    let rows = list_values_by_keys(&db, WS, 10, keys(&["7", "Work"]))
        .await
        .expect("list");
    assert_eq!(rows, vec![value("Work", "5,6", 5), value("7", "3,4", 1)]);
}

#[tokio::test]
async fn contact_custom_field_service_list_values_by_keys_only_bot_fields_or_no_keys_skips_the_query()
 {
    let db = fixture().await;
    assert!(
        list_values_by_keys(&db, WS, 10, keys(&["bot_field:3"]))
            .await
            .expect("list")
            .is_empty()
    );
    assert!(
        list_values_by_keys(&db, WS, 10, Vec::new())
            .await
            .expect("list")
            .is_empty()
    );
}
