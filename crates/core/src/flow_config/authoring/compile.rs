//! Port of `packages/flow-config/src/authoring/compile.ts`.

use crate::flow_config::prelude::*;
use crate::utils::custom_field::CustomFieldType;
use crate::utils::custom_field_operation::{
    FieldOperationType, field_operation_by_api_name, field_operation_definition,
    field_operations_for_type, is_field_operation_allowed,
};
use crate::utils::datetime::is_temporal_custom_field_type;

use std::collections::BTreeMap;

/// A template the compiler resolves by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringTemplate {
    pub id: String,
    pub language: String,
    pub status: String,
}

/// A custom field the compiler resolves by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringCustomField {
    pub id: String,
    pub field_type: String,
}

/// A flow the compiler resolves by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringFlowRef {
    pub id: String,
}

/// A pipeline stage (`FlowAuthoringPipeline.stages[]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineStageRef {
    pub id: String,
    pub name: String,
}

/// A pipeline field (`FlowAuthoringPipeline.fields[]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineFieldRef {
    pub id: String,
    pub name: String,
    pub reference_name: String,
}

/// `FlowAuthoringPipeline`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowAuthoringPipeline {
    pub id: String,
    pub name: String,
    pub reference_name: String,
    pub stages: Vec<PipelineStageRef>,
    pub fields: Vec<PipelineFieldRef>,
}

/// `FlowAuthoringContext`: reference data the compiler resolves DSL names against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowAuthoringContext {
    pub templates_by_name: BTreeMap<String, AuthoringTemplate>,
    pub custom_fields_by_name: BTreeMap<String, AuthoringCustomField>,
    pub flows_by_name: BTreeMap<String, AuthoringFlowRef>,
    /// Keyed by pipeline `referenceName`. `None` means the workspace has none.
    pub pipelines_by_reference_name: Option<BTreeMap<String, FlowAuthoringPipeline>>,
}

/// `CompiledFlow`.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledFlow {
    pub start_node_id: String,
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
    /// Compiled node id -> the spec-relative path (e.g. `steps[2]`) that produced it.
    pub spec_path_by_node_id: BTreeMap<String, String>,
}

/// Whether a step ends its step list — nothing may follow it.
fn ends_step_list(step_type: &str) -> bool {
    matches!(step_type, "branch" | "goto")
}

struct CompileState<'c> {
    nodes: Vec<Value>,
    edges: Vec<Value>,
    route_updates: Vec<FlowRouteUpdate>,
    step_id_to_node_id: BTreeMap<String, String>,
    spec_path_by_node_id: BTreeMap<String, String>,
    errors: Vec<FlowAuthoringError>,
    ctx: &'c FlowAuthoringContext,
    channel: Option<String>,
}

struct ResolveByName {
    code: &'static str,
    label: &'static str,
    hint: &'static str,
}

const TEMPLATE_NAME_RESOLUTION: ResolveByName = ResolveByName {
    code: "unknownTemplate",
    label: "WhatsApp template",
    hint: "Call capabilities.get and pick a name from its templates list.",
};

const FLOW_NAME_RESOLUTION: ResolveByName = ResolveByName {
    code: "unknownFlow",
    label: "flow",
    hint: "Call flows.list and pick a name from the results.",
};

const CUSTOM_FIELD_NAME_RESOLUTION: ResolveByName = ResolveByName {
    code: "unknownCustomField",
    label: "custom field",
    hint: "Call contacts.listFilterFields and pick a custom field name from the results.",
};

const PIPELINE_RESOLUTION: ResolveByName = ResolveByName {
    code: "unknownPipeline",
    label: "pipeline",
    hint: "Call pipelines.list and use a pipeline's referenceName (or id).",
};

/// The WhatsApp template status a template must have to be sent.
const APPROVED_TEMPLATE_STATUS: &str = "APPROVED";

const BOT_FIELD_CONDITION_PREFIX: &str = "botField:";
const CUSTOM_FIELD_CONDITION_PREFIX: &str = "customField:";

/// The context reference, detached from the `&mut` state borrow.
fn state_ctx<'c>(state: &CompileState<'c>) -> &'c FlowAuthoringContext {
    state.ctx
}

fn opt_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// A non-empty string field (`if (step.id)`).
fn truthy_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    opt_str(value, key).filter(|s| !s.is_empty())
}

fn steps_of<'a>(value: &'a Value, key: &str) -> Option<&'a [Value]> {
    value.get(key).and_then(Value::as_array).map(Vec::as_slice)
}

