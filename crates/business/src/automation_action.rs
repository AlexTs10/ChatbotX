//! `automation-action/references.ts` (Q.D5): the records a trigger action
//! points at (flows, flow nodes, tags, sequences, custom / bot fields,
//! pipelines, stages, pipeline fields, members, inbox teams) and which of
//! them do not exist in the workspace.

use std::collections::{HashMap, HashSet};

use chatbotx_core::flow_config::field_reference::{FieldReference, parse_field_reference};
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};

/// One referenced record (`ActionReference`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Reference {
    Flow(String),
    FlowNode { flow_id: String, id: String },
    Tag(String),
    Sequence(String),
    CustomField(String),
    CustomFieldKey(String),
    BotField(String),
    Pipeline(String),
    PipelineStage { pipeline_id: String, id: String },
    PipelineField { pipeline_id: String, id: String },
    WorkspaceMember(String),
    InboxTeam(String),
}

/// A reference and the action field it came from (`ReferenceUse`).
struct Use {
    field: String,
    reference: Reference,
}

/// JS `String(value ?? "")`.
fn as_text(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| as_text(Some(item)))
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".to_string(),
    }
}

fn used(
    field: &str,
    value: Option<&Value>,
    to_reference: impl Fn(String) -> Reference,
    optional: bool,
) -> Vec<Use> {
    let id = as_text(value);
    if id.is_empty() && optional {
        return Vec::new();
    }
    vec![Use {
        field: field.to_string(),
        reference: to_reference(id),
    }]
}

fn used_each(
    field: &str,
    values: Option<&Value>,
    to_reference: impl Fn(String) -> Reference,
) -> Vec<Use> {
    values
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .flat_map(|(position, value)| {
            used(
                &format!("{field}.{position}"),
                Some(value),
                &to_reference,
                false,
            )
        })
        .collect()
}

fn custom_field_reference(action: &Value, by_key: bool) -> Vec<Use> {
    let reference = match parse_field_reference(&as_text(action.get("customFieldId"))) {
        FieldReference::BotField { id } => Reference::BotField(id),
        FieldReference::CustomField { key } if by_key => Reference::CustomFieldKey(key),
        FieldReference::CustomField { key } => Reference::CustomField(key),
    };
    vec![Use {
        field: "customFieldId".to_string(),
        reference,
    }]
}

fn due_date(action: &Value) -> Vec<Use> {
    let due = action.get("dueDate");
    if due.and_then(|d| d.get("mode")).and_then(Value::as_str) != Some("customField") {
        return Vec::new();
    }
    used(
        "dueDate.customFieldId",
        due.and_then(|d| d.get("customFieldId")),
        Reference::CustomField,
        true,
    )
}

fn assignees(action: &Value) -> Vec<Use> {
    let mut out = used_each(
        "assigneeMemberIds",
        action.get("assigneeMemberIds"),
        Reference::WorkspaceMember,
    );
    out.extend(used_each(
        "assigneeInboxTeamIds",
        action.get("assigneeInboxTeamIds"),
        Reference::InboxTeam,
    ));
    out
}

fn stage_of(action: &Value, field: &str, pipeline_key: &str, optional: bool) -> Vec<Use> {
    let pipeline_id = as_text(action.get(pipeline_key));
    used(
        field,
        action.get(field),
        |id| Reference::PipelineStage {
            pipeline_id: pipeline_id.clone(),
            id,
        },
        optional,
    )
}

fn pipeline_field(action: &Value, id: String) -> Reference {
    Reference::PipelineField {
        pipeline_id: as_text(action.get("pipelineId")),
        id,
    }
}

