//! Port of `packages/flow-config/__tests__/spreadsheet-write-mapping.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn lookup(custom_field_id: &str) -> Option<Value> {
    match custom_field_id {
        "cf-name" => Some(json!({ "name": "Full Name", "type": "shortText" })),
        "cf-date" => Some(json!({ "name": "Ngày sinh", "type": "date" })),
        "cf-bad" => Some(json!({ "name": "Bad\nName", "type": "shortText" })),
        _ => None,
    }
}

#[test]
fn spreadsheet_write_mapping_versions_normalizes_missing_and_unknown_versions_to_v1() {
    assert_eq!(fc::to_spreadsheet_step_version(None), "v1");
    assert_eq!(fc::to_spreadsheet_step_version(Some("v9")), "v1");
    assert_eq!(fc::to_spreadsheet_step_version(Some("v2")), "v2");
}

#[test]
fn spreadsheet_write_mapping_versions_defaults_new_write_steps_to_v2_while_parsing_legacy_json_as_v1()
 {
    assert_eq!(fc::spreadsheet_send_data_default_fn()["version"], "v2");
    assert_eq!(fc::spreadsheet_update_row_default_fn()["version"], "v2");
    let legacy = without(
        &spread(
            &fc::spreadsheet_send_data_default_fn(),
            json!({
                "spreadsheetId": "11619011544072192",
                "sheetName": "Sheet1",
                "map": [{ "header": "Name", "customFieldId": "11619011544072193" }],
            }),
        ),
        &["version"],
    );
    assert_eq!(
        fc::spreadsheet_send_data_schema().parse(&legacy).unwrap()["version"],
        "v1"
    );
}

#[test]
fn spreadsheet_write_mapping_versions_accepts_contact_to_sheet_mappings_that_carry_an_empty_customfieldid()
 {
    // Legacy v1 write steps persisted `customFieldId: ""`; the schema must keep
    // validating them so opening an existing step does not error.
    let step = spread(
        &fc::spreadsheet_send_data_default_fn(),
        json!({
            "spreadsheetId": "11619011544072192",
            "sheetName": "Sheet1",
            "version": "v2",
            "map": [
                { "header": "Name", "value": "Ada", "customFieldId": "" },
                { "header": "Phone", "value": "{{Phone}}", "customFieldId": "" },
            ],
        }),
    );
    assert!(fc::spreadsheet_send_data_schema().parse(&step).is_ok());
}

#[test]
fn spreadsheet_write_mapping_versions_converts_legacy_mappings_to_raw_custom_field_tokens_all_or_nothing()
 {
    let step = json!({
        "stepType": "spreadsheetSendData",
        "map": [
            { "header": "Name", "customFieldId": "cf-name" },
            { "header": "Birthday", "customFieldId": "cf-date" },
        ],
    });

    assert_eq!(
        fc::upgrade_spreadsheet_write_step(&step, &lookup),
        spread(
            &step,
            json!({
                "version": "v2",
                "map": [
                    { "header": "Name", "customFieldId": "cf-name", "value": "{{raw:Full Name}}" },
                    { "header": "Birthday", "customFieldId": "cf-date", "value": "{{raw:Ngày sinh}}" },
                ],
            }),
        )
    );
}

#[test]
fn spreadsheet_write_mapping_versions_keeps_v1_when_any_mapping_cannot_be_converted() {
    let deleted_field_step = json!({
        "stepType": "spreadsheetSendData",
        "map": [
            { "header": "Name", "customFieldId": "cf-name" },
            { "header": "Missing", "customFieldId": "cf-missing" },
        ],
    });
    let invalid_name_step = json!({
        "stepType": "spreadsheetSendData",
        "map": [{ "header": "Bad", "customFieldId": "cf-bad" }],
    });

    assert_eq!(
        fc::upgrade_spreadsheet_write_step(&deleted_field_step, &lookup),
        deleted_field_step
    );
    assert_eq!(
        fc::upgrade_spreadsheet_write_step(&invalid_name_step, &lookup),
        invalid_name_step
    );
}

