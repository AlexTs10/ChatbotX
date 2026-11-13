//! `pipeline/flow-publish-check.ts`: a flow cannot be published while an
//! "Add to pipeline" step leaves a required pipeline field empty.

use std::collections::BTreeSet;

use chatbotx_core::flow_config::find_missing_required_pipeline_fields;
use chatbotx_db::Db;
use chatbotx_db::repo::pipeline as pipeline_repo;
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

struct AddToPipelineStep {
    node_name: String,
    pipeline_id: i64,
    field_values: Value,
}

/// `pipelineStepSchema.safeParse(step)` for `addToPipeline`, reduced to what
/// the check reads: a string `pipelineId` and a `fieldValues` array of
/// `{ pipelineFieldId, value }` strings. Anything else is skipped (as a
/// failed parse). A non-numeric pipeline id has no fields.
fn add_to_pipeline_step(node_name: &str, step: &Value) -> Option<AddToPipelineStep> {
    if step.get("stepType")?.as_str()? != "pipeline"
        || step.get("action")?.as_str()? != "addToPipeline"
    {
        return None;
    }
    let pipeline_id = step.get("pipelineId")?.as_str()?;
    let field_values = step.get("fieldValues")?.as_array()?;
    let rows_ok = field_values.iter().all(|row| {
        row.get("pipelineFieldId").is_some_and(Value::is_string)
            && row.get("value").is_some_and(Value::is_string)
    });
    if !rows_ok {
        return None;
    }
    Some(AddToPipelineStep {
        node_name: node_name.to_string(),
        pipeline_id: pipeline_id.parse().ok()?,
        field_values: Value::Array(field_values.clone()),
    })
}

/// `collectAddToPipelineSteps`: an unreadable node list has no steps.
fn collect_add_to_pipeline_steps(nodes: &Value) -> Vec<AddToPipelineStep> {
    let Some(nodes) = nodes.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for node in nodes {
        let data = node.get("data");
        let name = data
            .and_then(|d| d.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let steps = data
            .and_then(|d| d.get("details"))
            .and_then(|d| d.get("steps"))
            .and_then(Value::as_array);
        for step in steps.into_iter().flatten() {
            out.extend(add_to_pipeline_step(name, step));
        }
    }
    out
}

/// `assertFlowPipelineStepsPublishable` (`nodes` = the flow version nodes).
pub async fn assert_flow_pipeline_steps_publishable(
    db: &Db,
    workspace_id: i64,
    nodes: &Value,
) -> Result<()> {
    let steps = collect_add_to_pipeline_steps(nodes);
    if steps.is_empty() {
        return Ok(());
    }
    let pipeline_ids: Vec<i64> = steps
        .iter()
        .map(|step| step.pipeline_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let fields = tx::read(db, move |conn| {
        Ok(pipeline_repo::list_fields_by_pipeline_ids(
            conn,
            workspace_id,
            &pipeline_ids,
        )?)
    })
    .await?;
    let problems: Vec<String> = steps
        .iter()
        .filter_map(|step| {
            let definitions: Vec<Value> = fields
                .iter()
                .filter(|field| field.pipeline_id == step.pipeline_id)
                .map(|field| {
                    json!({
                        "id": field.id.to_string(),
                        "name": field.name,
                        "required": field.required,
                    })
                })
                .collect();
            let missing = find_missing_required_pipeline_fields(
                &step.field_values,
                &Value::Array(definitions),
            );
            if missing.is_empty() {
                return None;
            }
            let names: Vec<&str> = missing
                .iter()
                .map(|field| field["name"].as_str().unwrap_or(""))
                .collect();
            Some(format!("\"{}\": {}", step.node_name, names.join(", ")))
        })
        .collect();
    if problems.is_empty() {
        return Ok(());
    }
    Err(BusinessError::validation(
        "nodes",
        format!(
            "Add to pipeline is missing required pipeline fields. Set a value for each in the step: {}",
            problems.join("; ")
        ),
    ))
}