/// `PIPELINE_COLLECTORS` + the `pipelineId` itself.
fn pipeline(action: &Value) -> Vec<Use> {
    let mut out = used(
        "pipelineId",
        action.get("pipelineId"),
        Reference::Pipeline,
        false,
    );
    let step: Vec<Use> = match action.get("action").and_then(Value::as_str).unwrap_or("") {
        "addToPipeline" => {
            let mut uses = stage_of(action, "stageId", "pipelineId", true);
            uses.extend(assignees(action));
            uses.extend(due_date(action));
            for (position, entry) in action
                .get("fieldValues")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                uses.extend(used(
                    &format!("fieldValues.{position}.pipelineFieldId"),
                    entry.get("pipelineFieldId"),
                    |id| pipeline_field(action, id),
                    false,
                ));
            }
            uses
        }
        "moveToStage" => stage_of(action, "stageId", "pipelineId", false),
        "assign" => assignees(action),
        "changeDueDate" => due_date(action),
        "setCustomField" => used(
            "pipelineFieldId",
            action.get("pipelineFieldId"),
            |id| pipeline_field(action, id),
            true,
        ),
        "clearCustomField" => used_each("pipelineFieldIds", action.get("pipelineFieldIds"), |id| {
            pipeline_field(action, id)
        }),
        "transfer" => {
            let mut uses = used(
                "destinationPipelineId",
                action.get("destinationPipelineId"),
                Reference::Pipeline,
                false,
            );
            uses.extend(stage_of(
                action,
                "destinationStageId",
                "destinationPipelineId",
                true,
            ));
            uses
        }
        _ => Vec::new(),
    };
    out.extend(step);
    out
}

/// `COLLECTORS[action.type]`.
fn collect(action: &Value, field_names: bool) -> Vec<Use> {
    match as_text(action.get("type")).as_str() {
        "startAnotherFlow" => used("flowId", action.get("flowId"), Reference::Flow, false),
        "startExternalStep" => {
            let flow_id = as_text(action.get("flowId"));
            let mut out = used("flowId", action.get("flowId"), Reference::Flow, false);
            out.extend(used(
                "nodeId",
                action.get("nodeId"),
                |id| Reference::FlowNode {
                    flow_id: flow_id.clone(),
                    id,
                },
                false,
            ));
            out
        }
        "addTag" | "removeTag" => used_each("tagIds", action.get("tagIds"), Reference::Tag),
        "addSequence" | "removeSequence" => used_each(
            "sequenceIds",
            action.get("sequenceIds"),
            Reference::Sequence,
        ),
        "setCustomField" => custom_field_reference(action, field_names),
        "clearCustomField" => custom_field_reference(action, false),
        "pipeline" => pipeline(action),
        _ => Vec::new(),
    }
}

/// `isBigintId`: digits, and fits a bigint.
fn is_bigint_id(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) && value.parse::<i64>().is_ok()
}

/// `wellFormed`.
fn well_formed(reference: &Reference) -> bool {
    match reference {
        Reference::CustomFieldKey(key) => !key.is_empty(),
        Reference::FlowNode { flow_id, id } => is_bigint_id(id) && is_bigint_id(flow_id),
        Reference::PipelineStage { pipeline_id, id }
        | Reference::PipelineField { pipeline_id, id } => {
            is_bigint_id(id) && is_bigint_id(pipeline_id)
        }
        Reference::Flow(id)
        | Reference::Tag(id)
        | Reference::Sequence(id)
        | Reference::CustomField(id)
        | Reference::BotField(id)
        | Reference::Pipeline(id)
        | Reference::WorkspaceMember(id)
        | Reference::InboxTeam(id) => is_bigint_id(id),
    }
}

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// The ids (as stored, base 10) of `table` rows of the workspace among `ids`.
fn existing(
    conn: &Connection,
    table: &str,
    workspace_id: i64,
    ids: &[&String],
) -> Result<HashSet<String>> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let list: Vec<i64> = ids.iter().filter_map(|id| id.parse().ok()).collect();
    let sql = format!(
        r#"SELECT CAST("id" AS TEXT) FROM "{table}" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2))"#
    );
    let mut stmt = conn.prepare(&sql).map_err(repo_err)?;
    let rows = stmt
        .query_map(params![workspace_id, json!(list).to_string()], |r| {
            r.get::<_, String>(0)
        })
        .map_err(repo_err)?;
    rows.collect::<std::result::Result<_, _>>()
        .map_err(repo_err)
}