impl CompileState<'_> {
    fn add_error(
        &mut self,
        path: &str,
        code: &str,
        message: String,
        hint: Option<String>,
        candidates: Option<Vec<String>>,
    ) {
        self.errors.push(FlowAuthoringError {
            capability: None,
            path: path.to_string(),
            code: code.to_string(),
            message,
            hint,
            candidates,
        });
    }

    fn resolve_by_name<'m, T>(
        &mut self,
        map: &'m BTreeMap<String, T>,
        name: &str,
        path: &str,
        options: &ResolveByName,
    ) -> Option<&'m T> {
        if let Some(value) = map.get(name) {
            return Some(value);
        }
        let names: Vec<&str> = map.keys().map(String::as_str).collect();
        self.add_error(
            path,
            options.code,
            format!("No {} named \"{name}\" in this workspace.", options.label),
            Some(options.hint.to_string()),
            Some(closest_names(name, &names)),
        );
        None
    }

    /// Makes `spec_step_id` resolvable by `goto` before this step's node exists.
    fn preregister_step_id(&mut self, spec_step_id: Option<&str>, node_id: &str) {
        if let Some(id) = spec_step_id.filter(|id| !id.is_empty()) {
            self.step_id_to_node_id
                .insert(id.to_string(), node_id.to_string());
        }
    }

    fn register_node(
        &mut self,
        spec_step_id: Option<&str>,
        node: Value,
        step_path: &str,
        insert_at: Option<usize>,
    ) -> String {
        let node_id = str_field(&node, "id").to_string();
        match insert_at {
            Some(index) => self.nodes.insert(index, node),
            None => self.nodes.push(node),
        }
        self.spec_path_by_node_id
            .insert(node_id.clone(), step_path.to_string());
        self.preregister_step_id(spec_step_id, &node_id);
        node_id
    }

    fn add_handle_edge(&mut self, source: &str, handle_id: &str, target: &str) {
        self.edges.push(json!({
            "id": create_id(),
            "source": source,
            "sourceHandle": handle_id,
            "target": target,
            "targetHandle": target,
        }));
    }

    fn add_continue_edge(&mut self, source: &str, target: &str) {
        self.add_handle_edge(source, source, target);
    }

    fn channel(&self) -> String {
        self.channel
            .clone()
            .unwrap_or_else(|| "omnichannel".to_string())
    }
}

/// Nested step lists owned by one step, with their path relative to that step.
fn child_chains(step: &Value) -> Vec<(&[Value], String)> {
    match str_field(step, "type") {
        "send" => arr_field(step, "buttons")
            .iter()
            .enumerate()
            .filter_map(|(index, button)| {
                steps_of(button, "then").map(|then| (then, format!(".buttons[{index}].then")))
            })
            .collect(),
        "branch" => {
            let mut chains: Vec<(&[Value], String)> = arr_field(step, "cases")
                .iter()
                .enumerate()
                .map(|(index, branch_case)| {
                    (
                        arr_field(branch_case, "then"),
                        format!(".cases[{index}].then"),
                    )
                })
                .collect();
            if let Some(otherwise) = steps_of(step, "otherwise") {
                chains.push((otherwise, ".otherwise".to_string()));
            }
            chains
        }
        "pipeline" => {
            let mut chains = Vec::new();
            if let Some(on_error) = steps_of(step, "onError") {
                chains.push((on_error, ".onError".to_string()));
            }
            if let Some(on_skip) = steps_of(step, "onSkip") {
                chains.push((on_skip, ".onSkip".to_string()));
            }
            chains
        }
        _ => Vec::new(),
    }
}

/// Every explicit `id` a spec declares, recursively.
fn explicit_step_ids(steps: &[Value], path_prefix: &str, out: &mut Vec<(String, String)>) {
    for (index, step) in steps.iter().enumerate() {
        let step_path = format!("{path_prefix}[{index}]");
        if str_field(step, "type") != "goto"
            && let Some(id) = truthy_str(step, "id")
        {
            out.push((id.to_string(), step_path.clone()));
        }
        for (child_steps, suffix) in child_chains(step) {
            explicit_step_ids(child_steps, &format!("{step_path}{suffix}"), out);
        }
    }
}

fn assert_no_duplicate_step_ids(spec: &Value, state: &mut CompileState<'_>) {
    let mut ids = Vec::new();
    explicit_step_ids(arr_field(spec, "steps"), "steps", &mut ids);
    // Insertion-ordered grouping (`Map<id, paths>`).
    let mut seen: Vec<(String, Vec<String>)> = Vec::new();
    for (id, path) in ids {
        match seen.iter_mut().find(|(existing, _)| *existing == id) {
            Some((_, paths)) => paths.push(path),
            None => seen.push((id, vec![path])),
        }
    }
    for (id, paths) in &seen {
        if paths.len() > 1 {
            for path in paths {
                state.add_error(
                    path,
                    "duplicateStepId",
                    format!(
                        "Step id \"{id}\" is used by {} steps; ids must be unique across the whole spec.",
                        paths.len()
                    ),
                    None,
                    None,
                );
            }
        }
    }
}

fn with_steps(node: Value, steps: Vec<Value>) -> Value {
    let mut node = node;
    node["data"]["details"]["steps"] = Value::Array(steps);
    node
}

fn choose_channel(channel: &str) -> Value {
    json!({ "beforeStep": choose_channel_step_default_fn(&json!({ "channel": channel })) })
}

// ---- Per-step-type node builders ----

