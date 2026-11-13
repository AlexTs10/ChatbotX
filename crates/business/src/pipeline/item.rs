//! `pipeline/item.service.ts`, `item-base.service.ts`, `item-fields.ts` and
//! the item reads of `item-query.service.ts` (`getItem`, `getItemView`).
//! `value` is the stored x 10,000 integer; dates are unix ms.

use std::collections::{BTreeMap, HashMap, HashSet};

use chatbotx_db::Db;
use chatbotx_db::repo::models::{PipelineField, PipelineItem, PipelineStage};
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::repo::pipeline_item::{self as item_repo, PipelineItemInsert};
use chatbotx_db::repo::pipeline_item_content::{self as content_repo, AttachmentWithFile};
use chatbotx_db::repo::pipeline_reference::{self as reference_repo, ContactSummary};
use chatbotx_db::repo::{
    Json, now_ms, pipeline as pipeline_repo, pipeline_activity, pipeline_stage as stage_repo,
};
use chatbotx_db::rusqlite::Connection;
use chatbotx_db::rusqlite::types::Value;
use serde_json::json;

use super::access::{PipelineAccessScope, PipelineAction};
use super::assignees::{self, Assignees};
use super::currency::{PipelineView, view_of};
use super::events::PipelineItemChanged;
use super::mutation::{
    ActivityContext, EventSubject, ItemSnapshot, PipelineActor, PipelineEffect, activity_row_for,
    activity_rows_from_diff, created_effects, diff_pipeline_item, effects_from_diff, is_empty_diff,
    item_realtime, pipeline_event_metadata, realtime_events,
};
use super::service::{require_access, require_access_in, stored_settings};
use crate::contact_custom_field::normalize_value_for_storage;
use crate::errors::{BusinessError, Result};
use crate::tx;

const ITEM_NOT_FOUND: &str = "Item not found";

fn is_open(status: &str) -> bool {
    status == "open"
}

fn stage_names_of(stages: &[PipelineStage]) -> HashMap<i64, String> {
    stages.iter().map(|s| (s.id, s.name.clone())).collect()
}

/// `stageById`.
fn stage_by_id(stages: &[PipelineStage], stage_id: i64) -> Result<PipelineStage> {
    stages
        .iter()
        .find(|s| s.id == stage_id)
        .cloned()
        .ok_or_else(|| BusinessError::not_found("Stage not found"))
}

/// `assertStageNotBulkOwned`.
fn assert_stage_not_bulk_owned(stage: &PipelineStage) -> Result<()> {
    if stage.bulk_job_id.is_some() {
        return Err(BusinessError::conflict(
            "This stage is being updated. Try again shortly.",
            "stageBusy",
        ));
    }
    Ok(())
}

/// `resolveStage` + `lockStage`.
fn resolve_stage(stages: &[PipelineStage], stage_id: Option<i64>) -> Result<PipelineStage> {
    let stage = match stage_id {
        Some(id) => stage_by_id(stages, id)?,
        None => stages
            .first()
            .cloned()
            .ok_or_else(|| BusinessError::validation("stageId", "The pipeline has no stage"))?,
    };
    assert_stage_not_bulk_owned(&stage)?;
    Ok(stage)
}

/// `lockItemOrFail`.
fn lock_item(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    item_id: i64,
) -> Result<PipelineItem> {
    match item_repo::lock_for_update(conn, workspace_id, item_id)? {
        Some(item) if item.pipeline_id == pipeline_id => Ok(item),
        _ => Err(BusinessError::not_found(ITEM_NOT_FOUND)),
    }
}

fn assert_contact_exists(conn: &Connection, workspace_id: i64, contact_id: i64) -> Result<()> {
    if !reference_repo::contact_exists(conn, workspace_id, contact_id)? {
        return Err(BusinessError::not_found("Contact not found"));
    }
    Ok(())
}

