//! Port of `packages/business/__tests__/contact-custom-field-delete.test.ts`
//! (P3.6-D2 slice contact custom-field values). Adapted: the mocked field
//! lookup / `findValue` / delete become real rows; `emitCustomFieldChanged`
//! becomes the returned change (no event bus); the cache invalidation is not
//! observable (no read cache in the port), the stored rows are asserted.

mod common;

use chatbotx_business::contact_custom_field::{
    FieldChange, delete_by_custom_field_id, delete_by_key,
};
use common::{
    TestDb, WS, contact, custom_field, custom_field_value, custom_field_values, setup,
    two_workspaces,
};

/// Contacts 1-3; fields 51 `Birthday` (date), 52 `Plan` (shortText);
/// contact 1 has a Birthday, contacts 1-3 a Plan.
async fn fixture() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        for id in [1, 2, 3] {
            contact(tx, id, WS)?;
        }
        custom_field(tx, 51, WS, "Birthday", "date")?;
        custom_field(tx, 52, WS, "Plan", "shortText")?;
        custom_field_value(tx, 601, 1, 51, "2026-07-21T17:00:00.000Z")?;
        for (id, c) in [(611, 1), (612, 2), (613, 3)] {
            custom_field_value(tx, id, c, 52, "pro")?;
        }
        Ok(())
    })
    .await;
    db
}

fn plan(value: &str) -> (i64, String) {
    (52, value.to_string())
}

// ---- contactCustomFieldService.deleteByKey ----

#[tokio::test]
async fn contact_custom_field_service_delete_by_key_resolves_the_keyword_snapshots_the_value_and_emits_custom_field_changed_old_value_null()
 {
    let db = fixture().await;
    let change = delete_by_key(&db, WS, 1, "Birthday", false)
        .await
        .expect("delete");
    assert_eq!(
        change,
        Some(FieldChange {
            contact_id: 1,
            custom_field_id: 51,
            custom_field_name: "Birthday".to_string(),
            old_value: Some("2026-07-21T17:00:00.000Z".to_string()),
            new_value: None,
        })
    );
    assert_eq!(custom_field_values(&db, 1).await, vec![plan("pro")]);
}

#[tokio::test]
async fn contact_custom_field_service_delete_by_key_does_not_emit_when_the_contact_had_no_stored_value_no_op_delete_but_still_deletes_and_invalidates()
 {
    let db = fixture().await;
    let change = delete_by_key(&db, WS, 2, "Birthday", false)
        .await
        .expect("delete");
    assert_eq!(change, None);
    assert_eq!(custom_field_values(&db, 2).await, vec![plan("pro")]);
}

#[tokio::test]
async fn contact_custom_field_service_delete_by_key_throws_and_neither_reads_deletes_nor_emits_when_the_keyword_cannot_be_resolved()
 {
    let db = fixture().await;
    let error = delete_by_key(&db, WS, 1, "does-not-exist", false)
        .await
        .expect_err("unknown");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.message(), "Custom field not found");
    assert_eq!(custom_field_values(&db, 1).await.len(), 2);
}

// ---- contactCustomFieldService.deleteByCustomFieldId ----

#[tokio::test]
async fn contact_custom_field_service_delete_by_custom_field_id_bulk_delete_stays_event_free_and_invalidates_every_target_s_cache_in_one_batched_call()
 {
    let db = fixture().await;
    delete_by_custom_field_id(&db, WS, &[1, 2, 3], 52)
        .await
        .expect("delete");
    assert_eq!(
        custom_field_values(&db, 1).await,
        vec![(51, "2026-07-21T17:00:00.000Z".to_string())]
    );
    assert!(custom_field_values(&db, 2).await.is_empty());
    assert!(custom_field_values(&db, 3).await.is_empty());
}

#[tokio::test]
async fn contact_custom_field_service_delete_by_custom_field_id_is_a_no_op_for_an_empty_contact_ids_list()
 {
    let db = fixture().await;
    delete_by_custom_field_id(&db, WS, &[], 52)
        .await
        .expect("delete");
    for c in [1, 2, 3] {
        assert!(custom_field_values(&db, c).await.contains(&plan("pro")));
    }
}