/// Compiles one quick-reply button: its node chain (if any), route, and edge.
fn compile_send_button(
    button_spec: &Value,
    source_node_id: &str,
    button_path: &str,
    state: &mut CompileState<'_>,
) -> Value {
    let button = button_step_default_fn(
        &json!({ "label": button_spec.get("text").cloned().unwrap_or(Value::Null) }),
    );
    if let Some(call) = button_spec.get("call") {
        return spread_props(
            button,
            &json!({
                "buttonType": "callPhoneNumber",
                "beforeStep": spread_props(call_phone_number_step_default_fn(), &json!({ "phoneNumber": call })),
                "steps": [],
            }),
        );
    }
    if let Some(copy) = button_spec.get("copy") {
        return spread_props(
            button,
            &json!({
                "buttonType": "copyText",
                "beforeStep": spread_props(copy_text_step_default_fn(), &json!({ "text": copy })),
                "steps": [],
            }),
        );
    }
    let Some(then) = steps_of(button_spec, "then").filter(|then| !then.is_empty()) else {
        return button;
    };
    if let Some(entry_node_id) = compile_chain(then, button_path, state) {
        let button_id = str_field(&button, "id").to_string();
        state.route_updates.push(FlowRouteUpdate {
            source_node_id: source_node_id.to_string(),
            handle_id: button_id.clone(),
            route: Some(entry_node_id.clone()),
        });
        state.add_handle_edge(source_node_id, &button_id, &entry_node_id);
    }
    button
}

fn compile_send_step(step: &Value, step_path: &str, state: &mut CompileState<'_>) -> String {
    let node_id = create_id();
    let insert_at = state.nodes.len();
    // A button's `then` can `goto` back to this same send step.
    state.preregister_step_id(opt_str(step, "id"), &node_id);
    let buttons: Vec<Value> = arr_field(step, "buttons")
        .iter()
        .enumerate()
        .map(|(index, button_spec)| {
            compile_send_button(
                button_spec,
                &node_id,
                &format!("{step_path}.buttons[{index}].then"),
                state,
            )
        })
        .collect();
    let content_step = if let Some(text) = truthy_str(step, "text") {
        spread_props(
            send_text_step_default_fn(&json!({ "text": text })),
            &json!({ "buttons": buttons }),
        )
    } else if let Some(image_url) = truthy_str(step, "imageUrl") {
        spread_props(
            send_image_step_default_fn(),
            &json!({ "url": image_url, "buttons": buttons }),
        )
    } else {
        spread_props(
            send_file_step_default_fn(),
            &json!({ "url": opt_str(step, "fileUrl").unwrap_or(""), "buttons": buttons }),
        )
    };
    let node = with_steps(
        send_message_node_default_fn(&json!({
            "nodeProps": { "id": node_id },
            "detailProps": choose_channel(&state.channel()),
        })),
        vec![content_step],
    );
    state.register_node(opt_str(step, "id"), node, step_path, Some(insert_at))
}

fn compile_send_template_step(
    step: &Value,
    step_path: &str,
    state: &mut CompileState<'_>,
) -> Option<String> {
    let template_name = str_field(step, "templateName");
    let path = format!("{step_path}.templateName");
    let template = state
        .resolve_by_name(
            &state_ctx(state).templates_by_name,
            template_name,
            &path,
            &TEMPLATE_NAME_RESOLUTION,
        )?
        .clone();
    if template.status != APPROVED_TEMPLATE_STATUS {
        state.add_error(
            &path,
            "templateNotApproved",
            format!("WhatsApp template \"{template_name}\" must have status APPROVED."),
            Some(
                "Pick a template whose status is APPROVED in capabilities.get's templates list."
                    .to_string(),
            ),
            None,
        );
        return None;
    }
    let template_step = send_wa_template_message_step_default_fn(&json!({
        "template": { "id": template.id, "name": template_name, "language": template.language, "params": {} },
    }));
    // A WA template step only ever sends over WhatsApp.
    let node = with_steps(
        send_message_node_default_fn(&json!({ "detailProps": choose_channel("whatsapp") })),
        vec![template_step],
    );
    Some(state.register_node(opt_str(step, "id"), node, step_path, None))
}

fn compile_send_products_step(
    step: &Value,
    step_path: &str,
    state: &mut CompileState<'_>,
) -> String {
    let mut content = step.as_object().cloned().unwrap_or_default();
    content.remove("id");
    content.remove("type");
    let sections = content.remove("sections").unwrap_or_else(|| json!([]));
    content.insert("sections".to_string(), sections);
    let node = with_steps(
        send_message_node_default_fn(&json!({ "detailProps": choose_channel(&state.channel()) })),
        vec![send_products_step_default_fn(&Value::Object(content))],
    );
    state.register_node(opt_str(step, "id"), node, step_path, None)
}

fn compile_send_view_catalog_step(
    step: &Value,
    step_path: &str,
    state: &mut CompileState<'_>,
) -> String {
    let node = with_steps(
        send_message_node_default_fn(&json!({ "detailProps": choose_channel(&state.channel()) })),
        vec![send_view_catalog_step_default_fn(&json!({
            "text": step.get("text").cloned().unwrap_or(Value::Null),
            "footer": opt_str(step, "footer").unwrap_or(""),
            "thumbnailProductId": opt_str(step, "thumbnailProductId").unwrap_or(""),
        }))],
    );
    state.register_node(opt_str(step, "id"), node, step_path, None)
}