fn assert_media_files(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<()> {
    let unique: Vec<i64> = ids
        .iter()
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if reference_repo::list_media_library_file_ids(conn, workspace_id, &unique)?.len()
        != unique.len()
    {
        return Err(BusinessError::not_found("Attachment file not found"));
    }
    Ok(())
}

/// `assertOpenLimit`.
fn assert_open_limit(
    conn: &Connection,
    pipeline: &PipelineView,
    contact_id: i64,
    except_item_id: Option<i64>,
) -> Result<()> {
    let Some(max) = stored_settings(&pipeline.pipeline).max_open_items_per_contact else {
        return Ok(());
    };
    let others = item_repo::count_open_for_contact(
        conn,
        pipeline.pipeline.workspace_id,
        pipeline.pipeline.id,
        contact_id,
        except_item_id,
    )?;
    if others >= max {
        return Err(BusinessError::conflict(
            format!("This contact already has {max} open item(s) in this pipeline"),
            "pipelineItemLimitReached",
        ));
    }
    Ok(())
}

fn string_values(json: &Json) -> BTreeMap<String, String> {
    match &json.0 {
        serde_json::Value::Object(map) => map
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
            .collect(),
        _ => BTreeMap::new(),
    }
}

fn values_json(values: &BTreeMap<String, String>) -> Json {
    Json(json!(values))
}

/// `lockFieldsOrFail`.
fn lock_fields(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    field_ids: &[String],
) -> Result<()> {
    let ids: Vec<i64> = field_ids.iter().filter_map(|id| id.parse().ok()).collect();
    let existing = pipeline_repo::lock_fields_shared(conn, workspace_id, pipeline_id, &ids)?;
    if let Some(missing) = field_ids
        .iter()
        .find(|id| !id.parse::<i64>().is_ok_and(|n| existing.contains(&n)))
    {
        return Err(BusinessError::validation(
            "fieldValues",
            format!("Unknown field {missing}"),
        ));
    }
    Ok(())
}

/// `mergeFieldValues`: a null or empty value removes the key.
#[allow(clippy::too_many_arguments)]
fn merge_field_values(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    contact_id: i64,
    definitions: &[PipelineField],
    current: BTreeMap<String, String>,
    patch: &BTreeMap<String, Option<String>>,
    explicit_timezone: Option<&str>,
) -> Result<BTreeMap<String, String>> {
    let by_id: HashMap<String, &PipelineField> =
        definitions.iter().map(|f| (f.id.to_string(), f)).collect();
    if let Some(unknown) = patch.keys().find(|id| !by_id.contains_key(*id)) {
        return Err(BusinessError::validation(
            "fieldValues",
            format!("Unknown field {unknown}"),
        ));
    }
    let filled: Vec<String> = patch
        .iter()
        .filter(|(_, raw)| raw.as_deref().is_some_and(|r| !r.is_empty()))
        .map(|(id, _)| id.clone())
        .collect();
    lock_fields(conn, workspace_id, pipeline_id, &filled)?;
    let mut merged = current;
    let now = now_ms();
    for (field_id, raw) in patch {
        let definition = by_id[field_id];
        let Some(raw) = raw.as_deref().filter(|r| !r.is_empty()) else {
            merged.remove(field_id);
            continue;
        };
        let invalid = || {
            BusinessError::validation(
                "fieldValues",
                format!("Invalid value for {}", definition.name),
            )
        };
        let normalized = match normalize_value_for_storage(
            conn,
            workspace_id,
            contact_id,
            &definition.r#type,
            raw,
            explicit_timezone,
            now,
        ) {
            Ok(Some(value)) => value,
            Ok(None) => return Err(invalid()),
            Err(BusinessError::Repo(error)) => return Err(BusinessError::Repo(error)),
            Err(_) => return Err(invalid()),
        };
        merged.insert(field_id.clone(), normalized);
    }
    Ok(merged)
}

/// `assertRequiredFields`.
fn assert_required_fields(
    definitions: &[PipelineField],
    values: &BTreeMap<String, String>,
    only: Option<&[String]>,
) -> Result<()> {
    let missing = definitions.iter().find(|field| {
        let id = field.id.to_string();
        field.required
            && values.get(&id).is_none_or(String::is_empty)
            && only.is_none_or(|ids| ids.contains(&id))
    });
    if let Some(field) = missing {
        return Err(BusinessError::validation(
            "fieldValues",
            format!("{} is required", field.name),
        ));
    }
    Ok(())
}

/// `remapFieldValues`: by reference name and type.
fn remap_field_values(
    source: &[PipelineField],
    destination: &[PipelineField],
    values: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for field in source {
        let Some(value) = values.get(&field.id.to_string()) else {
            continue;
        };
        if let Some(target) = destination
            .iter()
            .find(|c| c.reference_name == field.reference_name && c.r#type == field.r#type)
        {
            out.insert(target.id.to_string(), value.clone());
        }
    }
    out
}

fn opt_text(value: Option<String>) -> Value {
    value.map_or(Value::Null, Value::Text)
}

fn opt_int(value: Option<i64>) -> Value {
    value.map_or(Value::Null, Value::Integer)
}

/// `statusBookkeeping`.
fn status_bookkeeping(patch: &mut Patch, status: &str, now: i64, actor: &PipelineActor) {
    patch.push(("statusChangedAt", Value::Integer(now)));
    if is_open(status) {
        patch.push(("closedAt", Value::Null));
        patch.push(("closedById", Value::Null));
    } else {
        patch.push(("closedAt", Value::Integer(now)));
        patch.push(("closedById", opt_int(actor.user_id)));
    }
}

fn subject<'a>(
    pipeline: &'a PipelineView,
    stage: &'a PipelineStage,
    item: &'a PipelineItem,
    actor: &'a PipelineActor,
    names: &'a HashMap<i64, String>,
) -> EventSubject<'a> {
    EventSubject {
        pipeline,
        stage,
        item,
        actor,
        stage_names: names,
    }
}

