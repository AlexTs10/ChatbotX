//! Port of `packages/flow-config/src/steps/spreadsheet-update-row.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn spreadsheet_update_row_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("spreadsheetUpdateRow")).describe("Step type discriminator: \"spreadsheetUpdateRow\".")),
("spreadsheetId", z::string().regex(r#"^\d+$"#, "")),
("sheetName", z::string().min(1)),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()])),
("version", spreadsheet_step_versions().catch_value(/* () => value */ json!("v1")).default(json!("v1")).describe("Mapping format. Omitted or invalid parses as legacy \"v1\", where `map[].value` is ignored; set \"v2\" to write `map[].value`.")),
("lookup", spreadsheet_column_filter_schema()),
("map", z::array(spreadsheet_contact_to_sheet_mapping_schema()).min(1))]));

default_fn!(pub fn spreadsheet_update_row_default_fn = json!({"id": create_id(), "stepType": "spreadsheetUpdateRow", "spreadsheetId": "", "sheetName": "", "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}], "version": "v2", "lookup": {"mode": "AND", "conditions": []}, "map": []}));
