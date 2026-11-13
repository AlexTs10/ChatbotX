//! `pipeline/flow-actions.ts`: the Pipelines flow step / trigger actions
//! (`runPipelineAction`, `runPipelineFlowAction`,
//! `PIPELINE_FLOW_ACTION_HANDLERS`). The step is the flow-config JSON
//! (`pipelineStepSchema` / a trigger action); ids in it are strings.

use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chatbotx_core::js;
use chatbotx_core::utils::datetime::is_temporal_custom_field_type;
use chatbotx_db::Db;
use chatbotx_db::repo::models::CustomField;
use chatbotx_db::repo::partials::pipeline::PIPELINE_LIMITS;
use chatbotx_db::repo::{
    Model, is_bigint_id, pipeline as pipeline_repo, pipeline_item as item_repo,
    pipeline_reference as reference_repo, pipeline_stage as stage_repo,
};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::Value;

use super::access::PipelineAccessScope;
use super::content::{self, ItemRef};
use super::events::PipelineItemChanged;
use super::item::{self, CreateItem, ItemFields, ItemMutation, TransferItem, UpdateItem};
use super::mutation::PipelineActor;
use crate::errors::{BusinessError, Result};
use crate::events::TriggerEvent;
use crate::tx;

/// Flow steps are system context: the workspace owns the pipeline, not a member.
const FLOW_SCOPE: PipelineAccessScope = PipelineAccessScope::System;
const FALLBACK_TITLE: &str = "Untitled";
const PIPELINE_MISSING_MESSAGE: &str = "The pipeline does not exist in this workspace";

/// Dedup is only safe for actions that add something; a retried "set" is already idempotent.
const DEDUPED_ACTIONS: &[&str] = &["addToPipeline", "addComment"];

/// The future an [`Interpolate`] returns.
pub type InterpolateFuture = Pin<Box<dyn Future<Output = String> + Send>>;

/// `interpolate`: resolves `{{variables}}` for the contact.
pub type Interpolate = Arc<dyn Fn(String) -> InterpolateFuture + Send + Sync>;

/// An [`Interpolate`] from a synchronous function.
pub fn interpolate_fn(f: impl Fn(&str) -> String + Send + Sync + 'static) -> Interpolate {
    Arc::new(move |text: String| {
        let out = f(&text);
        Box::pin(async move { out }) as InterpolateFuture
    })
}

/// `PipelineFlowActionStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineFlowActionStatus {
    Success,
    Error,
    Skip,
}

/// `PipelineFlowActionResult`, plus the realtime events and flow-trigger
/// events of the write (the caller publishes them; the business crate has no
/// realtime handle or event bus).
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineFlowActionResult {
    pub status: PipelineFlowActionStatus,
    /// Why the step errored or skipped; written to logs, never shown to the contact.
    pub message: Option<String>,
    pub events: Vec<PipelineItemChanged>,
    pub triggers: Vec<TriggerEvent>,
}

impl PipelineFlowActionResult {
    fn of(status: PipelineFlowActionStatus, message: Option<String>) -> PipelineFlowActionResult {
        PipelineFlowActionResult {
            status,
            message,
            events: Vec::new(),
            triggers: Vec::new(),
        }
    }

    fn success() -> PipelineFlowActionResult {
        PipelineFlowActionResult::of(PipelineFlowActionStatus::Success, None)
    }

    fn fail(message: impl Into<String>) -> PipelineFlowActionResult {
        PipelineFlowActionResult::of(PipelineFlowActionStatus::Error, Some(message.into()))
    }

    fn skipped(message: impl Into<String>) -> PipelineFlowActionResult {
        PipelineFlowActionResult::of(PipelineFlowActionStatus::Skip, Some(message.into()))
    }

    fn written<T>(mutation: ItemMutation<T>) -> PipelineFlowActionResult {
        PipelineFlowActionResult {
            events: mutation.events,
            triggers: mutation.triggers,
            ..PipelineFlowActionResult::success()
        }
    }
}

