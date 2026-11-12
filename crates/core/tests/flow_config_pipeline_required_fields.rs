//! Port of `packages/flow-config/__tests__/pipeline-required-fields.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn fields() -> Value {
    json!([
        { "id": "1", "name": "Budget", "required": true },
        { "id": "2", "name": "Notes", "required": false },
        { "id": "3", "name": "Company", "required": true },
    ])
}

fn key<'a>(fields: &'a [Value], k: &str) -> Vec<&'a str> {
    fields.iter().map(|f| f[k].as_str().unwrap()).collect()
}

#[test]
fn findmissingrequiredpipelinefields_lists_every_required_field_without_a_row_in_definition_order()
{
    let missing = fc::find_missing_required_pipeline_fields(&json!([]), &fields());
    assert_eq!(key(&missing, "name"), vec!["Budget", "Company"]);
}

#[test]
fn findmissingrequiredpipelinefields_a_blank_or_whitespace_only_value_counts_as_missing() {
    let missing = fc::find_missing_required_pipeline_fields(
        &json!([
            { "pipelineFieldId": "1", "value": "   " },
            { "pipelineFieldId": "3", "value": "" },
        ]),
        &fields(),
    );
    assert_eq!(key(&missing, "id"), vec!["1", "3"]);
}

#[test]
fn findmissingrequiredpipelinefields_a_literal_value_or_a_variable_satisfies_the_field() {
    assert!(
        fc::find_missing_required_pipeline_fields(
            &json!([
                { "pipelineFieldId": "1", "value": "1000" },
                { "pipelineFieldId": "3", "value": "{{company}}" },
            ]),
            &fields(),
        )
        .is_empty()
    );
}

#[test]
fn findmissingrequiredpipelinefields_optional_fields_are_never_reported() {
    assert!(
        fc::find_missing_required_pipeline_fields(
            &json!([]),
            &json!([{ "id": "2", "name": "Notes", "required": false }]),
        )
        .is_empty()
    );
}

#[test]
fn findmissingrequiredpipelinefields_with_several_rows_for_one_field_the_last_one_decides_like_the_action()
 {
    let filled_then_blank = fc::find_missing_required_pipeline_fields(
        &json!([
            { "pipelineFieldId": "1", "value": "500" },
            { "pipelineFieldId": "1", "value": "" },
        ]),
        &fields(),
    );
    let blank_then_filled = fc::find_missing_required_pipeline_fields(
        &json!([
            { "pipelineFieldId": "1", "value": "" },
            { "pipelineFieldId": "1", "value": "500" },
        ]),
        &fields(),
    );

    assert!(key(&filled_then_blank, "id").contains(&"1"));
    assert!(!key(&blank_then_filled, "id").contains(&"1"));
}