fn done<T>(value: T, effects: &[PipelineEffect]) -> ItemMutation<T> {
    ItemMutation {
        value,
        events: realtime_events(effects),
    }
}

/// The item write fields (`PipelineItemWriteFields`); `Some(None)` clears.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemFields {
    pub title: Option<String>,
    pub description: Option<Option<String>>,
    pub value: Option<Option<i64>>,
    pub status: Option<String>,
    pub priority: Option<String>,
    pub due_date: Option<Option<i64>>,
    pub stage_id: Option<i64>,
    pub field_values: Option<BTreeMap<String, Option<String>>>,
    pub field_values_timezone: Option<String>,
    pub assignee_member_ids: Option<Vec<i64>>,
    pub assignee_inbox_team_ids: Option<Vec<i64>>,
}

/// `CreatePipelineItemInput`.
#[derive(Debug, Clone)]
pub struct CreateItem {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub contact_id: i64,
    pub scope: PipelineAccessScope,
    pub actor: PipelineActor,
    pub fields: ItemFields,
    pub media_library_file_ids: Vec<i64>,
    pub idempotency_key: Option<String>,
}

/// `UpdatePipelineItemInput` (`move` = a `stage_id`-only update).
#[derive(Debug, Clone)]
pub struct UpdateItem {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub item_id: i64,
    pub scope: PipelineAccessScope,
    pub actor: PipelineActor,
    pub fields: ItemFields,
}

/// The `transfer` input.
#[derive(Debug, Clone)]
pub struct TransferItem {
    pub workspace_id: i64,
    pub pipeline_id: i64,
    pub item_id: i64,
    pub destination_pipeline_id: i64,
    pub destination_stage_id: Option<i64>,
    pub scope: PipelineAccessScope,
    pub actor: PipelineActor,
}

/// A write result with the realtime events it queued.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemMutation<T> {
    pub value: T,
    pub events: Vec<PipelineItemChanged>,
}

/// `PipelineItemView`.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemView {
    pub item: PipelineItem,
    pub contact: Option<ContactSummary>,
    pub assignees: Assignees,
    pub comment_count: i64,
}

/// `getItem`: the view and its attachments.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemDetail {
    pub view: ItemView,
    pub attachments: Vec<AttachmentWithFile>,
}

