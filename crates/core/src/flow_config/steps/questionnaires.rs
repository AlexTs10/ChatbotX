//! Port of `packages/flow-config/src/steps/questionnaires.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn questionnaire_action_modes = z::enum_(&["start", "end", "deleteApplicant"]));

schema!(pub fn questionnaires_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("questionnaires")).describe("Step type discriminator: \"questionnaires\".")),
("mode", questionnaire_action_modes().default(json!("start"))),
("questionnaireId", z::string().regex(r#"^\d+$"#, "")),
("states", z::preprocess(questionnaires_step_schema_states_preprocess, z::undefined()).optional())]));

default_fn!(pub fn questionnaires_step_default_fn = json!({"id": create_id(), "stepType": "questionnaires", "mode": "start", "questionnaireId": ""}));

fn questionnaires_step_schema_states_preprocess(_value: Option<Value>) -> Option<Value> {
    None
}
