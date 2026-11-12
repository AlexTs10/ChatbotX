//! Port of `packages/flow-config/src/steps/spreadsheet-clear-row.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn spreadsheet_clear_row_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("spreadsheetClearRow")).describe("Step type discriminator: \"spreadsheetClearRow\".")),
("spreadsheetId", z::string().regex(r#"^\d+$"#, "")),
("sheetName", z::string().min(1)),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()])),
("lookup", spreadsheet_column_filter_schema())]));

default_fn!(pub fn spreadsheet_clear_row_default_fn = json!({"id": create_id(), "stepType": "spreadsheetClearRow", "spreadsheetId": "", "sheetName": "", "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}], "lookup": {"mode": "AND", "conditions": []}}));