fn compile_wait_step(step: &Value, step_path: &str, state: &mut CompileState<'_>) -> String {
    let wait_step = spread_props(
        wait_step_default_fn(),
        &json!({ "duration": step.get("duration").cloned().unwrap_or(Value::Null), "unit": step.get("unit").cloned().unwrap_or(Value::Null) }),
    );
    state.register_node(
        opt_str(step, "id"),
        with_steps(wait_node_default_fn(&json!({})), vec![wait_step]),
        step_path,
        None,
    )
}

/// The operation code a `setCustomField` action asks for (default `set`).
fn resolve_field_operation(
    step: &Value,
    custom_field: Option<&AuthoringCustomField>,
    step_path: &str,
    state: &mut CompileState<'_>,
) -> FieldOperationType {
    let Some(operation) = truthy_str(step, "operation").and_then(field_operation_by_api_name)
    else {
        return FieldOperationType::Set;
    };
    if let Some(custom_field) = custom_field {
        let kind = CustomFieldType::parse(&custom_field.field_type);
        if !kind.is_some_and(|kind| is_field_operation_allowed(operation, kind)) {
            let supported: Vec<&str> = kind
                .map(field_operations_for_type)
                .unwrap_or_default()
                .into_iter()
                .map(|code| field_operation_definition(code).api_name)
                .collect();
            state.add_error(
                &format!("{step_path}.operation"),
                "invalidFieldOperation",
                format!(
                    "Operation \"{}\" is not supported for a {} field.",
                    str_field(step, "operation"),
                    custom_field.field_type
                ),
                Some(format!("Supported: {}", supported.join(", "))),
                None,
            );
        }
    }
    operation
}

fn resolve_custom_field(
    name: &str,
    path: &str,
    state: &mut CompileState<'_>,
) -> Option<AuthoringCustomField> {
    state
        .resolve_by_name(
            &state_ctx(state).custom_fields_by_name,
            name,
            path,
            &CUSTOM_FIELD_NAME_RESOLUTION,
        )
        .cloned()
}

fn compile_action_step(step: &Value, step_path: &str, state: &mut CompileState<'_>) -> String {
    let tag_names = step.get("tagNames").cloned().unwrap_or_else(|| json!([]));
    let action_step = match str_field(step, "action") {
        "addTags" => add_contact_tag_step_default_fn(&json!({ "tags": tag_names })),
        "removeTags" => spread_props(
            remove_contact_tag_step_default_fn(),
            &json!({ "tags": tag_names }),
        ),
        "setCustomField" => {
            let custom_field = resolve_custom_field(
                opt_str(step, "customFieldName").unwrap_or(""),
                &format!("{step_path}.customFieldName"),
                state,
            );
            let operation = resolve_field_operation(step, custom_field.as_ref(), step_path, state);
            spread_props(
                set_custom_field_step_default_fn(),
                &json!({
                    "inputFieldId": custom_field.as_ref().map_or("", |f| f.id.as_str()),
                    "operation": operation.as_str(),
                    "value": opt_str(step, "value").unwrap_or(""),
                }),
            )
        }
        "assignConversation" => assign_conversation_step_default_fn(
            &json!({ "assignedId": opt_str(step, "assigneeId").unwrap_or("") }),
        ),
        _ => archive_conversation_step_default_fn(&json!({})),
    };
    state.register_node(
        opt_str(step, "id"),
        with_steps(
            perform_action_node_default_fn(&json!({})),
            vec![action_step],
        ),
        step_path,
        None,
    )
}

/// Resolves a pipeline by `referenceName`, falling back to its id.
fn resolve_pipeline(
    reference: &str,
    path: &str,
    state: &mut CompileState<'_>,
) -> Option<FlowAuthoringPipeline> {
    let empty = BTreeMap::new();
    let pipelines = state
        .ctx
        .pipelines_by_reference_name
        .as_ref()
        .unwrap_or(&empty);
    if let Some(pipeline) = pipelines.get(reference) {
        return Some(pipeline.clone());
    }
    if let Some(pipeline) = pipelines.values().find(|p| p.id == reference) {
        return Some(pipeline.clone());
    }
    state
        .resolve_by_name(pipelines, reference, path, &PIPELINE_RESOLUTION)
        .cloned()
}

fn resolve_pipeline_stage(
    pipeline: &FlowAuthoringPipeline,
    name: &str,
    path: &str,
    state: &mut CompileState<'_>,
) -> Option<String> {
    let matches: Vec<&PipelineStageRef> = pipeline
        .stages
        .iter()
        .filter(|stage| stage.name == name)
        .collect();
    if matches.len() == 1 {
        return Some(matches[0].id.clone());
    }
    if matches.len() > 1 {
        state.add_error(
            path,
            "ambiguousPipelineStage",
            format!(
                "Pipeline \"{}\" has {} stages named \"{name}\".",
                pipeline.reference_name,
                matches.len()
            ),
            Some("Rename one of the stages, or pass the stage id instead.".to_string()),
            None,
        );
        return None;
    }
    if let Some(stage) = pipeline.stages.iter().find(|stage| stage.id == name) {
        return Some(stage.id.clone());
    }
    let names: Vec<&str> = pipeline
        .stages
        .iter()
        .map(|stage| stage.name.as_str())
        .collect();
    state.add_error(
        path,
        "unknownPipelineStage",
        format!(
            "Pipeline \"{}\" has no stage named \"{name}\".",
            pipeline.reference_name
        ),
        Some("Call pipelines.get and pick a stage name from the pipeline.".to_string()),
        Some(closest_names(name, &names)),
    );
    None
}