/// `pipelineItemService.create`.
pub async fn create(db: &Db, input: CreateItem) -> Result<ItemMutation<PipelineItem>> {
    let (pipeline, _) = require_access(
        db,
        input.workspace_id,
        input.pipeline_id,
        input.scope,
        PipelineAction::Edit,
    )
    .await?;
    let pipeline = view_of(db, input.workspace_id, pipeline).await?;
    let (row, effects) = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let CreateItem {
            workspace_id,
            pipeline_id,
            contact_id,
            actor,
            fields,
            media_library_file_ids,
            idempotency_key,
            ..
        } = input;
        if let Some(key) = &idempotency_key
            && let Some(existing) =
                item_repo::find_by_idempotency_key(conn, workspace_id, pipeline_id, key)?
        {
            if existing.contact_id != contact_id {
                return Err(idempotency_conflict());
            }
            return Ok((existing, Vec::new()));
        }
        let status = fields.status.clone().unwrap_or_else(|| "open".to_string());
        let stages = stage_repo::list_by_pipeline(conn, workspace_id, pipeline_id)?;
        let stage = resolve_stage(&stages, fields.stage_id)?;
        assert_contact_exists(conn, workspace_id, contact_id)?;
        let sets = assignees::assignees_from_input(
            fields.assignee_member_ids.as_deref(),
            fields.assignee_inbox_team_ids.as_deref(),
        )?;
        assignees::assert_assignees_exist(conn, workspace_id, &sets)?;
        assert_media_files(conn, workspace_id, &media_library_file_ids)?;
        let definitions = pipeline_repo::list_fields(conn, workspace_id, pipeline_id)?;
        let field_values = merge_field_values(
            conn,
            workspace_id,
            pipeline_id,
            contact_id,
            &definitions,
            BTreeMap::new(),
            &fields.field_values.clone().unwrap_or_default(),
            fields.field_values_timezone.as_deref(),
        )?;
        assert_required_fields(&definitions, &field_values, None)?;
        if is_open(&status) {
            assert_open_limit(conn, &pipeline, contact_id, None)?;
        }
        let now = now_ms();
        let open = is_open(&status);
        let inserted = item_repo::insert_idempotent(
            conn,
            &PipelineItemInsert {
                workspace_id,
                pipeline_id,
                stage_id: stage.id,
                contact_id,
                number: pipeline_repo::next_item_number(conn, workspace_id)?,
                title: fields.title.clone().unwrap_or_default(),
                description: fields.description.clone().flatten(),
                value: fields.value.flatten(),
                status: status.clone(),
                priority: fields
                    .priority
                    .clone()
                    .unwrap_or_else(|| "medium".to_string()),
                due_date: fields.due_date.flatten(),
                field_values: values_json(&field_values),
                created_by_id: actor.user_id,
                closed_by_id: if open { None } else { actor.user_id },
                stage_entered_at: now,
                status_changed_at: None,
                closed_at: if open { None } else { Some(now) },
                last_comment_at: None,
                idempotency_key: idempotency_key.clone(),
            },
        )?;
        let row = inserted.row;
        if !inserted.inserted {
            if row.contact_id != contact_id {
                return Err(idempotency_conflict());
            }
            return Ok((row, Vec::new()));
        }
        item_repo::replace_assignees(
            conn,
            workspace_id,
            row.id,
            &sets.workspace_member_ids,
            &sets.inbox_team_ids,
        )?;
        content_repo::insert_attachments(conn, workspace_id, row.id, &media_library_file_ids)?;
        let context = ActivityContext {
            item: &row,
            actor: &actor,
            now,
            from_pipeline_id: None,
        };
        let mut created = activity_row_for("created", &context);
        created.to_value = row.value;
        created.to_stage_id = Some(row.stage_id);
        created.to_status = Some(row.status.clone());
        pipeline_activity::insert_many(conn, &[created])?;
        let names = stage_names_of(&stages);
        let effects = created_effects(&subject(&pipeline, &stage, &row, &actor, &names));
        Ok((row, effects))
    })
    .await?;
    Ok(done(row, &effects))
}

