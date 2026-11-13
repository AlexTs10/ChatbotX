//! Port of `packages/business/__tests__/bot-field-value-operation.test.ts`
//! (P3.6-D2 slice bot fields). Adapted: the mocked `botFieldModel`
//! lookups, `updateSet` / `updateReturning` and the locked `select ... for
//! update` become real `BotField` rows (the asserted write = the stored
//! value and the returned row); "throws a ChatbotXException" =
//! `BusinessError::Exception`; "vanished" rows = an unknown key. The
//! template-lock order, cache invalidation, injected DB failures and the
//! `resolveByNameAndType` cases are not-applicable (no template locks, no
//! read cache, no fault injection; the template install planner is not
//! ported in this slice).

mod common;

use chatbotx_business::BusinessError;
use chatbotx_business::bot_field::{
    BotField, CreateBotField, FieldOperationType as Op, ListBotFields, TemporalInputParsing,
    UpdateBotField, ValueOperation, apply_value_operation, bulk_clear_values, bulk_update_by_keys,
    clear_value_by_key, create, list, update_by_key,
};
use chatbotx_db::rusqlite::params;
use common::{TestDb, WS, bot_field, bot_field_value, setup, two_workspaces};
use serde_json::json;

const VN: &str = "Asia/Ho_Chi_Minh";

/// `WS` (zone `Asia/Ho_Chi_Minh`) with bot field 1 "field" of `kind` and
/// `value`, and bot field 2 "other" (number, "1").
async fn with_field(kind: &'static str, value: Option<&'static str>) -> TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        tx.execute(
            r#"UPDATE "Workspace" SET "timezone" = ?1 WHERE "id" = ?2"#,
            params![VN, WS],
        )?;
        bot_field(tx, 1, WS, "field", kind, value, 1_000)?;
        bot_field(tx, 2, WS, "other", "number", Some("1"), 2_000)?;
        Ok(())
    })
    .await;
    db
}

async fn apply(db: &TestDb, input: ValueOperation) -> chatbotx_business::Result<BotField> {
    apply_value_operation(db, WS, input).await
}

async fn op(db: &TestDb, operation: Op, value: &str) -> chatbotx_business::Result<BotField> {
    apply(db, ValueOperation::new("field", operation, value)).await
}

fn with_zone(operation: Op, value: &str) -> ValueOperation {
    ValueOperation {
        source_timezone_override: Some(VN.to_string()),
        ..ValueOperation::new("field", operation, value)
    }
}

async fn stored(db: &TestDb) -> Option<String> {
    bot_field_value(db, 1).await.expect("row 1")
}

fn some(text: &str) -> Option<String> {
    Some(text.to_string())
}

fn code(error: &BusinessError) -> Option<&str> {
    error.code()
}

fn reason(error: &BusinessError) -> Option<serde_json::Value> {
    match error {
        BusinessError::Exception { data, .. } => data.as_ref().map(|d| d["reason"].clone()),
        BusinessError::Repo(_) => None,
    }
}

// ---- botFieldService.list ----

#[tokio::test]
async fn bot_field_service_list_sorts_and_counts_against_bot_field_model_not_custom_field_model() {
    let db = with_field("shortText", None).await;
    let page = list(&db, WS, ListBotFields::default()).await.expect("list");
    let ids: Vec<i64> = page.data.iter().map(|f| f.id).collect();
    assert_eq!(ids, vec![1, 2]);
    assert_eq!(page.page_count, 1);
}

// ---- botFieldService.applyValueOperation — set ----