fn resolve_pipeline_field(
    pipeline: &FlowAuthoringPipeline,
    reference: &str,
    path: &str,
    state: &mut CompileState<'_>,
) -> Option<String> {
    if let Some(field) = pipeline
        .fields
        .iter()
        .find(|f| f.reference_name == reference || f.id == reference)
    {
        return Some(field.id.clone());
    }
    let names: Vec<&str> = pipeline
        .fields
        .iter()
        .map(|f| f.reference_name.as_str())
        .collect();
    state.add_error(
        path,
        "unknownPipelineField",
        format!(
            "Pipeline \"{}\" has no custom field with referenceName \"{reference}\".",
            pipeline.reference_name
        ),
        Some("Call pipelines.get and use a field's referenceName.".to_string()),
        Some(closest_names(reference, &names)),
    );
    None
}

fn resolve_pipeline_due_date(
    due_date: Option<&Value>,
    path: &str,
    state: &mut CompileState<'_>,
) -> Value {
    let Some(due_date) = due_date.filter(|d| d["mode"] != "none") else {
        return json!({ "mode": "none" });
    };
    if due_date["mode"] == "fixed" {
        return due_date.clone();
    }
    let name = str_field(due_date, "customFieldName");
    let field_path = format!("{path}.customFieldName");
    let custom_field = resolve_custom_field(name, &field_path, state);
    if let Some(field) = &custom_field
        && !is_temporal_custom_field_type(&field.field_type)
    {
        state.add_error(
            &field_path,
            "invalidSpec",
            format!(
                "Custom field \"{name}\" is of type {}; a due date field must be date or datetime.",
                field.field_type
            ),
            None,
            None,
        );
    }
    json!({ "mode": "customField", "customFieldId": custom_field.map_or(String::new(), |f| f.id) })
}

/// Builds the compiled step for one action; reference misses are recorded on `state`.
fn build_pipeline_step(
    step: &Value,
    pipeline: &FlowAuthoringPipeline,
    step_path: &str,
    state: &mut CompileState<'_>,
) -> Value {
    let action = str_field(step, "action");
    let base = pipeline_step_default_fn(action, &json!({ "pipelineId": pipeline.id }));
    let or = |key: &str, fallback: Value| step.get(key).cloned().unwrap_or(fallback);
    let stage = |state: &mut CompileState<'_>,
                 name: Option<&str>,
                 target: &FlowAuthoringPipeline,
                 key: &str|
     -> Value {
        match name {
            None => Value::Null,
            Some(name) => json!(
                resolve_pipeline_stage(target, name, &format!("{step_path}.{key}"), state)
                    .unwrap_or_default()
            ),
        }
    };
    let set = |base: Value, patch: Value| -> Value {
        // `undefined` entries (Null here) are dropped like an absent spread key.
        let mut out = base;
        if let (Some(object), Value::Object(patch)) = (out.as_object_mut(), patch) {
            for (key, value) in patch {
                if value.is_null() {
                    object.remove(&key);
                } else {
                    object.insert(key, value);
                }
            }
        }
        out
    };
    match str_field(&base, "action") {
        "addToPipeline" => {
            let stage_id = stage(state, opt_str(step, "stage"), pipeline, "stage");
            let due_date = resolve_pipeline_due_date(
                step.get("dueDate"),
                &format!("{step_path}.dueDate"),
                state,
            );
            let field_values: Vec<Value> = arr_field(step, "fieldValues")
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let id = resolve_pipeline_field(
                        pipeline,
                        str_field(item, "field"),
                        &format!("{step_path}.fieldValues[{index}].field"),
                        state,
                    );
                    json!({ "pipelineFieldId": id.unwrap_or_default(), "value": item.get("value").cloned().unwrap_or(Value::Null) })
                })
                .collect();
            let title = or("title", base.get("title").cloned().unwrap_or(Value::Null));
            set(
                base,
                json!({
                    "title": title,
                    "description": or("description", json!("")),
                    "assigneeMemberIds": or("assigneeMemberIds", json!([])),
                    "assigneeInboxTeamIds": or("assigneeInboxTeamIds", json!([])),
                    "status": or("status", json!("open")),
                    "stageId": stage_id,
                    "dueDate": due_date,
                    "fieldValues": field_values,
                }),
            )
        }
        "moveToStage" => {
            let stage_id = stage(state, opt_str(step, "stage"), pipeline, "stage");
            let stage_id = if stage_id.is_null() {
                json!("")
            } else {
                stage_id
            };
            set(base, json!({ "stageId": stage_id }))
        }
        "changeValue" => set(base, json!({ "value": or("value", json!("")) })),
        "assign" => set(
            base,
            json!({
                "assigneeMemberIds": or("assigneeMemberIds", json!([])),
                "assigneeInboxTeamIds": or("assigneeInboxTeamIds", json!([])),
            }),
        ),
        "changeStatus" => {
            let status = or("status", base.get("status").cloned().unwrap_or(Value::Null));
            set(base, json!({ "status": status }))
        }
        "changePriority" => {
            let priority = or(
                "priority",
                base.get("priority").cloned().unwrap_or(Value::Null),
            );
            set(base, json!({ "priority": priority }))
        }
        "changeDueDate" => {
            let due_date = resolve_pipeline_due_date(
                step.get("dueDate"),
                &format!("{step_path}.dueDate"),
                state,
            );
            set(base, json!({ "dueDate": due_date }))
        }
        "changeTitle" => set(base, json!({ "title": or("title", json!("")) })),
        "setCustomField" => {
            let id = resolve_pipeline_field(
                pipeline,
                str_field(step, "field"),
                &format!("{step_path}.field"),
                state,
            );
            set(
                base,
                json!({ "pipelineFieldId": id.unwrap_or_default(), "value": or("value", json!("")) }),
            )
        }
        "clearCustomField" => {
            let ids: Vec<String> = arr_field(step, "fields")
                .iter()
                .enumerate()
                .map(|(index, reference)| {
                    resolve_pipeline_field(
                        pipeline,
                        reference.as_str().unwrap_or(""),
                        &format!("{step_path}.fields[{index}]"),
                        state,
                    )
                    .unwrap_or_default()
                })
                .collect();
            set(base, json!({ "pipelineFieldIds": ids }))
        }
        "changeDescription" => set(base, json!({ "description": or("description", json!("")) })),
        "addComment" => set(base, json!({ "text": or("text", json!("")) })),
        "transfer" => {
            let destination_path = format!("{step_path}.destinationPipeline");
            let destination = resolve_pipeline(
                opt_str(step, "destinationPipeline").unwrap_or(""),
                &destination_path,
                state,
            );
            // The runtime refuses a transfer into the same pipeline, however it is written.
            if destination.as_ref().is_some_and(|d| d.id == pipeline.id) {
                state.add_error(
                    &destination_path,
                    "invalidStep",
                    "transfer needs a destination pipeline different from \"pipeline\"."
                        .to_string(),
                    Some("Pick another pipeline from pipelines.list.".to_string()),
                    None,
                );
            }
            let stage_id = match &destination {
                Some(destination) => stage(
                    state,
                    opt_str(step, "destinationStage"),
                    destination,
                    "destinationStage",
                ),
                None => Value::Null,
            };
            set(
                base,
                json!({
                    "destinationPipelineId": destination.as_ref().map_or("", |d| d.id.as_str()),
                    "destinationStageId": stage_id,
                }),
            )
        }
        // removeFromPipeline
        _ => base,
    }
}

