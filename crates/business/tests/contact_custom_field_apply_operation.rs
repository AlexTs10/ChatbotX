//! Port of
//! `packages/business/__tests__/contact-custom-field-apply-operation.test.ts`
//! (P3.6-D2 slice contact custom-field values). Adapted: the mocked contact /
//! field lookups, `listValuesForPairs`, `deleteValue` and the transaction
//! handle become real rows in one write transaction; the call log ("lock"
//! before "write" before "commit", "emit" after "commit") becomes the stored
//! rows after the call (nothing stored after a failure = rolled back); the
//! emitted `customFieldChanged` events become the returned change list (no
//! event bus). "Outside the access scope" is a contact of another workspace
//! (no member access scopes in the port). The injected "db down" failure
//! becomes a rejected later operation (no fault injection); the logger
//! assertion is not observable.

mod common;

use chatbotx_business::BusinessError;
use chatbotx_business::contact_custom_field::{
    FieldChange, FieldOperation, FieldOperationType as Op, apply_operation_to_contacts,
    apply_operations,
};
use common::{
    OTHER_WS, TestDb, WS, contact, custom_field, custom_field_value, custom_field_values, setup,
    two_workspaces,
};
use serde_json::json;

const PLAN: i64 = 61;
const COUNT: i64 = 62;
const COUNT2: i64 = 63;

/// Contacts 1, 2 (`WS`) and 99 (`OTHER_WS`); fields 61 `plan` shortText,
/// 62 `count` number, 63 `count2` number; `stored` = `(contact, field,
/// value)` rows.
async fn fixture(stored: &'static [(i64, i64, &'static str)]) -> TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        contact(tx, 1, WS)?;
        contact(tx, 2, WS)?;
        contact(tx, 99, OTHER_WS)?;
        custom_field(tx, PLAN, WS, "plan", "shortText")?;
        custom_field(tx, COUNT, WS, "count", "number")?;
        custom_field(tx, COUNT2, WS, "count2", "number")?;
        for (n, (c, f, v)) in stored.iter().enumerate() {
            custom_field_value(tx, 800 + n as i64, *c, *f, v)?;
        }
        Ok(())
    })
    .await;
    db
}

async fn to_contacts(
    db: &TestDb,
    contacts: &[i64],
    field: i64,
    operation: Op,
    value: &str,
) -> chatbotx_business::Result<Vec<FieldChange>> {
    apply_operation_to_contacts(db, WS, contacts, field, operation, value, None).await
}

async fn on_one(
    db: &TestDb,
    operations: &[(i64, Op, &str)],
) -> chatbotx_business::Result<Vec<FieldChange>> {
    let operations = operations
        .iter()
        .map(|(field, operation, value)| FieldOperation {
            custom_field_id: *field,
            operation: *operation,
            value: (*value).to_string(),
        })
        .collect();
    apply_operations(db, WS, 1, operations, None).await
}

fn pairs(list: &[(i64, &str)]) -> Vec<(i64, String)> {
    list.iter().map(|(f, v)| (*f, (*v).to_string())).collect()
}

/// `(oldValue, newValue)` of each change.
fn olds_news(changes: &[FieldChange]) -> Vec<(Option<&str>, Option<&str>)> {
    changes
        .iter()
        .map(|c| (c.old_value.as_deref(), c.new_value.as_deref()))
        .collect()
}

fn reason(error: &BusinessError) -> serde_json::Value {
    match error {
        BusinessError::Exception { code, data, .. } => {
            assert_eq!(code, "invalidFieldOperation");
            data.as_ref().map_or(json!(null), |d| d["reason"].clone())
        }
        BusinessError::Repo(other) => panic!("unexpected repo error: {}", other.message()),
    }
}

