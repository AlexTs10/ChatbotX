//! Port of `packages/database/__tests__/pipeline-partial.test.ts` (P3.2b, ported:
//! `safeParse(..).success` is `parse_..(..).is_ok()`; `undefined` is `None`; each
//! `test.each` row is one test; `pipelineActivityKinds.options` is
//! `pipeline_item_filter::PIPELINE_ACTIVITY_KINDS`).

use chatbotx_db::repo::partials::pipeline::{
    CustomFieldType, PIPELINE_ACTIVITY_KINDS_BY_GROUP, PIPELINE_LIMITS, PIPELINE_TYPES,
    PipelineFieldCondition, PipelineFieldOperator, PipelineType, activity_kinds_of,
    is_pipeline_filter_day, parse_pipeline_reference_name, parse_pipeline_settings,
    pipeline_field_condition_error, pipeline_type_defaults, reference_name_from_name,
};
use chatbotx_db::repo::pipeline_item_filter::PIPELINE_ACTIVITY_KINDS;
use serde_json::{Value, json};

#[test]
fn pipeline_settings_schema_parses_an_empty_object_by_backfilling_every_key() {
    let settings = parse_pipeline_settings(&json!({})).expect("parse");
    assert_eq!(settings.version, 1);
    assert!(!settings.assign_contact_on_assign);
    assert_eq!(settings.max_open_items_per_contact, None);
    assert!(settings.card_elements.card_id);
    assert!(settings.card_elements.due_date);
    assert_eq!(
        serde_json::to_value(&settings.status_labels).expect("json"),
        json!({ "open": "Open", "won": "Won", "lost": "Lost" })
    );
}

#[test]
fn pipeline_settings_schema_backfills_keys_added_after_a_row_was_stored() {
    let settings =
        parse_pipeline_settings(&json!({ "cardElements": { "cardId": false } })).expect("parse");
    assert!(!settings.card_elements.card_id);
    assert!(settings.card_elements.comments);
}

fn assert_rejects_max_open_items(value: Value) {
    assert!(
        parse_pipeline_settings(&json!({ "maxOpenItemsPerContact": value })).is_err(),
        "{value}"
    );
}

#[test]
fn pipeline_settings_schema_rejects_max_open_items_per_contact_0() {
    assert_rejects_max_open_items(json!(0));
}

#[test]
fn pipeline_settings_schema_rejects_max_open_items_per_contact_101() {
    assert_rejects_max_open_items(json!(101));
}

#[test]
fn pipeline_settings_schema_rejects_max_open_items_per_contact_1_5() {
    assert_rejects_max_open_items(json!(1.5));
}

#[test]
fn pipeline_settings_schema_rejects_an_empty_status_label() {
    assert!(parse_pipeline_settings(&json!({ "statusLabels": { "won": "  " } })).is_err());
}

#[test]
fn pipeline_reference_name_schema_accepts_sales() {
    assert!(parse_pipeline_reference_name("sales").is_ok());
}

#[test]
fn pipeline_reference_name_schema_accepts_support_2() {
    assert!(parse_pipeline_reference_name("support_2").is_ok());
}

#[test]
fn pipeline_reference_name_schema_accepts_a() {
    assert!(parse_pipeline_reference_name("a").is_ok());
}

#[test]
fn pipeline_reference_name_schema_rejects_empty() {
    assert!(parse_pipeline_reference_name("").is_err());
}

#[test]
fn pipeline_reference_name_schema_rejects_sales_capitalized() {
    assert!(parse_pipeline_reference_name("Sales").is_err());
}

#[test]
fn pipeline_reference_name_schema_rejects_2fast() {
    assert!(parse_pipeline_reference_name("2fast").is_err());
}

#[test]
fn pipeline_reference_name_schema_rejects_has_space() {
    assert!(parse_pipeline_reference_name("has space").is_err());
}

#[test]
fn pipeline_reference_name_schema_rejects_a_b() {
    assert!(parse_pipeline_reference_name("a-b").is_err());
}

