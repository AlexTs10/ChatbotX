//! Port of `packages/flow-config/src/steps/pipeline.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn pipeline_step_limits() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(
            r#"{"title":200,"description":5000,"comment":2000,"assignees":20,"maxFieldIds":50}"#,
        )
        .expect("valid JSON")
    });
    &V
}

schema!(pub fn pipeline_step_actions = z::enum_(&["addToPipeline", "moveToStage", "changeValue", "assign", "changeStatus", "changePriority", "changeDueDate", "changeTitle", "setCustomField", "clearCustomField", "changeDescription", "removeFromPipeline", "addComment", "transfer"]));

schema!(pub fn pipeline_step_due_date_schema = z::discriminated_union("mode", vec![z::object(vec![("mode", z::literal(json!("none")))]),
z::object(vec![("mode", z::literal(json!("customField"))),
("customFieldId", z::union(vec![z::literal(json!("")),
z::string().regex(r#"^\d+$"#, "").max(18)]).describe("Contact custom field (date or datetime) holding the due date. Get ids from `contacts.listCustomFields`."))]),
z::object(vec![("mode", z::literal(json!("fixed"))),
("at", z::string().iso_datetime(true, false, None).describe("Fixed ISO-8601 instant."))])]));

schema!(pub fn pipeline_step_schema = z::discriminated_union("action", vec![z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("pipeline")).describe("Step type discriminator: \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema(),
skip_state_schema()])),
("action", z::literal(json!("addToPipeline"))),
("title", z::string().max(200).describe("Item title; accepts {{variables}}.")),
("description", z::string().max(5000)),
("assigneeMemberIds", z::array(z::string().regex(r#"^\d+$"#, "").max(18)).max(20).describe("Workspace member ids.")),
("assigneeInboxTeamIds", z::array(z::string().regex(r#"^\d+$"#, "").max(18)).max(20).describe("Inbox team ids.")),
("status", z::enum_(&["open", "won", "lost"])),
("stageId", z::union(vec![z::literal(json!("")),
z::string().regex(r#"^\d+$"#, "").max(18)]).optional().describe("Stage id; omitted means the first stage.")),
("dueDate", pipeline_step_due_date_schema()),
("fieldValues", z::array(z::object(vec![("pipelineFieldId", z::string().regex(r#"^\d+$"#, "").max(18).describe("Pipeline custom field id. Get ids from `pipelines.get`.")),
("value", z::string().describe("Value; accepts {{variables}}."))])).max(50).describe("Pipeline custom field values ({{variables}} allowed)."))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("pipeline")).describe("Step type discriminator: \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema(),
skip_state_schema()])),
("action", z::literal(json!("moveToStage"))),
("stageId", z::string().regex(r#"^\d+$"#, "").msg("Select a stage").max(18))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("pipeline")).describe("Step type discriminator: \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema(),
skip_state_schema()])),
("action", z::literal(json!("changeValue"))),
("value", z::string().describe("Opportunity value as a number >= 0; accepts {{variables}}."))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("pipeline")).describe("Step type discriminator: \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema(),
skip_state_schema()])),
("action", z::literal(json!("assign"))),
("assigneeMemberIds", z::array(z::string().regex(r#"^\d+$"#, "").max(18)).max(20).describe("Workspace member ids.")),
("assigneeInboxTeamIds", z::array(z::string().regex(r#"^\d+$"#, "").max(18)).max(20).describe("Inbox team ids."))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("pipeline")).describe("Step type discriminator: \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema(),
skip_state_schema()])),
("action", z::literal(json!("changeStatus"))),
("status", z::enum_(&["open", "won", "lost"]))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("pipeline")).describe("Step type discriminator: \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema(),
skip_state_schema()])),
("action", z::literal(json!("changePriority"))),
("priority", z::enum_(&["low", "medium", "high"]))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("pipeline")).describe("Step type discriminator: \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema(),
skip_state_schema()])),
("action", z::literal(json!("changeDueDate"))),
("dueDate", pipeline_step_due_date_schema())]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("pipeline")).describe("Step type discriminator: \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema(),
skip_state_schema()])),
("action", z::literal(json!("changeTitle"))),
("title", z::string().max(200))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("pipeline")).describe("Step type discriminator: \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema(),
skip_state_schema()])),
("action", z::literal(json!("setCustomField"))),
("pipelineFieldId", z::union(vec![z::literal(json!("")),
z::string().regex(r#"^\d+$"#, "").max(18)]).describe("Pipeline custom field id.")),
("value", z::string())]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("pipeline")).describe("Step type discriminator: \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema(),
skip_state_schema()])),
("action", z::literal(json!("clearCustomField"))),
("pipelineFieldIds", z::array(z::string().regex(r#"^\d+$"#, "").max(18)).max(50).describe("Pipeline custom field ids to clear."))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("pipeline")).describe("Step type discriminator: \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema(),
skip_state_schema()])),
("action", z::literal(json!("changeDescription"))),
("description", z::string().max(5000))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("pipeline")).describe("Step type discriminator: \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema(),
skip_state_schema()])),
("action", z::literal(json!("removeFromPipeline")))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("pipeline")).describe("Step type discriminator: \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema(),
skip_state_schema()])),
("action", z::literal(json!("addComment"))),
("text", z::string().max(2000))]),
z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("pipeline")).describe("Step type discriminator: \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("states", z::tuple(vec![success_state_schema(),
error_state_schema(),
skip_state_schema()])),
("action", z::literal(json!("transfer"))),
("destinationPipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18)),
("destinationStageId", z::union(vec![z::literal(json!("")),
z::string().regex(r#"^\d+$"#, "").max(18)]).optional().describe("Destination stage id; omitted means the first stage."))])]));

schema!(pub fn pipeline_action_schema = z::discriminated_union("action", vec![z::object(vec![("type", z::literal(json!("pipeline")).describe("Action type \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("action", z::literal(json!("addToPipeline"))),
("title", z::string().max(200).describe("Item title; accepts {{variables}}.")),
("description", z::string().max(5000)),
("assigneeMemberIds", z::array(z::string().regex(r#"^\d+$"#, "").max(18)).max(20).describe("Workspace member ids.")),
("assigneeInboxTeamIds", z::array(z::string().regex(r#"^\d+$"#, "").max(18)).max(20).describe("Inbox team ids.")),
("status", z::enum_(&["open", "won", "lost"])),
("stageId", z::union(vec![z::literal(json!("")),
z::string().regex(r#"^\d+$"#, "").max(18)]).optional().describe("Stage id; omitted means the first stage.")),
("dueDate", pipeline_step_due_date_schema()),
("fieldValues", z::array(z::object(vec![("pipelineFieldId", z::string().regex(r#"^\d+$"#, "").max(18).describe("Pipeline custom field id. Get ids from `pipelines.get`.")),
("value", z::string().describe("Value; accepts {{variables}}."))])).max(50).describe("Pipeline custom field values ({{variables}} allowed)."))]),
z::object(vec![("type", z::literal(json!("pipeline")).describe("Action type \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("action", z::literal(json!("moveToStage"))),
("stageId", z::string().regex(r#"^\d+$"#, "").msg("Select a stage").max(18))]),
z::object(vec![("type", z::literal(json!("pipeline")).describe("Action type \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("action", z::literal(json!("changeValue"))),
("value", z::string().describe("Opportunity value as a number >= 0; accepts {{variables}}."))]),
z::object(vec![("type", z::literal(json!("pipeline")).describe("Action type \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("action", z::literal(json!("assign"))),
("assigneeMemberIds", z::array(z::string().regex(r#"^\d+$"#, "").max(18)).max(20).describe("Workspace member ids.")),
("assigneeInboxTeamIds", z::array(z::string().regex(r#"^\d+$"#, "").max(18)).max(20).describe("Inbox team ids."))]),
z::object(vec![("type", z::literal(json!("pipeline")).describe("Action type \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("action", z::literal(json!("changeStatus"))),
("status", z::enum_(&["open", "won", "lost"]))]),
z::object(vec![("type", z::literal(json!("pipeline")).describe("Action type \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("action", z::literal(json!("changePriority"))),
("priority", z::enum_(&["low", "medium", "high"]))]),
z::object(vec![("type", z::literal(json!("pipeline")).describe("Action type \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("action", z::literal(json!("changeDueDate"))),
("dueDate", pipeline_step_due_date_schema())]),
z::object(vec![("type", z::literal(json!("pipeline")).describe("Action type \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("action", z::literal(json!("changeTitle"))),
("title", z::string().max(200))]),
z::object(vec![("type", z::literal(json!("pipeline")).describe("Action type \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("action", z::literal(json!("setCustomField"))),
("pipelineFieldId", z::union(vec![z::literal(json!("")),
z::string().regex(r#"^\d+$"#, "").max(18)]).describe("Pipeline custom field id.")),
("value", z::string())]),
z::object(vec![("type", z::literal(json!("pipeline")).describe("Action type \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("action", z::literal(json!("clearCustomField"))),
("pipelineFieldIds", z::array(z::string().regex(r#"^\d+$"#, "").max(18)).max(50).describe("Pipeline custom field ids to clear."))]),
z::object(vec![("type", z::literal(json!("pipeline")).describe("Action type \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("action", z::literal(json!("changeDescription"))),
("description", z::string().max(5000))]),
z::object(vec![("type", z::literal(json!("pipeline")).describe("Action type \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("action", z::literal(json!("removeFromPipeline")))]),
z::object(vec![("type", z::literal(json!("pipeline")).describe("Action type \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("action", z::literal(json!("addComment"))),
("text", z::string().max(2000))]),
z::object(vec![("type", z::literal(json!("pipeline")).describe("Action type \"pipeline\".")),
("pipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18).describe("Pipeline the action applies to. Get ids from `pipelines.list`.")),
("action", z::literal(json!("transfer"))),
("destinationPipelineId", z::string().regex(r#"^\d+$"#, "").msg("Select a pipeline").max(18)),
("destinationStageId", z::union(vec![z::literal(json!("")),
z::string().regex(r#"^\d+$"#, "").max(18)]).optional().describe("Destination stage id; omitted means the first stage."))])]));

/// `pipelineActionCanSkip`: every action but `addToPipeline` can skip.
pub fn pipeline_action_can_skip(action: &str) -> bool {
    action != "addToPipeline"
}

fn default_states() -> Value {
    json!([
        success_state_default_fn(),
        error_state_default_fn(),
        skip_state_default_fn()
    ])
}

/// `pipelineStepDefaultFn(action, carry)`.
pub fn pipeline_step_default_fn(action: &str, carry: &Value) -> Value {
    let pick = |key: &str| carry.get(key).filter(|v| !v.is_null()).cloned();
    let base = json!({
        "id": pick("id").unwrap_or_else(|| json!(create_id())),
        "stepType": "pipeline",
        "pipelineId": pick("pipelineId").unwrap_or_else(|| json!("")),
        "states": pick("states").unwrap_or_else(default_states),
        "action": action,
    });
    let none = json!({ "mode": "none" });
    let fields = match action {
        "addToPipeline" => json!({
            "title": "{{full_name}}",
            "description": "",
            "assigneeMemberIds": [],
            "assigneeInboxTeamIds": [],
            "status": "open",
            "dueDate": none,
            "fieldValues": [],
        }),
        "moveToStage" => json!({ "stageId": "" }),
        "changeValue" => json!({ "value": "" }),
        "assign" => json!({ "assigneeMemberIds": [], "assigneeInboxTeamIds": [] }),
        "changeStatus" => json!({ "status": "won" }),
        "changePriority" => json!({ "priority": "medium" }),
        "changeDueDate" => json!({ "dueDate": none }),
        "changeTitle" => json!({ "title": "" }),
        "setCustomField" => json!({ "pipelineFieldId": "", "value": "" }),
        "clearCustomField" => json!({ "pipelineFieldIds": [] }),
        "changeDescription" => json!({ "description": "" }),
        "removeFromPipeline" => json!({}),
        "addComment" => json!({ "text": "" }),
        "transfer" => json!({ "destinationPipelineId": "" }),
        other => panic!("Unhandled pipeline action: {other}"),
    };
    merge(base, fields)
}

/// `pipelineActionDefaultFn(action, carry)`: the step defaults without the
/// step envelope, as a trigger action.
pub fn pipeline_action_default_fn(action: &str, carry: &Value) -> Value {
    let mut fields = pipeline_step_default_fn(action, carry);
    if let Some(obj) = fields.as_object_mut() {
        obj.remove("id");
        obj.remove("stepType");
        obj.remove("states");
    }
    merge(fields, json!({ "type": "pipeline" }))
}

/// `findMissingRequiredPipelineFields(fieldValues, definitions)`: the last
/// row of a field wins; blank counts as missing, a variable as set.
pub fn find_missing_required_pipeline_fields(
    field_values: &Value,
    definitions: &Value,
) -> Vec<Value> {
    let mut final_values = std::collections::HashMap::new();
    for row in field_values.as_array().into_iter().flatten() {
        final_values.insert(
            str_field(row, "pipelineFieldId").to_owned(),
            str_field(row, "value").to_owned(),
        );
    }
    definitions
        .as_array()
        .into_iter()
        .flatten()
        .filter(|definition| {
            definition["required"] == true
                && crate::js::trim(
                    final_values
                        .get(str_field(definition, "id"))
                        .map_or("", String::as_str),
                )
                .is_empty()
        })
        .cloned()
        .collect()
}