#[test]
fn spreadsheet_write_mapping_versions_is_idempotent_for_v2_and_leaves_empty_maps_unchanged() {
    let v2_step = json!({
        "stepType": "spreadsheetUpdateRow",
        "version": "v2",
        "map": [{ "header": "Name", "value": "{{raw:Full Name}}" }],
    });
    let empty_map_step = json!({ "stepType": "spreadsheetUpdateRow", "map": [] });

    assert_eq!(
        fc::upgrade_spreadsheet_write_step(&v2_step, &lookup),
        v2_step
    );
    assert_eq!(
        fc::upgrade_spreadsheet_write_step(&empty_map_step, &lookup),
        empty_map_step
    );
}

#[test]
fn spreadsheet_write_mapping_versions_tags_write_step_versions_without_converting_mappings() {
    let step = json!({ "stepType": "spreadsheetSendData", "map": [{ "header": "Name", "customFieldId": "cf-name" }] });

    assert_eq!(
        fc::tag_spreadsheet_write_step_version(&step),
        spread(&step, json!({ "version": "v1" }))
    );
    assert_eq!(
        fc::tag_spreadsheet_write_step_version(&spread(&step, json!({ "version": "v2" }))),
        spread(&step, json!({ "version": "v2" }))
    );
}

#[test]
fn spreadsheet_write_mapping_versions_walks_top_level_button_carousel_and_email_nested_steps() {
    let details = json!({
        "beforeStep": { "stepType": "chooseChannel" },
        "steps": [
            {
                "stepType": "sendCarousel",
                "cards": [{
                    "buttons": [{
                        "buttonType": "performAction",
                        "beforeStep": null,
                        "steps": [{ "stepType": "spreadsheetSendData", "map": [{ "header": "Name", "customFieldId": "cf-name" }] }],
                    }],
                }],
            },
            {
                "stepType": "email",
                "elements": [{
                    "type": "button",
                    "buttonType": "performAction",
                    "beforeStep": null,
                    "steps": [{ "stepType": "spreadsheetUpdateRow", "map": [{ "header": "Birthday", "customFieldId": "cf-date" }] }],
                }],
            },
        ],
        "quickReplies": [{
            "buttonType": "performAction",
            "beforeStep": null,
            "steps": [{ "stepType": "spreadsheetSendData", "map": [{ "header": "Name", "customFieldId": "cf-name" }] }],
        }],
    });

    let upgraded = serde_json::to_string(&fc::upgrade_node_steps(&details, &lookup)).unwrap();
    assert!(upgraded.contains("{{raw:Full Name}}"));
    assert!(upgraded.contains("{{raw:Ngày sinh}}"));
    let tagged =
        serde_json::to_string(&fc::tag_node_spreadsheet_write_step_versions(&details)).unwrap();
    assert!(tagged.contains("\"version\":\"v1\""));
}

#[test]
fn spreadsheet_write_mapping_versions_walks_message_step_buttons_where_write_steps_also_nest() {
    let details = json!({
        "steps": [{
            "stepType": "sendText",
            "buttons": [{
                "buttonType": "performAction",
                "beforeStep": null,
                "steps": [{ "stepType": "spreadsheetSendData", "map": [{ "header": "Name", "customFieldId": "cf-name" }] }],
            }],
        }],
    });

    assert!(
        serde_json::to_string(&fc::upgrade_node_steps(&details, &lookup))
            .unwrap()
            .contains("{{raw:Full Name}}")
    );
    assert!(
        serde_json::to_string(&fc::tag_node_spreadsheet_write_step_versions(&details))
            .unwrap()
            .contains("\"version\":\"v1\"")
    );
}

#[test]
fn spreadsheet_write_mapping_versions_converts_an_unconfigured_legacy_row_to_an_empty_template() {
    let step = json!({
        "stepType": "spreadsheetSendData",
        "map": [
            { "header": "Name", "customFieldId": "cf-name" },
            { "header": "Unmapped", "customFieldId": "" },
        ],
    });

    assert_eq!(
        fc::upgrade_spreadsheet_write_step(&step, &lookup),
        spread(
            &step,
            json!({
                "version": "v2",
                "map": [
                    { "header": "Name", "customFieldId": "cf-name", "value": "{{raw:Full Name}}" },
                    { "header": "Unmapped", "customFieldId": "", "value": "" },
                ],
            }),
        )
    );
}

#[test]
fn spreadsheet_write_mapping_versions_builds_raw_custom_field_tokens() {
    assert_eq!(
        fc::to_custom_field_token(&json!({ "name": "Full Name" })),
        "{{raw:Full Name}}"
    );
}