#[tokio::test]
async fn bot_field_service_apply_value_operation_set_accepts_set_for_non_temporal_type_s() {
    for (kind, value, expected) in [
        ("shortText", "after", "after"),
        ("longText", "after", "after"),
        ("number", "42", "42"),
        ("boolean", "true", "true"),
        ("email", "after", "after"),
        ("phoneNumber", "after", "after"),
    ] {
        let db = with_field(kind, Some("before")).await;
        let row = op(&db, Op::Set, value).await.expect(kind);
        assert_eq!(row.value, some(expected), "{kind}");
        assert_eq!(stored(&db).await, some(expected), "{kind}");
    }
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_set_a_name_patch_locks_the_field_before_the_stored_row_is_re_read_then_diffs_the_fresh_row()
 {
    let db = with_field("shortText", None).await;
    let row = update_by_key(
        &db,
        WS,
        "field",
        UpdateBotField {
            name: Some("next".to_string()),
            ..UpdateBotField::default()
        },
    )
    .await
    .expect("update");
    assert_eq!(row.name, "next");
    assert_eq!(row.id, 1);
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_set_coerces_an_unrecognized_boolean_set_value_to_true_generous_runtime_coercion_never_throws()
 {
    let db = with_field("boolean", None).await;
    let row = op(&db, Op::Set, "after").await.expect("set");
    assert_eq!(row.value, some("true"));
    assert_eq!(stored(&db).await, some("true"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_set_a_blank_boolean_set_value_stays_empty_means_unset_not_false()
 {
    let db = with_field("boolean", Some("true")).await;
    let row = op(&db, Op::Set, "").await.expect("set");
    assert_eq!(row.value, some(""));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_set_rejects_an_unparseable_number_set_value_with_a_typed_error_instead_of_persisting_garbage()
 {
    let db = with_field("number", Some("3")).await;
    let error = op(&db, Op::Set, "1aaa1").await.expect_err("invalid");
    assert!(matches!(error, BusinessError::Exception { .. }));
    assert_eq!(stored(&db).await, some("3"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_set_canonicalizes_a_number_set_value_e_g_007_7() {
    let db = with_field("number", None).await;
    let row = op(&db, Op::Set, "007").await.expect("set");
    assert_eq!(row.value, some("7"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_set_normalizes_a_naive_date_value_using_the_caller_s_source_timezone_override()
 {
    let db = with_field("date", None).await;
    let row = apply(&db, with_zone(Op::Set, "2026-07-22"))
        .await
        .expect("set");
    assert_eq!(row.value, some("2026-07-22T00:00:00+07:00"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_set_normalizes_a_naive_datetime_value_using_the_caller_s_source_timezone_override()
 {
    let db = with_field("datetime", None).await;
    let row = apply(&db, with_zone(Op::Set, "2026-07-22 15:30"))
        .await
        .expect("set");
    assert_eq!(row.value, some("2026-07-22T08:30:00.000Z"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_set_rejects_a_date_value_that_cannot_be_normalized_instead_of_persisting_garbage()
 {
    let db = with_field("date", None).await;
    let error = apply(&db, with_zone(Op::Set, "not-a-date"))
        .await
        .expect_err("invalid");
    assert_eq!(code(&error), Some("invalidFieldOperation"));
    assert_eq!(stored(&db).await, None);
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_set_accepts_a_lenient_non_iso_date_value_for_a_date_bot_field_when_temporal_input_parsing_is_lenient()
 {
    let db = with_field("date", None).await;
    let input = ValueOperation {
        temporal_input_parsing: Some(TemporalInputParsing::Lenient),
        ..with_zone(Op::Set, "23/07/2026")
    };
    let row = apply(&db, input).await.expect("set");
    assert_eq!(row.value, some("2026-07-23T00:00:00+07:00"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_set_rejects_the_same_lenient_only_value_under_the_strict_default_proving_lenient_was_actually_forwarded()
 {
    let db = with_field("date", None).await;
    let error = apply(&db, with_zone(Op::Set, "23/07/2026"))
        .await
        .expect_err("strict");
    assert_eq!(code(&error), Some("invalidFieldOperation"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_set_a_blank_value_with_fill_empty_temporal_with_now_stamps_the_current_date_instead_of_persisting_an_empty_string()
 {
    let db = with_field("date", None).await;
    let input = ValueOperation {
        fill_empty_temporal_with_now: true,
        ..with_zone(Op::Set, "")
    };
    let row = apply(&db, input).await.expect("set");
    let value = row.value.expect("value");
    assert!(value.ends_with("T00:00:00+07:00"), "{value}");
    assert_eq!(stored(&db).await, Some(value));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_set_a_blank_value_without_fill_empty_temporal_with_now_stores_the_empty_string_unchanged_instead_of_stamping_now()
 {
    let db = with_field("date", Some("2026-01-01T00:00:00+07:00")).await;
    let row = apply(&db, with_zone(Op::Set, "")).await.expect("set");
    assert_eq!(row.value, some(""));
    assert_eq!(stored(&db).await, some(""));
}

// ---- botFieldService.applyValueOperation — append/prepend/increase/decrease ----

#[tokio::test]
async fn bot_field_service_apply_value_operation_append_prepend_increase_decrease_append_is_allowed_for_s_and_runs_as_one_atomic_update()
 {
    for kind in ["shortText", "longText"] {
        let db = with_field(kind, Some("foo")).await;
        let row = op(&db, Op::Append, "bar").await.expect(kind);
        assert_eq!(row.value, some("foobar"), "{kind}");
    }
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_append_prepend_increase_decrease_prepend_concatenates_on_the_left_via_a_single_atomic_update()
 {
    let db = with_field("longText", Some("bar")).await;
    let row = op(&db, Op::Prepend, "foo").await.expect("prepend");
    assert_eq!(row.value, some("foobar"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_append_prepend_increase_decrease_increase_runs_a_single_atomic_numeric_update_for_type_number()
 {
    let db = with_field("number", Some("10")).await;
    let row = op(&db, Op::Increase, "5").await.expect("increase");
    assert_eq!(row.value, some("15"));
    assert_eq!(stored(&db).await, some("15"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_append_prepend_increase_decrease_decrease_runs_a_single_atomic_numeric_update_for_type_number()
 {
    let db = with_field("number", Some("10")).await;
    let row = op(&db, Op::Decrease, "5").await.expect("decrease");
    assert_eq!(row.value, some("5"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_append_prepend_increase_decrease_a_historical_non_numeric_value_under_increase_fails_predictably_instead_of_a_raw_db_error()
 {
    let db = with_field("number", Some("not-a-number")).await;
    let error = op(&db, Op::Increase, "5").await.expect_err("non-numeric");
    assert_eq!(code(&error), Some("invalidFieldOperation"));
    assert_eq!(stored(&db).await, some("not-a-number"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_append_prepend_increase_decrease_a_historical_non_numeric_value_under_decrease_fails_predictably_instead_of_a_raw_db_error()
 {
    let db = with_field("number", Some("not-a-number")).await;
    let error = op(&db, Op::Decrease, "5").await.expect_err("non-numeric");
    assert_eq!(code(&error), Some("invalidFieldOperation"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_append_prepend_increase_decrease_throws_not_found_when_the_atomic_update_affects_no_row_id_vanished_mid_flight()
 {
    let db = with_field("number", Some("10")).await;
    let error = apply(&db, ValueOperation::new("missing", Op::Increase, "5"))
        .await
        .expect_err("missing");
    assert_eq!(error.message(), "Bot field not found");
}

// ---- botFieldService.applyValueOperation — operation x type policy ----

#[tokio::test]
async fn bot_field_service_apply_value_operation_operation_x_type_policy_s_is_rejected_for_type_s()
{
    let text_only = [Op::Append, Op::Prepend];
    let number_only = [Op::Increase, Op::Decrease];
    let non_text = [
        "number",
        "boolean",
        "date",
        "datetime",
        "email",
        "phoneNumber",
    ];
    let non_number = [
        "shortText",
        "longText",
        "boolean",
        "date",
        "datetime",
        "email",
        "phoneNumber",
    ];
    let cases = text_only
        .iter()
        .flat_map(|o| non_text.iter().map(move |t| (*o, *t)))
        .chain(
            number_only
                .iter()
                .flat_map(|o| non_number.iter().map(move |t| (*o, *t))),
        );
    for (operation, kind) in cases {
        let db = with_field(kind, Some("old")).await;
        let error = op(&db, operation, "x").await.expect_err(kind);
        assert_eq!(
            code(&error),
            Some("invalidFieldOperation"),
            "{operation} {kind}"
        );
        assert_eq!(stored(&db).await, some("old"), "{operation} {kind}");
    }
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_operation_x_type_policy_set_is_allowed_for_every_type_including_boolean_date_datetime_which_reject_every_other_operation()
 {
    for (kind, value) in [
        ("boolean", "true"),
        ("date", "2026-07-22"),
        ("datetime", "2026-07-22 15:30"),
    ] {
        let db = with_field(kind, Some("old")).await;
        apply(&db, with_zone(Op::Set, value)).await.expect(kind);
    }
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_operation_x_type_policy_thrown_exceptions_are_chatbot_x_exception_instances()
 {
    let db = with_field("boolean", None).await;
    let error = op(&db, Op::Increase, "1").await.expect_err("not allowed");
    assert!(matches!(error, BusinessError::Exception { .. }));
}

// ---- botFieldService.applyValueOperation — error contract ----

#[tokio::test]
async fn bot_field_service_apply_value_operation_error_contract_an_unsupported_operation_for_the_type_throws_invalid_field_operation_with_its_message_and_no_reason()
 {
    let db = with_field("boolean", None).await;
    let error = op(&db, Op::Increase, "1").await.expect_err("not allowed");
    assert_eq!(code(&error), Some("invalidFieldOperation"));
    assert_eq!(
        error.message(),
        "Operation \"O04\" is not supported for field type \"boolean\""
    );
    assert_eq!(reason(&error), None);
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_error_contract_a_calculator_rejection_carries_the_machine_readable_reason()
 {
    let db = with_field("number", Some("4")).await;
    let error = op(&db, Op::Formula, "1 +").await.expect_err("rejected");
    assert_eq!(code(&error), Some("invalidFieldOperation"));
    assert_eq!(reason(&error), Some(json!("invalidExpression")));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_error_contract_s_on_a_number_field_rejects_a_non_numeric_operand_with_the_documented_message_and_no_reason()
 {
    for operation in [Op::Increase, Op::Decrease] {
        let db = with_field("number", Some("10")).await;
        let error = op(&db, operation, "abc").await.expect_err("non-numeric");
        assert_eq!(code(&error), Some("invalidFieldOperation"));
        assert_eq!(
            error.message(),
            "\"abc\" is not a valid number value for this field."
        );
        assert_eq!(reason(&error), None);
    }
}

// ---- botFieldService.applyValueOperation — calculated operations ----

#[tokio::test]
async fn bot_field_service_apply_value_operation_calculated_operations_clear_nulls_the_value_under_the_row_lock()
 {
    let db = with_field("shortText", Some("old")).await;
    let row = op(&db, Op::Clear, "").await.expect("clear");
    assert_eq!(row.value, None);
    assert_eq!(bot_field_value(&db, 1).await, Some(None));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_calculated_operations_formula_is_computed_from_the_locked_value()
 {
    let db = with_field("number", Some("4")).await;
    let row = op(&db, Op::Formula, "value * 2 + 1")
        .await
        .expect("formula");
    assert_eq!(row.value, some("9"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_calculated_operations_set_now_stamps_the_current_instant_on_a_datetime_field()
 {
    let db = with_field("datetime", None).await;
    let row = op(&db, Op::SetNow, "").await.expect("setNow");
    let value = row.value.expect("value");
    assert!(value.ends_with('Z') && value.len() == 24, "{value}");
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_calculated_operations_add_days_moves_a_stored_datetime_by_calendar_days()
 {
    let db = with_field("datetime", Some("2026-03-10T00:00:00.000Z")).await;
    let row = op(&db, Op::AddDays, "3").await.expect("addDays");
    assert_eq!(row.value, some("2026-03-13T00:00:00.000Z"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_calculated_operations_a_rejected_calculation_throws_a_typed_error_with_the_reason_and_writes_nothing()
 {
    let db = with_field("number", Some("4")).await;
    let error = op(&db, Op::Formula, "1 +").await.expect_err("rejected");
    assert_eq!(reason(&error), Some(json!("invalidExpression")));
    assert_eq!(stored(&db).await, some("4"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_calculated_operations_an_operation_the_type_does_not_allow_never_reaches_the_calculator()
 {
    let db = with_field("shortText", Some("x")).await;
    let error = op(&db, Op::Formula, "value + 1")
        .await
        .expect_err("not allowed");
    assert_eq!(code(&error), Some("invalidFieldOperation"));
    assert_eq!(reason(&error), None);
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_calculated_operations_throws_not_found_when_the_row_vanishes_before_the_lock()
 {
    let db = with_field("number", Some("4")).await;
    let error = apply(&db, ValueOperation::new("missing", Op::Formula, "1"))
        .await
        .expect_err("missing");
    assert_eq!(error.message(), "Bot field not found");
}

// ---- botFieldService.clearValueByKey ----

#[tokio::test]
async fn bot_field_service_clear_value_by_key_nulls_the_value_without_deleting_the_row() {
    let db = with_field("shortText", Some("keep-the-row")).await;
    let row = clear_value_by_key(&db, WS, "field").await.expect("clear");
    assert_eq!(row.id, 1);
    assert_eq!(row.value, None);
    assert_eq!(bot_field_value(&db, 1).await, Some(None));
}

#[tokio::test]
async fn bot_field_service_clear_value_by_key_throws_not_found_when_the_row_is_deleted_concurrently_between_the_lookup_and_the_update()
 {
    let db = with_field("shortText", Some("keep-the-row")).await;
    let error = clear_value_by_key(&db, WS, "missing")
        .await
        .expect_err("missing");
    assert_eq!(error.message(), "Bot field not found");
}

// ---- botFieldService.bulkClearValues ----

#[tokio::test]
async fn bot_field_service_bulk_clear_values_nulls_the_values_of_the_selected_rows_and_invalidates_their_tags()
 {
    let db = with_field("shortText", Some("a")).await;
    bulk_clear_values(&db, WS, &[1, 2]).await.expect("clear");
    assert_eq!(bot_field_value(&db, 1).await, Some(None));
    assert_eq!(bot_field_value(&db, 2).await, Some(None));
}

#[tokio::test]
async fn bot_field_service_bulk_clear_values_is_a_no_op_for_an_empty_id_list() {
    let db = with_field("shortText", Some("a")).await;
    bulk_clear_values(&db, WS, &[]).await.expect("clear");
    assert_eq!(stored(&db).await, some("a"));
}

// ---- botFieldService.create — value normalization ----

async fn create_with(
    db: &TestDb,
    kind: &str,
    value: Option<&str>,
) -> chatbotx_business::Result<BotField> {
    create(
        db,
        WS,
        CreateBotField {
            name: "new".to_string(),
            r#type: kind.to_string(),
            value: value.map(str::to_string),
            ..CreateBotField::default()
        },
    )
    .await
}

#[tokio::test]
async fn bot_field_service_create_value_normalization_coerces_a_boolean_value_before_insert() {
    let db = with_field("shortText", None).await;
    let row = create_with(&db, "boolean", Some("yes"))
        .await
        .expect("create");
    assert_eq!(row.value, some("true"));
}

#[tokio::test]
async fn bot_field_service_create_value_normalization_canonicalizes_a_number_value_before_insert() {
    let db = with_field("shortText", None).await;
    let row = create_with(&db, "number", Some("007"))
        .await
        .expect("create");
    assert_eq!(row.value, some("7"));
}

#[tokio::test]
async fn bot_field_service_create_value_normalization_rejects_an_unparseable_number_value_instead_of_inserting_garbage()
 {
    let db = with_field("shortText", None).await;
    let error = create_with(&db, "number", Some("1aaa1"))
        .await
        .expect_err("invalid");
    assert!(matches!(error, BusinessError::Exception { .. }));
    let page = list(&db, WS, ListBotFields::default()).await.expect("list");
    assert_eq!(page.data.len(), 2);
}

#[tokio::test]
async fn bot_field_service_create_value_normalization_normalizes_a_date_value_against_the_workspace_timezone()
 {
    let db = with_field("shortText", None).await;
    let row = create_with(&db, "date", Some("2026-07-22"))
        .await
        .expect("create");
    assert_eq!(row.value, some("2026-07-22T00:00:00+07:00"));
}

#[tokio::test]
async fn bot_field_service_create_value_normalization_leaves_a_null_value_alone_no_normalization_attempted()
 {
    let db = with_field("shortText", None).await;
    let row = create_with(&db, "boolean", None).await.expect("create");
    assert_eq!(row.value, None);
}

// ---- botFieldService.updateByKey — value normalization ----

fn value_patch(value: &str) -> UpdateBotField {
    UpdateBotField {
        value: Some(Some(value.to_string())),
        ..UpdateBotField::default()
    }
}

#[tokio::test]
async fn bot_field_service_update_by_key_value_normalization_coerces_a_boolean_value_using_the_existing_row_s_type_when_the_patch_omits_type()
 {
    let db = with_field("boolean", None).await;
    let row = update_by_key(&db, WS, "field", value_patch("on"))
        .await
        .expect("update");
    assert_eq!(row.value, some("true"));
}

#[tokio::test]
async fn bot_field_service_update_by_key_value_normalization_normalizes_against_the_new_type_when_the_patch_changes_type_and_value_together()
 {
    let db = with_field("shortText", None).await;
    let patch = UpdateBotField {
        r#type: Some("number".to_string()),
        ..value_patch("05")
    };
    let row = update_by_key(&db, WS, "field", patch)
        .await
        .expect("update");
    assert_eq!((row.r#type.as_str(), row.value), ("number", some("5")));
}

#[tokio::test]
async fn bot_field_service_update_by_key_value_normalization_rejects_an_unparseable_number_value_instead_of_persisting_garbage()
 {
    let db = with_field("number", Some("3")).await;
    let error = update_by_key(&db, WS, "field", value_patch("1aaa1"))
        .await
        .expect_err("invalid");
    assert!(matches!(error, BusinessError::Exception { .. }));
    assert_eq!(stored(&db).await, some("3"));
}

#[tokio::test]
async fn bot_field_service_update_by_key_value_normalization_leaves_an_undefined_value_alone_a_patch_that_doesn_t_touch_value()
 {
    let db = with_field("number", Some("3")).await;
    let patch = UpdateBotField {
        description: Some(Some("new desc".to_string())),
        ..UpdateBotField::default()
    };
    let row = update_by_key(&db, WS, "field", patch)
        .await
        .expect("update");
    assert_eq!(row.description, some("new desc"));
    assert_eq!(row.value, some("3"));
}

// ---- botFieldService.bulkUpdateByKeys — delegates to updateByKey (inherits normalization) ----

#[tokio::test]
async fn bot_field_service_bulk_update_by_keys_delegates_to_update_by_key_inherits_normalization_coerces_each_update_s_value_via_the_same_primitive_as_update_by_key()
 {
    let db = with_field("boolean", None).await;
    bulk_update_by_keys(
        &db,
        WS,
        vec![
            ("1".to_string(), "yes".to_string()),
            ("2".to_string(), "007".to_string()),
        ],
    )
    .await
    .expect("bulk");
    assert_eq!(stored(&db).await, some("true"));
    assert_eq!(bot_field_value(&db, 2).await, Some(some("7")));
}

// ---- botFieldService.applyValueOperation — increase/decrease operand validation ----

#[tokio::test]
async fn bot_field_service_apply_value_operation_increase_decrease_operand_validation_rejects_a_non_numeric_operand_with_a_typed_error_before_touching_the_db()
 {
    let db = with_field("number", Some("10")).await;
    let error = op(&db, Op::Increase, "not-a-number")
        .await
        .expect_err("invalid");
    assert_eq!(code(&error), Some("invalidFieldOperation"));
    assert_eq!(stored(&db).await, some("10"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_increase_decrease_operand_validation_canonicalizes_the_operand_before_it_reaches_the_atomic_sql()
 {
    let db = with_field("number", Some("10")).await;
    let row = op(&db, Op::Increase, "007").await.expect("increase");
    assert_eq!(row.value, some("17"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_increase_decrease_operand_validation_trims_a_whitespace_padded_operand_and_adds_the_canonical_value()
 {
    let db = with_field("number", Some("10")).await;
    let row = op(&db, Op::Increase, " 1.5 ").await.expect("increase");
    assert_eq!(row.value, some("11.5"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_increase_decrease_operand_validation_accepts_a_prefixed_operand_widened_number_vocabulary()
 {
    let db = with_field("number", Some("10")).await;
    let row = op(&db, Op::Increase, "+2").await.expect("increase");
    assert_eq!(row.value, some("12"));
}

#[tokio::test]
async fn bot_field_service_apply_value_operation_increase_decrease_operand_validation_still_rejects_an_unparseable_operand_with_a_typed_error()
 {
    let db = with_field("number", Some("10")).await;
    let error = op(&db, Op::Decrease, "1aaa1").await.expect_err("invalid");
    assert_eq!(code(&error), Some("invalidFieldOperation"));
}
