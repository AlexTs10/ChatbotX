//! Port of `packages/flow-config/src/steps/spreadsheet.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn operator() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"IS":"is","IS_NOT":"is_not","GTE":"gte","LTE":"lte","GT":"gt","LT":"lt","CONTAINS":"contains","NOT_CONTAINS":"not_contains","STARTS_WITH":"starts_with","ENDS_WITH":"ends_with"}"#).expect("valid JSON")
    });
    &V
}

pub fn filter_mode() -> &'static Value {
    static V: LazyLock<Value> =
        LazyLock::new(|| serde_json::from_str(r#"{"AND":"AND","OR":"OR"}"#).expect("valid JSON"));
    &V
}

schema!(pub fn spreadsheet_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::union(vec![z::literal(json!("spreadsheetGetRandomRow")),
z::literal(json!("spreadsheetGetRow")),
z::literal(json!("spreadsheetClearRow")),
z::literal(json!("spreadsheetSendData")),
z::literal(json!("spreadsheetUpdateRow"))])),
("spreadsheetId", z::string().regex(r#"^\d+$"#, "")),
("sheetName", z::string().min(1)),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]));

schema!(pub fn spreadsheet_step_versions = z::enum_(&["v1", "v2"]));

default_fn!(pub fn spreadsheet_default_fn = json!({"id": create_id(), "stepType": "spreadsheetGetRow", "spreadsheetId": "", "sheetName": "", "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));

schema!(pub fn spreadsheet_sheet_to_contact_mapping_schema = z::object(vec![("customFieldId", z::union(vec![z::string().regex(r#"^\d+$"#, ""),
z::string().refine(|v| is_bot_field_reference(v.as_str().unwrap_or("")), Some("Must be a valid bot field reference"), None),
z::literal(json!(""))]).describe("Id of the contact custom field (from `customFields.list`) that receives the sheet value.")),
("header", z::string().min(1).describe("Sheet column header whose value is read into the custom field."))]));

schema!(pub fn spreadsheet_contact_to_sheet_mapping_schema = z::object(vec![("header", z::string().min(1).describe("Sheet column header that receives the value.")),
("customFieldId", z::union(vec![z::string().regex(r#"^\d+$"#, ""),
z::string().refine(|v| is_bot_field_reference(v.as_str().unwrap_or("")), Some("Must be a valid bot field reference"), None),
z::literal(json!(""))]).optional().describe("Legacy v1 field. With `version: \"v2\"` leave it unset and use `value`; under v1 (the default when `version` is omitted) `value` is ignored.")),
("value", z::string().default(json!("")).describe("Value written to the column. May include `{{variable}}` placeholders such as contact fields."))]));

schema!(pub fn spreadsheet_column_filter_schema = z::object(vec![("mode", z::enum_(&["AND", "OR"]).describe("`AND` requires every condition to match a row; `OR` any one.")),
("conditions", z::array(z::object(vec![("column", z::string().describe("Sheet column header to test.")),
("operator", z::enum_(&["is", "is_not", "gte", "lte", "gt", "lt", "contains", "not_contains", "starts_with", "ends_with"]).describe("Comparison: is, is_not, gte, lte, gt, lt, contains, not_contains, starts_with or ends_with.")),
("value", z::string().describe("Value to compare the column against. May include `{{variable}}` placeholders."))])).describe("Row-matching conditions that select the sheet row."))]));

default_fn!(pub fn spreadsheet_column_filter_default_fn = json!({"mode": "AND", "conditions": []}));

/// `toSpreadsheetStepVersion`: `v2`, else `v1` (missing/unknown).
pub fn to_spreadsheet_step_version(version: Option<&str>) -> &'static str {
    match version {
        Some("v2") => "v2",
        _ => "v1",
    }
}

/// `spreadsheetSheetToContactMappingDefaultFn(header)`.
pub fn spreadsheet_sheet_to_contact_mapping_default_fn(header: &str) -> Value {
    json!({ "customFieldId": "", "header": header })
}

/// `spreadsheetContactToSheetMappingDefaultFn(header)`.
pub fn spreadsheet_contact_to_sheet_mapping_default_fn(header: &str) -> Value {
    json!({ "header": header, "value": "" })
}
