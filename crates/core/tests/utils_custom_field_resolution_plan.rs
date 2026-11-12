//! Port of `packages/utils/__tests__/custom-field-resolution-plan.test.ts`.

use chatbotx_core::utils::custom_field::{
    CustomFieldRequest, CustomFieldResolutionPlan, CustomFieldRow, CustomFieldType,
    pick_custom_field_row, plan_custom_field_resolution,
};

fn row(id: &str, name: &str, kind: CustomFieldType) -> CustomFieldRow {
    CustomFieldRow {
        id: id.to_owned(),
        name: name.to_owned(),
        kind,
    }
}

fn request(name: &str, kind: CustomFieldType) -> CustomFieldRequest {
    CustomFieldRequest {
        name: name.to_owned(),
        kind,
    }
}

fn rename(name: &str) -> CustomFieldResolutionPlan {
    CustomFieldResolutionPlan::Rename {
        planned_name: name.to_owned(),
    }
}

use CustomFieldType::{Number, ShortText};

/// Port of: packages/utils/__tests__/custom-field-resolution-plan.test.ts:14 "pickCustomFieldRow > an exact trimmed-name match beats a case-only match, then the lowest id wins"
#[test]
fn pickcustomfieldrow_an_exact_trimmed_name_match_beats_a_case_only_match_then_the_lowest_id_wins()
{
    assert_eq!(
        pick_custom_field_row(
            Some(&row("5", "Age", Number)),
            &row("9", "age", Number),
            Some("age")
        )
        .id,
        "9"
    );
    assert_eq!(
        pick_custom_field_row(
            Some(&row("9", "Age", Number)),
            &row("3", "AGE", Number),
            Some("age")
        )
        .id,
        "3"
    );
    assert_eq!(
        pick_custom_field_row(None, &row("9", "Age", Number), Some("age")).id,
        "9"
    );
}

/// Port of: packages/utils/__tests__/custom-field-resolution-plan.test.ts:36 "planCustomFieldResolution > reuses a field of the same type whatever its casing, picking the install's winner"
#[test]
fn plancustomfieldresolution_reuses_a_field_of_the_same_type_whatever_its_casing_picking_the_install_s_winner()
 {
    let plan = plan_custom_field_resolution(
        &[row("9", "Age", Number), row("5", "age", Number)],
        &[request("age", Number)],
    );
    match plan.plans.get("number:age") {
        Some(CustomFieldResolutionPlan::Reuse(winner)) => assert_eq!(winner.id, "5"),
        other => panic!("expected reuse, got {other:?}"),
    }
}

/// Port of: packages/utils/__tests__/custom-field-resolution-plan.test.ts:47 "planCustomFieldResolution > a name used by another type is renamed, an unused one is created as written"
#[test]
fn plancustomfieldresolution_a_name_used_by_another_type_is_renamed_an_unused_one_is_created_as_written()
 {
    let plan = plan_custom_field_resolution(
        &[row("1", "Age", Number)],
        &[request("age", ShortText), request("City", ShortText)],
    );
    assert_eq!(plan.plans.get("shortText:age"), Some(&rename("age_1")));
    assert_eq!(
        plan.plans.get("shortText:city"),
        Some(&CustomFieldResolutionPlan::Create {
            planned_name: "City".to_owned()
        })
    );
}

/// Port of: packages/utils/__tests__/custom-field-resolution-plan.test.ts:65 "planCustomFieldResolution > two requested fields sharing a name across types: the second is renamed"
#[test]
fn plancustomfieldresolution_two_requested_fields_sharing_a_name_across_types_the_second_is_renamed()
 {
    let plan =
        plan_custom_field_resolution(&[], &[request("age", ShortText), request("Age", Number)]);
    assert!(matches!(
        plan.plans.get("shortText:age"),
        Some(CustomFieldResolutionPlan::Create { .. })
    ));
    assert_eq!(plan.plans.get("number:age"), Some(&rename("Age_1")));
}

/// Port of: packages/utils/__tests__/custom-field-resolution-plan.test.ts:80 "planCustomFieldResolution > requested fields that fold to one key are a single request (the last stands)"
#[test]
fn plancustomfieldresolution_requested_fields_that_fold_to_one_key_are_a_single_request_the_last_stands()
 {
    let plan = plan_custom_field_resolution(
        &[],
        &[request("age", ShortText), request("AGE ", ShortText)],
    );
    assert_eq!(plan.requests, vec![request("AGE ", ShortText)]);
    assert_eq!(plan.plans.len(), 1);
}

/// Port of: packages/utils/__tests__/custom-field-resolution-plan.test.ts:92 "planCustomFieldResolution > a rename skips a numbered name that already exists"
#[test]
fn plancustomfieldresolution_a_rename_skips_a_numbered_name_that_already_exists() {
    let plan = plan_custom_field_resolution(
        &[row("1", "Age", Number), row("2", "age_1", ShortText)],
        &[request("age", ShortText)],
    );
    assert_eq!(plan.plans.get("shortText:age"), Some(&rename("age_2")));
}

/// Port of: packages/utils/__tests__/custom-field-resolution-plan.test.ts:103 "planCustomFieldResolution > a rename never collides with a name planned earlier in the same request"
#[test]
fn plancustomfieldresolution_a_rename_never_collides_with_a_name_planned_earlier_in_the_same_request()
 {
    let plan = plan_custom_field_resolution(
        &[row("1", "Age", Number)],
        &[request("age", ShortText), request("age_1", Number)],
    );
    assert_eq!(plan.plans.get("shortText:age"), Some(&rename("age_1")));
    assert_eq!(plan.plans.get("number:age_1"), Some(&rename("age_1_1")));
}
