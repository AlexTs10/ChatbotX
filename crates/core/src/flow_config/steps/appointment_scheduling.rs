//! Port of `packages/flow-config/src/steps/appointment-scheduling.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn appointment_scheduling_modes = z::enum_(&["bookFromCustomField", "checkAvailabilityFromCustomField", "book", "checkAvailability", "cancel"]));

schema!(pub fn appointment_scheduling_step_schema = z::discriminated_union("mode", vec![z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("appointmentScheduling")).describe("Step type discriminator: \"appointmentScheduling\".")),
("calendarId", z::string().regex(r#"^\d+$"#, "")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()])),
("mode", z::literal(json!("bookFromCustomField"))),
("dateTimeFieldId", z::string().trim().min(1))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("appointmentScheduling")).describe("Step type discriminator: \"appointmentScheduling\".")),
("calendarId", z::string().regex(r#"^\d+$"#, "")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()])),
("mode", z::literal(json!("checkAvailabilityFromCustomField"))),
("startDateFieldId", z::string().trim().min(1)),
("endDateFieldId", z::string().trim().min(1)),
("resultUsedByAI", z::boolean().default(json!(false))),
("outputCustomFieldId", z::string().trim().min(1))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("appointmentScheduling")).describe("Step type discriminator: \"appointmentScheduling\".")),
("calendarId", z::string().regex(r#"^\d+$"#, "")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()])),
("mode", z::literal(json!("book")))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("appointmentScheduling")).describe("Step type discriminator: \"appointmentScheduling\".")),
("calendarId", z::string().regex(r#"^\d+$"#, "")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()])),
("mode", z::literal(json!("cancel")))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("appointmentScheduling")).describe("Step type discriminator: \"appointmentScheduling\".")),
("calendarId", z::string().regex(r#"^\d+$"#, "")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()])),
("mode", z::literal(json!("checkAvailability"))),
("resultUsedByAI", z::boolean().default(json!(false))),
("outputCustomFieldId", z::string().trim().min(1))])]));

default_fn!(pub fn appointment_scheduling_step_default_fn = json!({"id": create_id(), "stepType": "appointmentScheduling", "mode": "bookFromCustomField", "calendarId": "", "dateTimeFieldId": "", "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));