/// `pipelineItemService.update` / `move`.
pub async fn update(db: &Db, input: UpdateItem) -> Result<ItemMutation<PipelineItem>> {
    let (pipeline, _) = require_access(
        db,
        input.workspace_id,
        input.pipeline_id,
        input.scope,
        PipelineAction::Edit,
    )
    .await?;
    let pipeline = view_of(db, input.workspace_id, pipeline).await?;
    let (row, effects) = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let UpdateItem {
            workspace_id,
            pipeline_id,
            item_id,
            actor,
            fields,
            ..
        } = input;
        let before = lock_item(conn, workspace_id, pipeline_id, item_id)?;
        let reopens = fields.status.as_deref() == Some("open") && !is_open(&before.status);
        let stages = stage_repo::list_by_pipeline(conn, workspace_id, pipeline_id)?;
        let current = stage_by_id(&stages, before.stage_id)?;
        let stage = match fields.stage_id {
            Some(id) if id != before.stage_id => {
                let target = resolve_stage(&stages, Some(id))?;
                assert_stage_not_bulk_owned(&current)?;
                target
            }
            _ => current,
        };
        let assignee_patched =
            fields.assignee_member_ids.is_some() || fields.assignee_inbox_team_ids.is_some();
        let assignees_before = if assignee_patched {
            let before_sets = assignees::current_in(conn, workspace_id, item_id)?;
            assert_contact_exists(conn, workspace_id, before.contact_id)?;
            let sets = assignees::assignees_from_input(
                fields.assignee_member_ids.as_deref(),
                fields.assignee_inbox_team_ids.as_deref(),
            )?;
            assignees::assert_assignees_exist(conn, workspace_id, &sets)?;
            Some(before_sets)
        } else {
            None
        };
        let before_values = string_values(&before.field_values);
        let field_values = match &fields.field_values {
            Some(patch) => {
                let definitions = pipeline_repo::list_fields(conn, workspace_id, pipeline_id)?;
                let merged = merge_field_values(
                    conn,
                    workspace_id,
                    pipeline_id,
                    before.contact_id,
                    &definitions,
                    before_values.clone(),
                    patch,
                    fields.field_values_timezone.as_deref(),
                )?;
                let keys: Vec<String> = patch.keys().cloned().collect();
                assert_required_fields(&definitions, &merged, Some(&keys))?;
                merged
            }
            None => before_values,
        };
        let status = fields
            .status
            .clone()
            .unwrap_or_else(|| before.status.clone());
        if reopens {
            assert_open_limit(conn, &pipeline, before.contact_id, Some(before.id))?;
        }
        let now = now_ms();
        let next = ItemSnapshot {
            stage_id: stage.id,
            status: status.clone(),
            priority: fields
                .priority
                .clone()
                .unwrap_or_else(|| before.priority.clone()),
            value: fields.value.unwrap_or(before.value),
            due_date: fields.due_date.unwrap_or(before.due_date),
            title: fields.title.clone().unwrap_or_else(|| before.title.clone()),
            description: fields
                .description
                .clone()
                .unwrap_or_else(|| before.description.clone()),
            field_values,
        };
        let diff = diff_pipeline_item(&ItemSnapshot::of(&before), &next);
        let mut row = if is_empty_diff(&diff) {
            before.clone()
        } else {
            let mut patch: Patch = vec![
                ("title", Value::Text(next.title.clone())),
                ("description", opt_text(next.description.clone())),
                ("value", opt_int(next.value)),
                ("priority", Value::Text(next.priority.clone())),
                ("dueDate", opt_int(next.due_date)),
                ("status", Value::Text(next.status.clone())),
                ("stageId", Value::Integer(next.stage_id)),
                (
                    "fieldValues",
                    Value::Text(values_json(&next.field_values).0.to_string()),
                ),
            ];
            if diff.stage.is_some() {
                patch.push(("stageEnteredAt", Value::Integer(now)));
            }
            if diff.status.is_some() {
                status_bookkeeping(&mut patch, &status, now, &actor);
            }
            item_repo::update(conn, workspace_id, item_id, &patch)?
                .ok_or_else(|| BusinessError::not_found(ITEM_NOT_FOUND))?
        };
        let mut activity = activity_rows_from_diff(
            &diff,
            &ActivityContext {
                item: &row,
                actor: &actor,
                now,
                from_pipeline_id: None,
            },
        );
        let assignees_changed = match &assignees_before {
            Some(before_sets) => assignees::apply_in(
                conn,
                workspace_id,
                item_id,
                fields.assignee_member_ids.as_deref(),
                fields.assignee_inbox_team_ids.as_deref(),
                before_sets,
            )?,
            None => false,
        };
        if assignees_changed {
            if is_empty_diff(&diff) {
                row = item_repo::touch(conn, workspace_id, item_id)?.unwrap_or(row);
            }
            activity.push(activity_row_for(
                "assigneesChanged",
                &ActivityContext {
                    item: &row,
                    actor: &actor,
                    now,
                    from_pipeline_id: None,
                },
            ));
        }
        pipeline_activity::insert_many(conn, &activity)?;
        let names = stage_names_of(&stages);
        let mut effects =
            effects_from_diff(&diff, &subject(&pipeline, &stage, &row, &actor, &names));
        if assignees_changed && diff.stage.is_none() {
            effects.push(item_realtime(&row, "updated"));
        }
        Ok((row, effects))
    })
    .await?;
    Ok(done(row, &effects))
}