fn compile_pipeline_step(
    step: &Value,
    step_path: &str,
    state: &mut CompileState<'_>,
) -> Option<String> {
    let pipeline = resolve_pipeline(
        str_field(step, "pipeline"),
        &format!("{step_path}.pipeline"),
        state,
    );
    let node_id = create_id();
    let insert_at = state.nodes.len();
    // onError/onSkip can `goto` back to this step.
    state.preregister_step_id(opt_str(step, "id"), &node_id);

    let action = str_field(step, "action");
    if truthy(step.get("onSkip")) && !pipeline_action_can_skip(action) {
        state.add_error(
            &format!("{step_path}.onSkip"),
            "invalidStep",
            format!("action \"{action}\" never skips, so onSkip would never run."),
            None,
            None,
        );
    }

    let pipeline_step = pipeline
        .as_ref()
        .map(|pipeline| build_pipeline_step(step, pipeline, step_path, state));
    // Chains compile (and report their own errors) even when the pipeline failed to resolve.
    for (key, state_type) in [("onError", "error"), ("onSkip", "skip")] {
        let Some(steps) = steps_of(step, key).filter(|steps| !steps.is_empty()) else {
            continue;
        };
        let entry_node_id = compile_chain(steps, &format!("{step_path}.{key}"), state);
        let handle = pipeline_step.as_ref().and_then(|ps| {
            arr_field(ps, "states")
                .iter()
                .find(|item| item["stateType"] == state_type)
                .map(|item| str_field(item, "id").to_string())
        });
        if let (Some(entry), Some(handle)) = (entry_node_id, handle) {
            state.add_handle_edge(&node_id, &handle, &entry);
        }
    }
    let pipeline_step = pipeline_step?;
    let node = with_steps(
        perform_action_node_default_fn(&json!({ "nodeProps": { "id": node_id } })),
        vec![pipeline_step],
    );
    Some(state.register_node(opt_str(step, "id"), node, step_path, Some(insert_at)))
}

fn compile_start_flow_step(
    step: &Value,
    step_path: &str,
    state: &mut CompileState<'_>,
) -> Option<String> {
    let target = state
        .resolve_by_name(
            &state_ctx(state).flows_by_name,
            str_field(step, "flowName"),
            &format!("{step_path}.flowName"),
            &FLOW_NAME_RESOLUTION,
        )?
        .clone();
    let node = start_flow_node_default_fn(&json!({
        "detailProps": { "beforeStep": start_external_flow_step_default_fn(&json!({ "flowId": target.id })) },
    }));
    Some(state.register_node(opt_str(step, "id"), node, step_path, None))
}