/// `PipelineFlowActionContext`.
#[derive(Clone)]
pub struct PipelineFlowActionContext {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub flow_id: i64,
    /// Stable per flow run; present only when the executor supplied one.
    pub flow_execution_key: Option<String>,
    pub interpolate: Interpolate,
}

/// `PipelineActionContext`: what runs a pipeline action (a flow step, a trigger, ...).
#[derive(Clone)]
pub struct PipelineActionContext {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub actor: PipelineActor,
    /// Durable key of this occurrence of the action, identical on every retry.
    /// Only the actions that add something use it (`DEDUPED_ACTIONS`); when
    /// absent, a retried add is not deduplicated.
    pub base_idempotency_key: Option<String>,
    pub interpolate: Interpolate,
}

/// `PIPELINE_FLOW_ACTION_HANDLERS`: the actions that have a handler (the
/// handlers are the `match` in [`handle`]).
pub const PIPELINE_FLOW_ACTION_HANDLERS: &[&str] = &[
    "addToPipeline",
    "moveToStage",
    "changeValue",
    "assign",
    "changeStatus",
    "changePriority",
    "changeDueDate",
    "changeTitle",
    "setCustomField",
    "clearCustomField",
    "changeDescription",
    "removeFromPipeline",
    "addComment",
    "transfer",
];

/// `ActionEnv`.
struct Env<'a> {
    db: &'a Db,
    ctx: &'a PipelineActionContext,
    pipeline_id: i64,
    /// Dedup key for the one action per step that must not repeat on a retry.
    idempotency_key: Option<String>,
}

impl Env<'_> {
    async fn interpolate(&self, text: &str) -> String {
        (self.ctx.interpolate)(text.to_string()).await
    }

    /// `interpolateText`: interpolates `text`, trims it and cuts it to `max` UTF-16 units.
    async fn interpolate_text(&self, text: &str, max: usize) -> String {
        let interpolated = self.interpolate(text).await;
        js::slice_utf16(js::trim(&interpolated), 0, max).to_string()
    }
}

// Step fields ----------------------------------------------------------------

fn text<'a>(step: &'a Value, key: &str) -> &'a str {
    step.get(key).and_then(Value::as_str).unwrap_or("")
}