/// `pipelineItemService.transfer`.
pub async fn transfer(db: &Db, input: TransferItem) -> Result<ItemMutation<PipelineItem>> {
    if input.destination_pipeline_id == input.pipeline_id {
        return Err(BusinessError::validation(
            "destinationPipelineId",
            "Choose a different pipeline",
        ));
    }
    require_access(
        db,
        input.workspace_id,
        input.pipeline_id,
        input.scope,
        PipelineAction::Edit,
    )
    .await?;
    let (destination, _) = require_access(
        db,
        input.workspace_id,
        input.destination_pipeline_id,
        input.scope,
        PipelineAction::Edit,
    )
    .await?;
    let destination = view_of(db, input.workspace_id, destination).await?;
    let (row, effects) = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let TransferItem {
            workspace_id,
            pipeline_id,
            item_id,
            destination_stage_id,
            actor,
            ..
        } = input;
        let before = lock_item(conn, workspace_id, pipeline_id, item_id)?;
        let destination_id = destination.pipeline.id;
        let source_stages = stage_repo::list_by_pipeline(conn, workspace_id, pipeline_id)?;
        let destination_stages = stage_repo::list_by_pipeline(conn, workspace_id, destination_id)?;
        let source_fields = pipeline_repo::list_fields(conn, workspace_id, pipeline_id)?;
        let destination_fields = pipeline_repo::list_fields(conn, workspace_id, destination_id)?;
        assert_stage_not_bulk_owned(&stage_by_id(&source_stages, before.stage_id)?)?;
        let stage = resolve_stage(&destination_stages, destination_stage_id)?;
        let field_values = remap_field_values(
            &source_fields,
            &destination_fields,
            &string_values(&before.field_values),
        );
        let keys: Vec<String> = field_values.keys().cloned().collect();
        lock_fields(conn, workspace_id, destination_id, &keys)?;
        if is_open(&before.status) {
            assert_open_limit(conn, &destination, before.contact_id, None)?;
        }
        let now = now_ms();
        let patch: Patch = vec![
            ("pipelineId", Value::Integer(destination_id)),
            ("stageId", Value::Integer(stage.id)),
            ("stageEnteredAt", Value::Integer(now)),
            (
                "fieldValues",
                Value::Text(values_json(&field_values).0.to_string()),
            ),
            ("idempotencyKey", Value::Null),
        ];
        let row = item_repo::update(conn, workspace_id, item_id, &patch)?
            .ok_or_else(|| BusinessError::not_found(ITEM_NOT_FOUND))?;
        let mut activity = activity_row_for(
            "transferred",
            &ActivityContext {
                item: &row,
                actor: &actor,
                now,
                from_pipeline_id: Some(pipeline_id),
            },
        );
        activity.from_stage_id = Some(before.stage_id);
        activity.to_stage_id = Some(stage.id);
        pipeline_activity::insert_many(conn, &[activity])?;
        let names = stage_names_of(&source_stages);
        let metadata = pipeline_event_metadata(
            &subject(&destination, &stage, &row, &actor, &names),
            json!({
                "fromStageId": before.stage_id.to_string(),
                "fromStageName": names.get(&before.stage_id),
                "fromPipelineId": pipeline_id.to_string(),
            }),
        );
        let effects = vec![
            PipelineEffect::Event {
                event_type: "pipelineItemMovedToStage",
                workspace_id,
                contact_id: row.contact_id,
                metadata,
            },
            super::mutation::realtime_effect(
                workspace_id,
                pipeline_id,
                Some((item_id, row.contact_id)),
                "deleted",
            ),
            item_realtime(&row, "created"),
        ];
        Ok((row, effects))
    })
    .await?;
    Ok(done(row, &effects))
}