fn compile_add_note_step(step: &Value, step_path: &str, state: &mut CompileState<'_>) -> String {
    let node = add_notes_node_default_fn(&json!({
        "detailProps": { "beforeStep": add_notes_step_default_fn(&json!({ "text": step.get("note").cloned().unwrap_or(Value::Null) })) },
    }));
    state.register_node(opt_str(step, "id"), node, step_path, None)
}

fn resolve_branch_condition(
    condition: &Value,
    path: &str,
    state: &mut CompileState<'_>,
) -> Option<Value> {
    let field = str_field(condition, "field");
    let mut compiled = Map::new();
    if field.starts_with(BOT_FIELD_CONDITION_PREFIX) {
        state.add_error(
            &format!("{path}.field"),
            "invalidSpec",
            "Bot field conditions are not supported by the flow-spec DSL yet — use a static field or 'customField:<name>'."
                .to_string(),
            None,
            None,
        );
        return None;
    }
    if let Some(name) = field.strip_prefix(CUSTOM_FIELD_CONDITION_PREFIX) {
        let custom_field = resolve_custom_field(name, &format!("{path}.field"), state)?;
        compiled.insert("field".into(), json!("customField"));
        compiled.insert("customFieldId".into(), json!(custom_field.id));
    } else {
        compiled.insert("field".into(), json!(field));
    }
    compiled.insert(
        "operator".into(),
        condition.get("operator").cloned().unwrap_or(Value::Null),
    );
    if let Some(value) = condition.get("value") {
        compiled.insert("value".into(), value.clone());
    }
    Some(Value::Object(compiled))
}

fn compile_branch_step(step: &Value, step_path: &str, state: &mut CompileState<'_>) -> String {
    let node_id = create_id();
    let insert_at = state.nodes.len();
    let otherwise_id = create_id();
    // A case's `then` can `goto` back to this same branch step.
    state.preregister_step_id(opt_str(step, "id"), &node_id);
    let mut cases = Vec::new();
    for (case_index, branch_case) in arr_field(step, "cases").iter().enumerate() {
        let case_default = condition_case_default_fn();
        let case_path = format!("{step_path}.cases[{case_index}]");
        let conditions: Vec<Value> = arr_field(branch_case, "when")
            .iter()
            .enumerate()
            .filter_map(|(index, condition)| {
                resolve_branch_condition(condition, &format!("{case_path}.when[{index}]"), state)
            })
            .collect();
        if let Some(entry) = compile_chain(
            arr_field(branch_case, "then"),
            &format!("{case_path}.then"),
            state,
        ) {
            state.add_handle_edge(&node_id, str_field(&case_default, "id"), &entry);
        }
        cases.push(spread_props(
            case_default,
            &json!({
                "operator": opt_str(branch_case, "match").unwrap_or("and"),
                "conditions": conditions,
            }),
        ));
    }
    if let Some(otherwise) = steps_of(step, "otherwise").filter(|o| !o.is_empty())
        && let Some(entry) = compile_chain(otherwise, &format!("{step_path}.otherwise"), state)
    {
        state.add_handle_edge(&node_id, &otherwise_id, &entry);
    }
    let node = with_steps(
        condition_node_default_fn(&json!({ "nodeProps": { "id": node_id } })),
        vec![condition_step_default_fn(
            &json!({ "cases": cases, "otherwiseId": otherwise_id }),
        )],
    );
    state.register_node(opt_str(step, "id"), node, step_path, Some(insert_at))
}

fn compile_goto_step(
    step: &Value,
    step_path: &str,
    state: &mut CompileState<'_>,
) -> Option<String> {
    let target_id = str_field(step, "targetId");
    if let Some(node_id) = state
        .step_id_to_node_id
        .get(target_id)
        .filter(|id| !id.is_empty())
    {
        return Some(node_id.clone());
    }
    state.add_error(
        &format!("{step_path}.targetId"),
        "invalidGotoTarget",
        format!("\"goto\" targets step id \"{target_id}\", which is not an earlier step's id in this spec."),
        Some("Set an explicit `id` on the step you want to jump to, earlier in the spec.".to_string()),
        None,
    );
    None
}

fn compile_step(step: &Value, step_path: &str, state: &mut CompileState<'_>) -> Option<String> {
    match str_field(step, "type") {
        "send" => Some(compile_send_step(step, step_path, state)),
        "sendTemplate" => compile_send_template_step(step, step_path, state),
        "sendProducts" => Some(compile_send_products_step(step, step_path, state)),
        "sendViewCatalog" => Some(compile_send_view_catalog_step(step, step_path, state)),
        "wait" => Some(compile_wait_step(step, step_path, state)),
        "branch" => Some(compile_branch_step(step, step_path, state)),
        "action" => Some(compile_action_step(step, step_path, state)),
        "pipeline" => compile_pipeline_step(step, step_path, state),
        "startFlow" => compile_start_flow_step(step, step_path, state),
        "addNote" => Some(compile_add_note_step(step, step_path, state)),
        "goto" => compile_goto_step(step, step_path, state),
        other => {
            state.add_error(
                step_path,
                "compileFailed",
                format!("Unhandled step type: {other}"),
                None,
                None,
            );
            None
        }
    }
}