/// `(pipelineId, id)` of `table` rows (stages / fields) of these pipelines.
fn by_pipeline(
    conn: &Connection,
    table: &str,
    workspace_id: i64,
    pipeline_ids: &[&String],
) -> Result<HashMap<String, HashSet<String>>> {
    let mut out: HashMap<String, HashSet<String>> = HashMap::new();
    if pipeline_ids.is_empty() {
        return Ok(out);
    }
    let list: Vec<i64> = pipeline_ids
        .iter()
        .filter_map(|id| id.parse().ok())
        .collect();
    let sql = format!(
        r#"SELECT CAST("pipelineId" AS TEXT), CAST("id" AS TEXT) FROM "{table}" WHERE "workspaceId" = ?1 AND "pipelineId" IN (SELECT value FROM json_each(?2))"#
    );
    let mut stmt = conn.prepare(&sql).map_err(repo_err)?;
    let rows = stmt
        .query_map(params![workspace_id, json!(list).to_string()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(repo_err)?;
    for row in rows {
        let (pipeline, id) = row.map_err(repo_err)?;
        out.entry(pipeline).or_default().insert(id);
    }
    Ok(out)
}

/// `listCurrentVersionNodesByFlowIds`: the node ids of each flow's current version.
fn flow_nodes(
    conn: &Connection,
    workspace_id: i64,
    flow_ids: &[&String],
) -> Result<HashMap<String, HashSet<String>>> {
    let mut out: HashMap<String, HashSet<String>> = HashMap::new();
    if flow_ids.is_empty() {
        return Ok(out);
    }
    let list: Vec<i64> = flow_ids.iter().filter_map(|id| id.parse().ok()).collect();
    let mut stmt = conn
        .prepare(
            r#"SELECT CAST(f."id" AS TEXT), v."nodes" FROM "Flow" f
               JOIN "FlowVersion" v ON v."id" = f."currentVersionId" AND v."workspaceId" = ?1
               WHERE f."workspaceId" = ?1 AND f."id" IN (SELECT value FROM json_each(?2))"#,
        )
        .map_err(repo_err)?;
    let rows = stmt
        .query_map(params![workspace_id, json!(list).to_string()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(repo_err)?;
    for row in rows {
        let (flow, nodes) = row.map_err(repo_err)?;
        let nodes: Value = serde_json::from_str(&nodes).unwrap_or(Value::Null);
        let ids = nodes
            .as_array()
            .into_iter()
            .flatten()
            .map(|node| as_text(node.get("id")))
            .collect();
        out.insert(flow, ids);
    }
    Ok(out)
}

/// `customFieldService.findManyByKeys`: the keys that name a custom field
/// (an id key is compared by value, then by name).
fn custom_field_keys(
    conn: &Connection,
    workspace_id: i64,
    keys: &[&String],
) -> Result<HashSet<String>> {
    let mut out = HashSet::new();
    let mut stmt = conn
        .prepare(
            r#"SELECT 1 FROM "CustomField" WHERE "workspaceId" = ?1 AND ("id" = ?2 OR "name" = ?3) LIMIT 1"#,
        )
        .map_err(repo_err)?;
    for key in keys {
        let id = if is_bigint_id(key) {
            key.parse::<i64>().ok()
        } else {
            None
        };
        if stmt
            .exists(params![workspace_id, id, key.as_str()])
            .map_err(repo_err)?
        {
            out.insert((*key).clone());
        }
    }
    Ok(out)
}

/// `findInvalidActionReferences`: `(index, field)` of each reference that is
/// malformed or names nothing in the workspace. `indexes` limits the check
/// to those actions; `field_names` lets `setCustomField` name a custom field
/// by name.
pub fn find_invalid_action_references(
    conn: &Connection,
    workspace_id: i64,
    actions: &[Value],
    indexes: Option<&HashSet<usize>>,
    field_names: bool,
) -> Result<Vec<(usize, String)>> {
    let entries: Vec<(usize, Use)> = actions
        .iter()
        .enumerate()
        .filter(|(index, _)| indexes.is_none_or(|set| set.contains(index)))
        .flat_map(|(index, action)| {
            collect(action, field_names)
                .into_iter()
                .map(move |u| (index, u))
        })
        .collect();
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let good: Vec<&Reference> = entries
        .iter()
        .map(|(_, u)| &u.reference)
        .filter(|r| well_formed(r))
        .collect();
    let ids = |pick: fn(&Reference) -> Option<&String>| -> Vec<&String> {
        let mut seen = HashSet::new();
        good.iter()
            .filter_map(|r| pick(r))
            .filter(|id| seen.insert(*id))
            .collect()
    };
    let flows = existing(
        conn,
        "Flow",
        workspace_id,
        &ids(|r| match r {
            Reference::Flow(id) => Some(id),
            _ => None,
        }),
    )?;
    let nodes = flow_nodes(
        conn,
        workspace_id,
        &ids(|r| match r {
            Reference::FlowNode { flow_id, .. } => Some(flow_id),
            _ => None,
        }),
    )?;
    let tags = existing(
        conn,
        "Tag",
        workspace_id,
        &ids(|r| match r {
            Reference::Tag(id) => Some(id),
            _ => None,
        }),
    )?;
    let sequences = existing(
        conn,
        "Sequence",
        workspace_id,
        &ids(|r| match r {
            Reference::Sequence(id) => Some(id),
            _ => None,
        }),
    )?;
    let custom_fields = existing(
        conn,
        "CustomField",
        workspace_id,
        &ids(|r| match r {
            Reference::CustomField(id) => Some(id),
            _ => None,
        }),
    )?;
    let field_keys = custom_field_keys(
        conn,
        workspace_id,
        &ids(|r| match r {
            Reference::CustomFieldKey(key) => Some(key),
            _ => None,
        }),
    )?;
    let bot_fields = existing(
        conn,
        "BotField",
        workspace_id,
        &ids(|r| match r {
            Reference::BotField(id) => Some(id),
            _ => None,
        }),
    )?;
    let pipelines = existing(
        conn,
        "Pipeline",
        workspace_id,
        &ids(|r| match r {
            Reference::Pipeline(id) => Some(id),
            _ => None,
        }),
    )?;
    let stages = by_pipeline(
        conn,
        "PipelineStage",
        workspace_id,
        &ids(|r| match r {
            Reference::PipelineStage { pipeline_id, .. } => Some(pipeline_id),
            _ => None,
        }),
    )?;
    let fields = by_pipeline(
        conn,
        "PipelineField",
        workspace_id,
        &ids(|r| match r {
            Reference::PipelineField { pipeline_id, .. } => Some(pipeline_id),
            _ => None,
        }),
    )?;
    let members = existing(
        conn,
        "WorkspaceMember",
        workspace_id,
        &ids(|r| match r {
            Reference::WorkspaceMember(id) => Some(id),
            _ => None,
        }),
    )?;
    let teams = existing(
        conn,
        "InboxTeam",
        workspace_id,
        &ids(|r| match r {
            Reference::InboxTeam(id) => Some(id),
            _ => None,
        }),
    )?;
    let in_group = |map: &HashMap<String, HashSet<String>>, group: &String, id: &String| {
        map.get(group).is_some_and(|set| set.contains(id))
    };
    let resolves = |reference: &Reference| match reference {
        Reference::Flow(id) => flows.contains(id),
        Reference::FlowNode { flow_id, id } => in_group(&nodes, flow_id, id),
        Reference::Tag(id) => tags.contains(id),
        Reference::Sequence(id) => sequences.contains(id),
        Reference::CustomField(id) => custom_fields.contains(id),
        Reference::CustomFieldKey(key) => field_keys.contains(key),
        Reference::BotField(id) => bot_fields.contains(id),
        Reference::Pipeline(id) => pipelines.contains(id),
        Reference::PipelineStage { pipeline_id, id } => in_group(&stages, pipeline_id, id),
        Reference::PipelineField { pipeline_id, id } => in_group(&fields, pipeline_id, id),
        Reference::WorkspaceMember(id) => members.contains(id),
        Reference::InboxTeam(id) => teams.contains(id),
    };
    Ok(entries
        .into_iter()
        .filter(|(_, u)| !(well_formed(&u.reference) && resolves(&u.reference)))
        .map(|(index, u)| (index, u.field))
        .collect())
}

/// `indexesOfNewActions`: the indexes of `next` with no equal action left in
/// `stored` (each stored action matches once). `Value` equality ignores key
/// order, like `stableStringify`.
pub fn indexes_of_new_actions(next: &[Value], stored: &[Value]) -> HashSet<usize> {
    let mut remaining: Vec<Option<&Value>> = stored.iter().map(Some).collect();
    let mut fresh = HashSet::new();
    for (index, action) in next.iter().enumerate() {
        match remaining
            .iter_mut()
            .find(|slot| slot.is_some_and(|s| s == action))
        {
            Some(slot) => *slot = None,
            None => {
                fresh.insert(index);
            }
        }
    }
    fresh
}