/// `pipelineItemService.delete` (the activity history stays).
pub async fn delete(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    item_id: i64,
    scope: PipelineAccessScope,
    actor: PipelineActor,
) -> Result<ItemMutation<()>> {
    require_access(db, workspace_id, pipeline_id, scope, PipelineAction::Delete).await?;
    let effects = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let item = lock_item(conn, workspace_id, pipeline_id, item_id)?;
        item_repo::delete(conn, workspace_id, item_id)?;
        let mut row = activity_row_for(
            "deleted",
            &ActivityContext {
                item: &item,
                actor: &actor,
                now: now_ms(),
                from_pipeline_id: None,
            },
        );
        row.from_stage_id = Some(item.stage_id);
        pipeline_activity::insert_many(conn, &[row])?;
        Ok(vec![item_realtime(&item, "deleted")])
    })
    .await?;
    Ok(done((), &effects))
}

/// `pipelineItemQueryService.getItem` (without `conversationId`).
pub async fn get_item(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    item_id: i64,
    scope: PipelineAccessScope,
) -> Result<ItemDetail> {
    tx::read(db, move |conn| {
        require_access_in(
            conn,
            workspace_id,
            pipeline_id,
            &scope,
            PipelineAction::View,
        )?;
        let item = item_repo::find_by_id(conn, workspace_id, item_id, Some(pipeline_id))?
            .ok_or_else(|| BusinessError::not_found(ITEM_NOT_FOUND))?;
        let view = views_in(conn, workspace_id, vec![item], &scope)?
            .pop()
            .ok_or_else(|| BusinessError::not_found(ITEM_NOT_FOUND))?;
        let attachments = content_repo::list_attachments(conn, workspace_id, item_id)?;
        Ok(ItemDetail { view, attachments })
    })
    .await
}

fn idempotency_conflict() -> BusinessError {
    BusinessError::conflict(
        "This idempotency key was already used for another contact",
        "pipelineItemIdempotencyKeyConflict",
    )
}

/// `toViews`: assignees, comment counts and contacts (email and phone masked
/// for a scope without the contact permission).
pub fn views_in(
    conn: &Connection,
    workspace_id: i64,
    items: Vec<PipelineItem>,
    scope: &PipelineAccessScope,
) -> Result<Vec<ItemView>> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let item_ids: Vec<i64> = items.iter().map(|i| i.id).collect();
    let mut contact_ids: Vec<i64> = items.iter().map(|i| i.contact_id).collect();
    contact_ids.sort_unstable();
    contact_ids.dedup();
    let assignee_rows = item_repo::list_assignees_for_items(conn, workspace_id, &item_ids)?;
    let counts = content_repo::count_comments_for_items(conn, workspace_id, &item_ids)?;
    let show_pii = super::access::can_view_contact_email_and_phone(scope);
    let contacts: HashMap<i64, ContactSummary> =
        reference_repo::list_contact_summaries(conn, workspace_id, &contact_ids)?
            .into_iter()
            .map(|mut c| {
                if !show_pii {
                    c.email = None;
                    c.phone_number = None;
                }
                (c.id, c)
            })
            .collect();
    let mut grouped = assignees::group_assignees(&assignee_rows);
    let counts: HashMap<i64, i64> = counts.iter().map(|c| (c.item_id, c.count)).collect();
    Ok(items
        .into_iter()
        .map(|item| ItemView {
            contact: contacts.get(&item.contact_id).cloned(),
            assignees: grouped.remove(&item.id).unwrap_or_default(),
            comment_count: counts.get(&item.id).copied().unwrap_or(0),
            item,
        })
        .collect())
}