// ---- contactCustomFieldService.applyFieldOperations ----

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_locks_the_pairs_writes_inside_the_transaction_and_emits_only_after_commit()
 {
    let db = fixture(&[]).await;
    let changes = to_contacts(&db, &[1], PLAN, Op::Set, "pro")
        .await
        .expect("apply");
    assert_eq!(
        changes,
        vec![FieldChange {
            contact_id: 1,
            custom_field_id: PLAN,
            custom_field_name: "plan".to_string(),
            old_value: None,
            new_value: Some("pro".to_string()),
        }]
    );
    assert_eq!(custom_field_values(&db, 1).await, pairs(&[(PLAN, "pro")]));
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_reads_every_current_value_in_one_query_and_takes_one_lock_set()
 {
    let db = fixture(&[(1, PLAN, "a")]).await;
    to_contacts(&db, &[1, 2], PLAN, Op::Append, "x")
        .await
        .expect("apply");
    assert_eq!(custom_field_values(&db, 1).await, pairs(&[(PLAN, "ax")]));
    assert_eq!(custom_field_values(&db, 2).await, pairs(&[(PLAN, "x")]));
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_emits_nothing_when_the_value_does_not_change()
 {
    let db = fixture(&[(1, PLAN, "pro")]).await;
    let changes = to_contacts(&db, &[1], PLAN, Op::Set, "pro")
        .await
        .expect("apply");
    assert!(changes.is_empty());
    assert_eq!(custom_field_values(&db, 1).await, pairs(&[(PLAN, "pro")]));
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_append_increase_and_decrease_are_computed_from_the_stored_value()
 {
    let db = fixture(&[(1, COUNT, "10")]).await;
    let changes = on_one(
        &db,
        &[(COUNT, Op::Increase, "5"), (COUNT, Op::Decrease, "3")],
    )
    .await
    .expect("apply");
    assert_eq!(
        olds_news(&changes),
        vec![(Some("10"), Some("15")), (Some("15"), Some("12"))]
    );
    assert_eq!(custom_field_values(&db, 1).await, pairs(&[(COUNT, "12")]));
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_decrease_on_a_missing_value_yields_the_negative_operand()
 {
    let db = fixture(&[]).await;
    to_contacts(&db, &[1], COUNT, Op::Decrease, "5")
        .await
        .expect("apply");
    assert_eq!(custom_field_values(&db, 1).await, pairs(&[(COUNT, "-5")]));
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_clear_deletes_the_value_and_emits_one_post_commit_event()
 {
    let db = fixture(&[(1, PLAN, "old")]).await;
    let changes = to_contacts(&db, &[1], PLAN, Op::Clear, "")
        .await
        .expect("apply");
    assert_eq!(olds_news(&changes), vec![(Some("old"), None)]);
    assert_eq!(changes[0].custom_field_name, "plan");
    assert!(custom_field_values(&db, 1).await.is_empty());
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_clearing_a_value_that_does_not_exist_emits_nothing()
 {
    let db = fixture(&[]).await;
    let changes = to_contacts(&db, &[1], PLAN, Op::Clear, "")
        .await
        .expect("apply");
    assert!(changes.is_empty());
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_a_rejected_operation_on_one_contact_throws_a_structured_error_and_emits_nothing()
 {
    let db = fixture(&[]).await;
    let error = to_contacts(&db, &[1], COUNT, Op::Increase, "abc")
        .await
        .expect_err("rejected");
    assert_eq!(reason(&error), json!("operandNotNumber"));
    assert!(custom_field_values(&db, 1).await.is_empty());
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_an_operation_the_field_type_does_not_allow_is_rejected()
 {
    let db = fixture(&[]).await;
    let error = to_contacts(&db, &[1], PLAN, Op::Formula, "value + 1")
        .await
        .expect_err("rejected");
    assert_eq!(reason(&error), json!("operationNotAllowed"));
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_a_bulk_edit_skips_the_contacts_a_rejection_hits_and_applies_the_rest()
 {
    let db = fixture(&[(1, COUNT, "1"), (2, COUNT, "not a number")]).await;
    let changes = to_contacts(&db, &[1, 2], COUNT, Op::Increase, "2")
        .await
        .expect("apply");
    assert_eq!(changes.len(), 1);
    assert_eq!(custom_field_values(&db, 1).await, pairs(&[(COUNT, "3")]));
    assert_eq!(
        custom_field_values(&db, 2).await,
        pairs(&[(COUNT, "not a number")])
    );
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_contacts_outside_the_caller_s_access_scope_are_dropped()
 {
    let db = fixture(&[]).await;
    let changes = to_contacts(&db, &[1, 99], PLAN, Op::Set, "pro")
        .await
        .expect("apply");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].contact_id, 1);
    assert!(custom_field_values(&db, 99).await.is_empty());
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_runs_a_whole_batch_in_one_transaction_all_writes_before_the_single_commit()
 {
    let db = fixture(&[]).await;
    let changes = on_one(
        &db,
        &[
            (PLAN, Op::Set, "a"),
            (PLAN, Op::Append, "b"),
            (PLAN, Op::Prepend, "c"),
        ],
    )
    .await
    .expect("apply");
    let news: Vec<_> = changes.iter().map(|c| c.new_value.as_deref()).collect();
    assert_eq!(news, vec![Some("a"), Some("ab"), Some("cab")]);
    assert_eq!(custom_field_values(&db, 1).await, pairs(&[(PLAN, "cab")]));
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_an_unknown_custom_field_fails_before_anything_is_locked_written_or_emitted()
 {
    let db = fixture(&[]).await;
    let error = on_one(&db, &[(PLAN, Op::Set, "a"), (6_999, Op::Set, "b")])
        .await
        .expect_err("unknown");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.message(), "Custom field not found");
    assert!(custom_field_values(&db, 1).await.is_empty());
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_a_failure_after_an_earlier_write_rolls_everything_back_without_events()
 {
    let db = fixture(&[]).await;
    on_one(&db, &[(PLAN, Op::Set, "a"), (PLAN, Op::Increase, "1")])
        .await
        .expect_err("rejected");
    assert!(custom_field_values(&db, 1).await.is_empty());
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_with_one_of_two_requested_contacts_out_of_scope_the_call_is_strict_and_rejects()
 {
    let db = fixture(&[(1, COUNT, "abc")]).await;
    let error = to_contacts(&db, &[1, 99], COUNT, Op::Increase, "2")
        .await
        .expect_err("strict");
    assert_eq!(error.code(), Some("invalidFieldOperation"));
    assert_eq!(custom_field_values(&db, 1).await, pairs(&[(COUNT, "abc")]));
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_with_both_contacts_accessible_the_invalid_one_is_skipped_and_logged()
 {
    let db = fixture(&[(1, COUNT, "abc"), (2, COUNT, "1")]).await;
    let changes = to_contacts(&db, &[1, 2], COUNT, Op::Increase, "2")
        .await
        .expect("apply");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].contact_id, 2);
    assert_eq!(custom_field_values(&db, 2).await, pairs(&[(COUNT, "3")]));
}

// ---- contactCustomFieldService.applyFieldOperations > sequential normalization in one batch ----

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_sequential_normalization_in_one_batch_set_increase_and_formula_feed_each_normalized_value_into_the_next()
 {
    let db = fixture(&[]).await;
    let changes = on_one(
        &db,
        &[
            (COUNT, Op::Set, "007"),
            (COUNT, Op::Increase, "2"),
            (COUNT, Op::Formula, "value*2"),
        ],
    )
    .await
    .expect("apply");
    assert_eq!(
        olds_news(&changes),
        vec![
            (None, Some("7")),
            (Some("7"), Some("9")),
            (Some("9"), Some("18"))
        ]
    );
    assert_eq!(custom_field_values(&db, 1).await, pairs(&[(COUNT, "18")]));
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_sequential_normalization_in_one_batch_an_increase_after_a_clear_starts_from_zero()
 {
    let db = fixture(&[(1, COUNT, "50")]).await;
    let changes = on_one(&db, &[(COUNT, Op::Clear, ""), (COUNT, Op::Increase, "4")])
        .await
        .expect("apply");
    assert_eq!(
        olds_news(&changes),
        vec![(Some("50"), None), (None, Some("4"))]
    );
    assert_eq!(custom_field_values(&db, 1).await, pairs(&[(COUNT, "4")]));
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_sequential_normalization_in_one_batch_operations_on_two_fields_keep_independent_values()
 {
    let db = fixture(&[(1, COUNT, "10"), (1, COUNT2, "100")]).await;
    on_one(
        &db,
        &[
            (COUNT, Op::Increase, "1"),
            (COUNT2, Op::Decrease, "1"),
            (COUNT, Op::Increase, "1"),
        ],
    )
    .await
    .expect("apply");
    assert_eq!(
        custom_field_values(&db, 1).await,
        pairs(&[(COUNT, "12"), (COUNT2, "99")])
    );
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_appending_a_padded_operand_stores_it_verbatim_without_trimming()
 {
    let db = fixture(&[(1, PLAN, "a")]).await;
    to_contacts(&db, &[1], PLAN, Op::Append, " x ")
        .await
        .expect("apply");
    assert_eq!(custom_field_values(&db, 1).await, pairs(&[(PLAN, "a x ")]));
}

#[tokio::test]
async fn contact_custom_field_service_apply_field_operations_appending_a_whitespace_only_operand_writes_it_only_an_empty_operand_is_unchanged()
 {
    let db = fixture(&[(1, PLAN, "a"), (2, PLAN, "a")]).await;
    to_contacts(&db, &[1], PLAN, Op::Append, "   ")
        .await
        .expect("apply");
    assert_eq!(custom_field_values(&db, 1).await, pairs(&[(PLAN, "a   ")]));
    let changes = to_contacts(&db, &[2], PLAN, Op::Append, "")
        .await
        .expect("apply");
    assert!(changes.is_empty());
    assert_eq!(custom_field_values(&db, 2).await, pairs(&[(PLAN, "a")]));
}
