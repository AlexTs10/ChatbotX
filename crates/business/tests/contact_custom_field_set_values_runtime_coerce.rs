//! Port of
//! `packages/business/__tests__/contact-custom-field-set-values-runtime-coerce.test.ts`
//! (P3.6-D2 slice contact custom-field values). Adapted: the mocked
//! definitions / values become real rows, `insertValues` the stored rows;
//! the `normalizeCustomFieldValueForStorage` call-count spy becomes the
//! stored canonical values (one normalization per field: a second one of
//! the same canonical value would store the same text).

mod common;

use chatbotx_business::contact_custom_field::{
    NewContactValues, SetValueByKeyInput, SetValuesInput,
    insert_normalized_values_for_new_contacts, set_value_by_key, set_values,
};
use common::{TestDb, WS, contact, custom_field, custom_field_values, setup, two_workspaces};

/// Contact 10; fields 41 `flag` boolean, 42 `amount` number, 43 `note`
/// shortText.
async fn fixture() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        contact(tx, 10, WS)?;
        custom_field(tx, 41, WS, "flag", "boolean")?;
        custom_field(tx, 42, WS, "amount", "number")?;
        custom_field(tx, 43, WS, "note", "shortText")?;
        Ok(())
    })
    .await;
    db
}

fn values(fields: &[(i64, &str)]) -> Vec<(i64, String)> {
    fields
        .iter()
        .map(|(id, v)| (*id, (*v).to_string()))
        .collect()
}

async fn set(db: &TestDb, fields: &[(i64, &str)]) -> chatbotx_business::Result<()> {
    set_values(
        db,
        SetValuesInput {
            workspace_id: WS,
            contact_id: 10,
            fields: values(fields),
            ..SetValuesInput::default()
        },
    )
    .await
    .map(|_| ())
}

fn entries(fields: &[(i64, &str)]) -> Vec<NewContactValues> {
    vec![NewContactValues {
        contact_id: 10,
        fields: values(fields),
    }]
}

// ---- setValues — boolean/number runtime coercion end to end ----

#[tokio::test]
async fn set_values_boolean_number_runtime_coercion_end_to_end_coerces_a_widened_boolean_literal_to_canonical_true()
 {
    let db = fixture().await;
    set(&db, &[(41, "YES")]).await.expect("set");
    assert_eq!(custom_field_values(&db, 10).await, values(&[(41, "true")]));
}

#[tokio::test]
async fn set_values_boolean_number_runtime_coercion_end_to_end_coerces_an_unrecognized_boolean_literal_to_true_instead_of_throwing()
 {
    let db = fixture().await;
    set(&db, &[(41, "12313")]).await.expect("set");
    assert_eq!(custom_field_values(&db, 10).await, values(&[(41, "true")]));
}

#[tokio::test]
async fn set_values_boolean_number_runtime_coercion_end_to_end_canonicalizes_a_number_value() {
    let db = fixture().await;
    set(&db, &[(42, "007")]).await.expect("set");
    assert_eq!(custom_field_values(&db, 10).await, values(&[(42, "7")]));
}

#[tokio::test]
async fn set_values_boolean_number_runtime_coercion_end_to_end_rejects_an_invalid_number_with_a_typed_error_and_persists_nothing()
 {
    let db = fixture().await;
    let error = set(&db, &[(42, "1aaa1")])
        .await
        .expect_err("invalid number");
    assert_eq!(error.code(), Some("invalidCustomFieldValue"));
    assert!(custom_field_values(&db, 10).await.is_empty());
}

#[tokio::test]
async fn set_values_boolean_number_runtime_coercion_end_to_end_normalizes_exactly_once_per_field_via_set_values()
 {
    let db = fixture().await;
    set(&db, &[(41, "yes"), (42, "42")]).await.expect("set");
    assert_eq!(
        custom_field_values(&db, 10).await,
        values(&[(41, "true"), (42, "42")])
    );
}

#[tokio::test]
async fn set_values_boolean_number_runtime_coercion_end_to_end_set_value_by_key_id_based_lookup_the_trigger_action_path_forwards_the_raw_value_to_set_values_without_pre_normalizing_exactly_one_normalize_call()
 {
    let db = fixture().await;
    set_value_by_key(
        &db,
        SetValueByKeyInput {
            workspace_id: WS,
            contact_id: 10,
            keyword: "41".to_string(),
            value: "on".to_string(),
            ..SetValueByKeyInput::default()
        },
    )
    .await
    .expect("set");
    assert_eq!(custom_field_values(&db, 10).await, values(&[(41, "true")]));
}

// ---- insertNormalizedValuesForNewContacts — canonical-contract guard ----

#[tokio::test]
async fn insert_normalized_values_for_new_contacts_canonical_contract_guard_canonical_boolean_number_values_pass_through_and_insert()
 {
    let db = fixture().await;
    insert_normalized_values_for_new_contacts(&db, WS, entries(&[(41, "true"), (42, "1.5")]))
        .await
        .expect("insert");
    assert_eq!(
        custom_field_values(&db, 10).await,
        values(&[(41, "true"), (42, "1.5")])
    );
}

#[tokio::test]
async fn insert_normalized_values_for_new_contacts_canonical_contract_guard_non_canonical_boolean_is_rejected_before_any_insert()
 {
    let db = fixture().await;
    let error = insert_normalized_values_for_new_contacts(&db, WS, entries(&[(41, "TRUE")]))
        .await
        .expect_err("non-canonical");
    assert!(error.message().contains("Non-canonical boolean"), "{error}");
    assert!(custom_field_values(&db, 10).await.is_empty());
}

#[tokio::test]
async fn insert_normalized_values_for_new_contacts_canonical_contract_guard_non_canonical_number_is_rejected_before_any_insert()
 {
    let db = fixture().await;
    let error = insert_normalized_values_for_new_contacts(&db, WS, entries(&[(42, "007")]))
        .await
        .expect_err("non-canonical");
    assert!(error.message().contains("Non-canonical number"), "{error}");
    assert!(custom_field_values(&db, 10).await.is_empty());
}

#[tokio::test]
async fn insert_normalized_values_for_new_contacts_canonical_contract_guard_blank_values_and_non_boolean_number_types_stay_on_the_caller_s_contract()
 {
    let db = fixture().await;
    insert_normalized_values_for_new_contacts(
        &db,
        WS,
        entries(&[(41, ""), (43, "  anything goes  ")]),
    )
    .await
    .expect("insert");
    assert_eq!(
        custom_field_values(&db, 10).await,
        values(&[(41, ""), (43, "  anything goes  ")])
    );
}
