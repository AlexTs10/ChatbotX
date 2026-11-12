//! Port of `packages/flow-config/src/steps/spreadsheet-send-data.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn spreadsheet_send_data_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("spreadsheetSendData")).describe("Step type discriminator: \"spreadsheetSendData\".")),
("spreadsheetId", z::string().regex(r#"^\d+$"#, "")),
("sheetName", z::string().min(1)),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()])),
("version", spreadsheet_step_versions().catch_value(/* () => value */ json!("v1")).default(json!("v1")).describe("Mapping format. Omitted or invalid parses as legacy \"v1\", where `map[].value` is ignored; set \"v2\" to write `map[].value`.")),
("map", z::array(spreadsheet_contact_to_sheet_mapping_schema()).min(1))]));

default_fn!(pub fn spreadsheet_send_data_default_fn = json!({"id": create_id(), "stepType": "spreadsheetSendData", "spreadsheetId": "", "sheetName": "", "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}], "version": "v2", "map": []}));