#[test]
fn pipeline_reference_name_schema_rejects_65_characters() {
    assert!(parse_pipeline_reference_name(&"x".repeat(65)).is_err());
}

#[test]
fn pipeline_type_defaults_covers_every_pipeline_type_with_a_first_stage_and_status_labels() {
    for pipeline_type in PIPELINE_TYPES {
        let defaults = pipeline_type_defaults(pipeline_type);
        assert!(!defaults.first_stage_name.is_empty(), "{pipeline_type:?}");
        assert!(!defaults.status_labels.0.is_empty(), "{pipeline_type:?}");
    }
}

#[test]
fn pipeline_type_defaults_maps_each_type_to_its_ui_noun() {
    assert_eq!(
        pipeline_type_defaults(PipelineType::Opportunity).noun_key,
        "opportunity"
    );
    assert_eq!(
        pipeline_type_defaults(PipelineType::Ticket).noun_key,
        "ticket"
    );
    assert_eq!(
        pipeline_type_defaults(PipelineType::Custom).noun_key,
        "card"
    );
}

#[test]
fn pipeline_type_defaults_keeps_the_bulk_batch_smaller_than_the_synchronous_threshold() {
    const { assert!(PIPELINE_LIMITS.bulk_batch_size < PIPELINE_LIMITS.bulk_sync_items) };
}

#[test]
fn reference_name_from_name_folds_vietnamese_diacritics_and_the_stroked_d() {
    assert_eq!(
        reference_name_from_name("Đàm phán khách hàng", "pipeline"),
        "dam_phan_khach_hang"
    );
    assert_eq!(
        reference_name_from_name("Customer Support", "pipeline"),
        "customer_support"
    );
}

#[test]
fn reference_name_from_name_prefixes_names_that_do_not_start_with_a_letter() {
    assert_eq!(
        reference_name_from_name("2024 Plan", "pipeline"),
        "pipeline_2024_plan"
    );
}

#[test]
fn activity_groups_every_activity_kind_belongs_to_exactly_one_group() {
    let mut grouped: Vec<&str> = PIPELINE_ACTIVITY_KINDS_BY_GROUP
        .iter()
        .flat_map(|(_, kinds)| kinds.iter().copied())
        .collect();
    grouped.sort_unstable();
    let mut kinds = PIPELINE_ACTIVITY_KINDS.to_vec();
    kinds.sort_unstable();
    assert_eq!(grouped, kinds);
}

#[test]
fn activity_groups_no_group_means_no_kind_filter_groups_expand_to_their_kinds() {
    assert_eq!(activity_kinds_of(None), None);
    assert_eq!(activity_kinds_of(Some(&[])), None);
    assert_eq!(
        activity_kinds_of(Some(&["comments", "stage"])),
        Some(vec!["commentAdded", "commentDeleted", "stageChanged"])
    );
}

#[test]
fn custom_field_filter_operands_a_filter_day_must_be_a_real_day_in_years_0001_9999() {
    assert!(is_pipeline_filter_day(Some("2026-05-20")));
    assert!(is_pipeline_filter_day(Some("0001-01-01")));
    assert!(!is_pipeline_filter_day(Some("0000-01-01")));
    assert!(!is_pipeline_filter_day(Some("2026-02-30")));
}

#[test]
fn custom_field_filter_operands_date_and_datetime_fields_both_take_a_yyyy_mm_dd_operand() {
    for field_type in [CustomFieldType::Date, CustomFieldType::Datetime] {
        assert!(
            pipeline_field_condition_error(
                field_type,
                &PipelineFieldCondition::with(PipelineFieldOperator::Is, "0000-01-01"),
            )
            .is_some(),
            "{field_type:?}"
        );
        assert_eq!(
            pipeline_field_condition_error(
                field_type,
                &PipelineFieldCondition::with(PipelineFieldOperator::Is, "2026-05-20"),
            ),
            None,
            "{field_type:?}"
        );
    }
}