fn texts(step: &Value, key: &str) -> Vec<String> {
    step.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| item.as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// `(pipelineFieldId, value)` of each `fieldValues` row.
fn field_value_rows(step: &Value) -> Vec<(String, String)> {
    step.get("fieldValues")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    (
                        text(row, "pipelineFieldId").to_string(),
                        text(row, "value").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

// Id checks ------------------------------------------------------------------

/// `assertIdShape`.
fn assert_id_shape(id: &str, field: &str) -> Result<i64> {
    if !is_bigint_id(id) {
        return Err(BusinessError::validation(field, "The id is not valid"));
    }
    id.parse()
        .map_err(|_| BusinessError::validation(field, "The id is not valid"))
}

/// `assertIdsShape`.
fn assert_ids_shape(ids: &[String], field: &str) -> Result<Vec<i64>> {
    ids.iter().map(|id| assert_id_shape(id, field)).collect()
}

/// `requireId`.
fn require_id(id: &str, field: &str, label: &str) -> Result<i64> {
    if id.is_empty() {
        return Err(BusinessError::validation(
            field,
            format!("Choose a {label}"),
        ));
    }
    assert_id_shape(id, field)
}

// Lookups --------------------------------------------------------------------

/// Ids of the step that still exist; deleted ones are dropped.
struct LiveReferences {
    member_ids: Vec<i64>,
    team_ids: Vec<i64>,
    field_ids: Vec<i64>,
    stage_id: Option<i64>,
}

#[derive(Default)]
struct References {
    member_ids: Vec<i64>,
    team_ids: Vec<i64>,
    field_ids: Vec<i64>,
    stage_id: Option<i64>,
}

/// `liveReferences`: the admins, teams, pipeline fields and stage a step names
/// that still exist. A flow keeps running after one of them is deleted: the
/// missing ones are dropped (and logged). Only the lookups the step needs are made.
async fn live_references(env: &Env<'_>, refs: References) -> Result<LiveReferences> {
    let workspace_id = env.ctx.workspace_id;
    let pipeline_id = env.pipeline_id;
    let source_id = env.ctx.actor.source_id.clone();
    tx::read(env.db, move |conn| {
        let members: HashSet<i64> =
            reference_repo::list_workspace_members(conn, workspace_id, &refs.member_ids)?
                .into_iter()
                .map(|member| member.id)
                .collect();
        let teams: HashSet<i64> =
            reference_repo::list_inbox_team_ids(conn, workspace_id, &refs.team_ids)?
                .into_iter()
                .collect();
        let fields: HashSet<i64> = if refs.field_ids.is_empty() {
            HashSet::new()
        } else {
            pipeline_repo::list_fields(conn, workspace_id, pipeline_id)?
                .into_iter()
                .map(|field| field.id)
                .collect()
        };
        let stage_live = match refs.stage_id {
            Some(stage_id) => stage_repo::list_by_pipeline(conn, workspace_id, pipeline_id)?
                .iter()
                .any(|stage| stage.id == stage_id),
            None => false,
        };
        let split = |ids: &[i64], set: &HashSet<i64>| -> (Vec<i64>, Vec<i64>) {
            ids.iter().copied().partition(|id| set.contains(id))
        };
        let (member_ids, dropped_members) = split(&refs.member_ids, &members);
        let (team_ids, dropped_teams) = split(&refs.team_ids, &teams);
        let (field_ids, dropped_fields) = split(&refs.field_ids, &fields);
        let dropped_stage = refs.stage_id.filter(|_| !stage_live);
        if !dropped_members.is_empty()
            || !dropped_teams.is_empty()
            || !dropped_fields.is_empty()
            || dropped_stage.is_some()
        {
            tracing::warn!(
                workspace_id,
                pipeline_id,
                source_id = ?source_id,
                dropped_member_ids = ?dropped_members,
                dropped_team_ids = ?dropped_teams,
                dropped_field_ids = ?dropped_fields,
                dropped_stage_id = ?dropped_stage,
                "Pipeline step names deleted references; they were skipped"
            );
        }
        Ok(LiveReferences {
            member_ids,
            team_ids,
            field_ids,
            stage_id: refs.stage_id.filter(|_| stage_live),
        })
    })
    .await
}

fn find_custom_field(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<CustomField>> {
    conn.query_row(
        &format!(
            r#"{} WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            CustomField::select_sql()
        ),
        params![id, workspace_id],
        CustomField::from_row,
    )
    .optional()
    .map_err(|error| BusinessError::Repo(error.into()))
}

/// `contactCustomFieldValueService.findValue`.
fn find_contact_value(conn: &Connection, contact_id: i64, field_id: i64) -> Result<Option<String>> {
    conn.query_row(
        r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = ?1 AND "customFieldId" = ?2"#,
        params![contact_id, field_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| BusinessError::Repo(error.into()))
}

/// `MissingDueDateField`: how a due date read from a contact custom field
/// treats a field that was deleted (or no longer holds dates).
#[derive(Clone, Copy, PartialEq, Eq)]
enum MissingDueDateField {
    /// Adding an item still adds it without a due date.
    NoDueDate,
    /// "Change due date" fails instead of clearing the date.
    Error,
}

/// `resolveDueDate`: unix ms or `None`.
async fn resolve_due_date(
    due_date: &Value,
    env: &Env<'_>,
    on_missing_field: MissingDueDateField,
) -> Result<Option<i64>> {
    match text(due_date, "mode") {
        "none" => return Ok(None),
        "fixed" => {
            return js::parse_date(text(due_date, "at"))
                .map(Some)
                .ok_or_else(|| {
                    BusinessError::validation("dueDate", "The fixed due date is not valid")
                });
        }
        _ => {}
    }
    let workspace_id = env.ctx.workspace_id;
    let contact_id = env.ctx.contact_id;
    let custom_field_id = text(due_date, "customFieldId");
    if custom_field_id.is_empty() {
        return Err(BusinessError::validation(
            "dueDate",
            "Choose a due date field",
        ));
    }
    let custom_field_id = assert_id_shape(custom_field_id, "dueDate")?;
    let (field, stored) = tx::read(env.db, move |conn| {
        let field = find_custom_field(conn, workspace_id, custom_field_id)?
            .filter(|field| is_temporal_custom_field_type(&field.r#type));
        let stored = match &field {
            Some(field) => find_contact_value(conn, contact_id, field.id)?,
            None => None,
        };
        Ok((field, stored))
    })
    .await?;
    if field.is_none() {
        if on_missing_field == MissingDueDateField::NoDueDate {
            tracing::warn!(
                workspace_id,
                custom_field_id,
                "Pipeline step due date field no longer exists; adding without a due date"
            );
            return Ok(None);
        }
        return Err(BusinessError::validation(
            "dueDate",
            "The due date field must be an existing date or datetime custom field",
        ));
    }
    let Some(stored) = stored.filter(|value| !js::trim(value).is_empty()) else {
        return Ok(None);
    };
    js::parse_date(&stored).map(Some).ok_or_else(|| {
        BusinessError::validation(
            "dueDate",
            "The contact's due date field does not hold a valid date",
        )
    })
}

/// `parseValue`: a number >= 0 (major units) as the stored x 10,000 integer.
fn parse_value(raw: &str) -> Result<i64> {
    let trimmed = js::trim(raw);
    let parsed = if trimmed.is_empty() {
        f64::NAN
    } else {
        js::to_number(trimmed)
    };
    if !parsed.is_finite() || parsed < 0.0 || parsed > PIPELINE_LIMITS.item_value as f64 {
        return Err(BusinessError::validation(
            "value",
            "Value must be a number that is 0 or greater",
        ));
    }
    Ok((parsed * 10_000.0).round() as i64)
}

// Handlers -------------------------------------------------------------------

async fn update_target(
    env: &Env<'_>,
    item_id: i64,
    fields: ItemFields,
) -> Result<PipelineFlowActionResult> {
    let mutation = item::update(
        env.db,
        UpdateItem {
            workspace_id: env.ctx.workspace_id,
            pipeline_id: env.pipeline_id,
            item_id,
            scope: FLOW_SCOPE,
            actor: env.ctx.actor.clone(),
            fields,
        },
    )
    .await?;
    Ok(PipelineFlowActionResult::written(mutation))
}

async fn add_to_pipeline(step: &Value, env: &Env<'_>) -> Result<PipelineFlowActionResult> {
    let stage_id = Some(text(step, "stageId")).filter(|id| !id.is_empty());
    let member_ids = assert_ids_shape(&texts(step, "assigneeMemberIds"), "assigneeMemberIds")?;
    let team_ids = assert_ids_shape(&texts(step, "assigneeInboxTeamIds"), "assigneeInboxTeamIds")?;
    let rows = field_value_rows(step);
    let named: Vec<String> = rows
        .iter()
        .map(|(id, _)| id.clone())
        .filter(|id| !id.is_empty())
        .collect();
    let field_ids = assert_ids_shape(&named, "fieldValues")?;
    let stage_id = stage_id
        .map(|id| assert_id_shape(id, "stageId"))
        .transpose()?;
    // A deleted stage falls back to the first one; deleted admins, teams and
    // fields are left out, so the item is still created.
    let live = live_references(
        env,
        References {
            member_ids,
            team_ids,
            field_ids,
            stage_id,
        },
    )
    .await?;
    let title = env
        .interpolate_text(text(step, "title"), PIPELINE_LIMITS.title)
        .await;
    let description = env
        .interpolate_text(text(step, "description"), PIPELINE_LIMITS.description)
        .await;
    let due_date = resolve_due_date(
        step.get("dueDate").unwrap_or(&Value::Null),
        env,
        MissingDueDateField::NoDueDate,
    )
    .await?;
    let live_fields: HashSet<String> = live.field_ids.iter().map(i64::to_string).collect();
    let mut field_values = BTreeMap::new();
    for (field_id, value) in rows {
        if live_fields.contains(&field_id) {
            let value = env.interpolate(&value).await;
            field_values.insert(field_id, Some(value));
        }
    }
    let status = Some(text(step, "status"))
        .filter(|status| !status.is_empty())
        .unwrap_or("open")
        .to_string();
    let mutation = item::create(
        env.db,
        CreateItem {
            workspace_id: env.ctx.workspace_id,
            pipeline_id: env.pipeline_id,
            contact_id: env.ctx.contact_id,
            scope: FLOW_SCOPE,
            actor: env.ctx.actor.clone(),
            fields: ItemFields {
                title: Some(if title.is_empty() {
                    FALLBACK_TITLE.to_string()
                } else {
                    title
                }),
                description: Some(Some(description).filter(|d| !d.is_empty())),
                status: Some(status),
                stage_id: live.stage_id,
                due_date: Some(due_date),
                assignee_member_ids: Some(live.member_ids),
                assignee_inbox_team_ids: Some(live.team_ids),
                field_values: Some(field_values),
                ..ItemFields::default()
            },
            media_library_file_ids: Vec::new(),
            idempotency_key: env.idempotency_key.clone(),
        },
    )
    .await?;
    Ok(PipelineFlowActionResult::written(mutation))
}

/// The contact-scoped actions, run against the contact's latest open item.
async fn on_target(action: &str, step: &Value, env: &Env<'_>) -> Result<PipelineFlowActionResult> {
    let workspace_id = env.ctx.workspace_id;
    let pipeline_id = env.pipeline_id;
    let contact_id = env.ctx.contact_id;
    let (target, pipeline_exists) = tx::read(env.db, move |conn| {
        let target =
            item_repo::find_latest_open_for_contact(conn, workspace_id, pipeline_id, contact_id)?;
        // Only the miss path pays for the existence check: a deleted pipeline
        // routes to error, a pipeline the contact simply is not in to skip.
        let exists = match target {
            Some(_) => true,
            None => pipeline_repo::find_by_id(conn, workspace_id, pipeline_id)?.is_some(),
        };
        Ok((target, exists))
    })
    .await?;
    let Some(target) = target else {
        return Ok(if pipeline_exists {
            PipelineFlowActionResult::skipped("The contact has no open item in this pipeline")
        } else {
            PipelineFlowActionResult::fail(PIPELINE_MISSING_MESSAGE)
        });
    };
    let item_id = target.id;
    match action {
        "moveToStage" => {
            let stage_id = require_id(text(step, "stageId"), "stageId", "stage")?;
            let fields = ItemFields {
                stage_id: Some(stage_id),
                ..ItemFields::default()
            };
            update_target(env, item_id, fields).await
        }
        "changeValue" => {
            let value = parse_value(&env.interpolate(text(step, "value")).await)?;
            let fields = ItemFields {
                value: Some(Some(value)),
                ..ItemFields::default()
            };
            update_target(env, item_id, fields).await
        }
        "assign" => {
            let members = texts(step, "assigneeMemberIds");
            let teams = texts(step, "assigneeInboxTeamIds");
            // An empty assignee list would otherwise clear the item's assignees
            // and report success.
            if members.is_empty() && teams.is_empty() {
                return Err(BusinessError::validation(
                    "assigneeMemberIds",
                    "The assign step has no assignees",
                ));
            }
            let member_ids = assert_ids_shape(&members, "assigneeMemberIds")?;
            let team_ids = assert_ids_shape(&teams, "assigneeInboxTeamIds")?;
            let live = live_references(
                env,
                References {
                    member_ids,
                    team_ids,
                    ..References::default()
                },
            )
            .await?;
            if live.member_ids.is_empty() && live.team_ids.is_empty() {
                return Err(BusinessError::validation(
                    "assigneeMemberIds",
                    "None of the step's assignees exist any more",
                ));
            }
            let fields = ItemFields {
                assignee_member_ids: Some(live.member_ids),
                assignee_inbox_team_ids: Some(live.team_ids),
                ..ItemFields::default()
            };
            update_target(env, item_id, fields).await
        }
        "changeStatus" => {
            let fields = ItemFields {
                status: Some(text(step, "status").to_string()),
                ..ItemFields::default()
            };
            update_target(env, item_id, fields).await
        }
        "changePriority" => {
            let fields = ItemFields {
                priority: Some(text(step, "priority").to_string()),
                ..ItemFields::default()
            };
            update_target(env, item_id, fields).await
        }
        "changeDueDate" => {
            let due_date = resolve_due_date(
                step.get("dueDate").unwrap_or(&Value::Null),
                env,
                MissingDueDateField::Error,
            )
            .await?;
            let fields = ItemFields {
                due_date: Some(due_date),
                ..ItemFields::default()
            };
            update_target(env, item_id, fields).await
        }
        "changeTitle" => {
            let title = env
                .interpolate_text(text(step, "title"), PIPELINE_LIMITS.title)
                .await;
            if title.is_empty() {
                return Err(BusinessError::validation("title", "The title is empty"));
            }
            let fields = ItemFields {
                title: Some(title),
                ..ItemFields::default()
            };
            update_target(env, item_id, fields).await
        }
        "setCustomField" => {
            let field_id = text(step, "pipelineFieldId");
            if field_id.is_empty() {
                return Err(BusinessError::validation(
                    "pipelineFieldId",
                    "Choose a field",
                ));
            }
            assert_id_shape(field_id, "pipelineFieldId")?;
            let value = env.interpolate(text(step, "value")).await;
            let fields = ItemFields {
                field_values: Some(BTreeMap::from([(field_id.to_string(), Some(value))])),
                ..ItemFields::default()
            };
            update_target(env, item_id, fields).await
        }
        "clearCustomField" => {
            let ids: Vec<String> = texts(step, "pipelineFieldIds")
                .into_iter()
                .filter(|id| !id.is_empty())
                .collect();
            if ids.is_empty() {
                return Err(BusinessError::validation(
                    "pipelineFieldIds",
                    "Choose at least one field",
                ));
            }
            let field_ids = assert_ids_shape(&ids, "pipelineFieldIds")?;
            // A deleted field has no value left to clear.
            let live = live_references(
                env,
                References {
                    field_ids,
                    ..References::default()
                },
            )
            .await?;
            if live.field_ids.is_empty() {
                return Ok(PipelineFlowActionResult::success());
            }
            let fields = ItemFields {
                field_values: Some(
                    live.field_ids
                        .iter()
                        .map(|id| (id.to_string(), None))
                        .collect(),
                ),
                ..ItemFields::default()
            };
            update_target(env, item_id, fields).await
        }
        "changeDescription" => {
            let description = env
                .interpolate_text(text(step, "description"), PIPELINE_LIMITS.description)
                .await;
            let fields = ItemFields {
                description: Some(Some(description).filter(|d| !d.is_empty())),
                ..ItemFields::default()
            };
            update_target(env, item_id, fields).await
        }
        "removeFromPipeline" => {
            let mutation = item::delete(
                env.db,
                env.ctx.workspace_id,
                env.pipeline_id,
                item_id,
                FLOW_SCOPE,
                env.ctx.actor.clone(),
            )
            .await?;
            Ok(PipelineFlowActionResult::written(mutation))
        }
        "addComment" => {
            let interpolated = env.interpolate(text(step, "text")).await;
            let comment = js::trim(&interpolated).to_string();
            if comment.is_empty() {
                return Err(BusinessError::validation("text", "The comment is empty"));
            }
            if js::utf16_len(&comment) > PIPELINE_LIMITS.comment {
                return Err(BusinessError::validation(
                    "text",
                    format!(
                        "The comment is longer than {} characters",
                        PIPELINE_LIMITS.comment
                    ),
                ));
            }
            let mutation = content::add_comment(
                env.db,
                ItemRef {
                    workspace_id: env.ctx.workspace_id,
                    pipeline_id: env.pipeline_id,
                    item_id,
                    scope: FLOW_SCOPE,
                },
                env.ctx.actor.clone(),
                comment,
                env.idempotency_key.clone(),
            )
            .await?;
            Ok(PipelineFlowActionResult::written(mutation))
        }
        "transfer" => {
            let destination_pipeline_id = require_id(
                text(step, "destinationPipelineId"),
                "destinationPipelineId",
                "destination pipeline",
            )?;
            let destination_stage_id = Some(text(step, "destinationStageId"))
                .filter(|id| !id.is_empty())
                .map(|id| assert_id_shape(id, "destinationStageId"))
                .transpose()?;
            let mutation = item::transfer(
                env.db,
                TransferItem {
                    workspace_id: env.ctx.workspace_id,
                    pipeline_id: env.pipeline_id,
                    item_id,
                    destination_pipeline_id,
                    destination_stage_id,
                    scope: FLOW_SCOPE,
                    actor: env.ctx.actor.clone(),
                },
            )
            .await?;
            Ok(PipelineFlowActionResult::written(mutation))
        }
        other => Ok(PipelineFlowActionResult::fail(format!(
            "Unknown pipeline action: {other}"
        ))),
    }
}

/// `PIPELINE_FLOW_ACTION_HANDLERS[action]`.
async fn handle(action: &str, step: &Value, env: &Env<'_>) -> Result<PipelineFlowActionResult> {
    match action {
        "addToPipeline" => add_to_pipeline(step, env).await,
        other => on_target(other, step, env).await,
    }
}

/// `runPipelineAction`: runs one Pipelines action for a contact. Never fails
/// for caller-fixable problems: a missing target is `skip`, an exception
/// (validation / conflict / not found) is `error`. A repository error is
/// `Err`, so the job retries. An unknown action is `error` (the original threw
/// a `TypeError`).
pub async fn run_pipeline_action(
    db: &Db,
    action: &Value,
    ctx: &PipelineActionContext,
) -> Result<PipelineFlowActionResult> {
    let pipeline_id = text(action, "pipelineId");
    if pipeline_id.is_empty() {
        return Ok(PipelineFlowActionResult::fail("No pipeline is selected"));
    }
    let Some(pipeline_id) = Some(pipeline_id)
        .filter(|id| is_bigint_id(id))
        .and_then(|id| id.parse::<i64>().ok())
    else {
        return Ok(PipelineFlowActionResult::fail(
            "The pipeline id is not valid",
        ));
    };
    let name = text(action, "action");
    let env = Env {
        db,
        ctx,
        pipeline_id,
        idempotency_key: if DEDUPED_ACTIONS.contains(&name) {
            ctx.base_idempotency_key.clone()
        } else {
            None
        },
    };
    match handle(name, action, &env).await {
        Ok(result) => Ok(result),
        Err(BusinessError::Exception { message, .. }) => {
            Ok(PipelineFlowActionResult::fail(message))
        }
        Err(error) => Err(error),
    }
}

/// `runPipelineFlowAction`: one Pipelines flow step, attributed to its flow.
pub async fn run_pipeline_flow_action(
    db: &Db,
    step: &Value,
    ctx: &PipelineFlowActionContext,
) -> Result<PipelineFlowActionResult> {
    let step_id = text(step, "id");
    run_pipeline_action(
        db,
        step,
        &PipelineActionContext {
            workspace_id: ctx.workspace_id,
            contact_id: ctx.contact_id,
            actor: PipelineActor {
                kind: "flow".to_string(),
                user_id: None,
                source_id: Some(ctx.flow_id.to_string()),
            },
            base_idempotency_key: ctx
                .flow_execution_key
                .as_ref()
                .filter(|key| !key.is_empty())
                .map(|key| format!("{key}:{step_id}")),
            interpolate: Arc::clone(&ctx.interpolate),
        },
    )
    .await
}