/// Compiles a step list into a chain of nodes wired by "Continue" edges and
/// returns its entry point (`None` only for an empty list).
fn compile_chain(
    steps: &[Value],
    path_prefix: &str,
    state: &mut CompileState<'_>,
) -> Option<String> {
    let mut previous_node_id: Option<String> = None;
    let mut previous_step_was_terminal = false;
    let mut entry_node_id: Option<String> = None;

    for (index, step) in steps.iter().enumerate() {
        let step_path = format!("{path_prefix}[{index}]");
        let step_type = str_field(step, "type");
        let is_terminal = ends_step_list(step_type);

        if index < steps.len() - 1 && is_terminal {
            state.add_error(
                &format!("{path_prefix}[{}]", index + 1),
                "unreachableStep",
                format!(
                    "Step {} of {} can never run — \"{step_type}\" at step {} ends its step list.",
                    index + 2,
                    steps.len(),
                    index + 1
                ),
                Some(
                    "Move the following steps inside this step's own branch (e.g. a branch case's `then`) or delete them."
                        .to_string(),
                ),
                None,
            );
        }

        if let Some(node_id) = compile_step(step, &step_path, state) {
            if entry_node_id.is_none() {
                entry_node_id = Some(node_id.clone());
            }
            if let Some(previous) = previous_node_id.as_deref()
                && !previous_step_was_terminal
            {
                // A goto to the immediately preceding step would wire a node to itself.
                if node_id == previous {
                    state.add_error(
                        &step_path,
                        "selfLoopGoto",
                        "\"goto\" targets the immediately preceding step, which would create a self-loop edge.".to_string(),
                        Some(
                            "Target an earlier step, or remove this \"goto\" — the flow already continues into the preceding step by default."
                                .to_string(),
                        ),
                        None,
                    );
                } else {
                    let previous = previous.to_string();
                    state.add_continue_edge(&previous, &node_id);
                }
            }
            previous_node_id = Some(node_id);
        }
        previous_step_was_terminal = is_terminal;
    }
    entry_node_id
}

fn validate_structure(spec: &Value, state: &mut CompileState<'_>) {
    assert_no_duplicate_step_ids(spec, state);
    if arr_field(spec, "steps")
        .first()
        .is_some_and(|step| step["type"] == "goto")
    {
        state.add_error(
            "steps[0]",
            "invalidFirstStep",
            "\"goto\" cannot be the first step — there is no earlier step yet to jump from."
                .to_string(),
            None,
            None,
        );
    }
}

fn finalize_graph(
    state: CompileState<'_>,
    start_node_id: Option<String>,
) -> Result<CompiledFlow, FlowAuthoringException> {
    // A caller can bypass parsing and pass `steps: []`; the graph still needs a start node.
    let Some(start_node_id) = start_node_id else {
        return Err(FlowAuthoringException::new(vec![FlowAuthoringError {
            path: "steps".to_string(),
            code: "compileFailed".to_string(),
            message: "Compilation produced no nodes.".to_string(),
            ..FlowAuthoringError::default()
        }]));
    };
    let routed = apply_route_updates_in_nodes(&state.nodes, &state.route_updates);
    let node_ids: Vec<&str> = routed.iter().map(|node| str_field(node, "id")).collect();
    let layout_edges: Vec<LayoutEdge<'_>> = state
        .edges
        .iter()
        .map(|edge| LayoutEdge {
            source: str_field(edge, "source"),
            target: str_field(edge, "target"),
        })
        .collect();
    let positions = layout_nodes(&node_ids, &layout_edges, &start_node_id);
    let nodes = routed
        .iter()
        .map(|node| {
            let id = str_field(node, "id");
            let mut node = node.clone();
            if let Some(position) = positions.get(id) {
                node["position"] =
                    json!({ "x": num_value(position.x), "y": num_value(position.y) });
            }
            node["data"]["isStartNode"] = json!(id == start_node_id);
            node
        })
        .collect();
    Ok(CompiledFlow {
        start_node_id,
        nodes,
        edges: state.edges,
        spec_path_by_node_id: state.spec_path_by_node_id,
    })
}

/// `compileFlowSpec(spec, ctx)`: a `flowSpecSchema`-shaped spec to
/// `{ startNodeId, nodes, edges }` ready for publish. `Err` is the thrown
/// `FlowAuthoringException` (every error found, never a partial result).
pub fn compile_flow_spec(
    spec: &Value,
    ctx: &FlowAuthoringContext,
) -> Result<CompiledFlow, FlowAuthoringException> {
    let mut state = CompileState {
        nodes: Vec::new(),
        edges: Vec::new(),
        route_updates: Vec::new(),
        step_id_to_node_id: BTreeMap::new(),
        spec_path_by_node_id: BTreeMap::new(),
        errors: Vec::new(),
        ctx,
        channel: opt_str(spec, "channel").map(str::to_string),
    };
    validate_structure(spec, &mut state);
    let start_node_id = compile_chain(arr_field(spec, "steps"), "steps", &mut state);
    if !state.errors.is_empty() {
        return Err(FlowAuthoringException::new(state.errors));
    }
    finalize_graph(state, start_node_id)
}
