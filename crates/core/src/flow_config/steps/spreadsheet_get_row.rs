//! Port of `packages/flow-config/src/steps/spreadsheet-get-row.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn spreadsheet_get_row_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("spreadsheetGetRow")).describe("Step type discriminator: \"spreadsheetGetRow\".")),
("spreadsheetId", z::string().regex(r#"^\d+$"#, "")),
("sheetName", z::string().min(1)),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()])),
("lookup", spreadsheet_column_filter_schema()),
("map", z::array(spreadsheet_sheet_to_contact_mapping_schema()).min(1))]));

default_fn!(pub fn spreadsheet_get_row_default_fn = json!({"id": create_id(), "stepType": "spreadsheetGetRow", "spreadsheetId": "", "sheetName": "", "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}], "lookup": {"mode": "AND", "conditions": []}, "map": []}));
