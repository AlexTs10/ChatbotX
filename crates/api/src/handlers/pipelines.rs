//! `features/pipelines/api/public.ts` + `public-items.ts`: `pipelines.*`.
//! A workspace token acts with `PIPELINE_TOKEN_SCOPE` (bypasses the ACL).

use std::collections::BTreeMap;

use chatbotx_business::pipeline::currency::PipelineView;
use chatbotx_business::pipeline::item::{
    self, CreateItem, ItemFields, ItemView, TransferItem, UpdateItem,
};
use chatbotx_business::pipeline::members;
use chatbotx_business::pipeline::mutation::{PipelineActor, major_units};
use chatbotx_business::pipeline::service::{self, CreatePipeline, UpdatePipeline, stored_settings};
use chatbotx_business::pipeline::settings::{
    CardElementsPatch, PipelineSettingsPatch, StatusLabelsPatch, VisibleFeaturesPatch,
};
use chatbotx_business::pipeline::structure::{
    self, FieldPatch, FieldValues, StageValues, is_color,
};
use chatbotx_business::pipeline::{PipelineAccessScope, PipelinePermissions};
use chatbotx_db::repo::models::{PipelineField, PipelineMember, PipelineStage};
use chatbotx_db::repo::partials::pipeline::{PIPELINE_REFERENCE_NAME_MAX, PipelineType};
use chatbotx_db::repo::pipeline_item_content::AttachmentWithFile;
use serde_json::{Value, json};

use super::custom_fields::CUSTOM_FIELD_TYPES;
use super::input::{Input, Num, Seg, Str, child, key};
use super::{
    ApiState, Call, ErrorMap, FINDING, LISTING, Reply, business_error, date, id, json, no_content,
    number, opt_date, opt_id,
};

/// `PIPELINE_TOKEN_SCOPE`.
const TOKEN: PipelineAccessScope = PipelineAccessScope::WorkspaceToken;

/// `possibleErrorsOnMutatingPipeline`.
pub const MUTATING_PIPELINE: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("stageBusy", 409),
    ("pipelineItemLimitReached", 409),
    ("pipelineItemIdempotencyKeyConflict", 409),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

const PIPELINE_TYPES: &[&str] = &["opportunity", "ticket", "custom"];

fn no_nul(text: &str) -> bool {
    !text.contains('\0')
}

fn is_reference_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn is_currency(text: &str) -> bool {
    text.len() == 3 && text.bytes().all(|b| b.is_ascii_uppercase())
}

/// `nameSchema`.
pub const NAME: Str = Str {
    pattern: Some(no_nul),
    ..Str::trimmed(1, 120)
};
const REFERENCE_NAME: Str = Str {
    trim: false,
    min: None,
    max: Some(PIPELINE_REFERENCE_NAME_MAX),
    pattern: Some(is_reference_name),
};
const CURRENCY: Str = Str {
    trim: false,
    min: None,
    max: None,
    pattern: Some(is_currency),
};
const STAGE_DESCRIPTION: Str = Str {
    pattern: Some(no_nul),
    ..Str::max(300)
};
const COLOR: Str = Str {
    trim: false,
    min: None,
    max: None,
    pattern: Some(is_color),
};
const LABEL: Str = Str::trimmed(1, 40);

/// `toPublicPipeline`.
pub fn pipeline_json(view: &PipelineView) -> Value {
    let pipeline = &view.pipeline;
    json!({
        "id": id(pipeline.id),
        "name": pipeline.name,
        "type": pipeline.r#type,
        "referenceName": pipeline.reference_name,
        "currency": view.currency,
        "currencyOverride": view.currency_override,
        "settings": serde_json::to_value(stored_settings(pipeline)).unwrap_or_default(),
        "createdAt": date(pipeline.created_at),
        "updatedAt": date(pipeline.updated_at),
    })
}

/// `toPublicStage`.
pub fn stage_json(stage: &PipelineStage) -> Value {
    json!({
        "id": id(stage.id),
        "name": stage.name,
        "description": stage.description,
        "textColor": stage.text_color,
        "backgroundColor": stage.background_color,
        "position": stage.position,
        "processing": stage.bulk_job_id.is_some(),
    })
}

/// `toPublicField`.
pub fn field_json(field: &PipelineField) -> Value {
    json!({
        "id": id(field.id),
        "name": field.name,
        "referenceName": field.reference_name,
        "type": field.r#type,
        "required": field.required,
        "position": field.position,
    })
}

fn detail_json(view: &PipelineView, stages: &[PipelineStage], fields: &[PipelineField]) -> Value {
    let mut out = pipeline_json(view);
    out["stages"] = Value::Array(stages.iter().map(stage_json).collect());
    out["fields"] = Value::Array(fields.iter().map(field_json).collect());
    out
}

/// The body of a DELETE (when sent) or its query.
fn delete_input(call: &Call<'_>) -> Input {
    if call.body.iter().all(u8::is_ascii_whitespace) {
        Input::from_query(call.params, call.query)
    } else {
        Input::from_body(call.params, call.body)
    }
}

fn pipeline_type(name: &str) -> PipelineType {
    match name {
        "ticket" => PipelineType::Ticket,
        "custom" => PipelineType::Custom,
        _ => PipelineType::Opportunity,
    }
}

/// `pipelineSettingsPatchSchema`.
fn settings_patch(input: &mut Input) -> Option<PipelineSettingsPatch> {
    let value = input.get("settings").cloned()?;
    let path = key("settings");
    let Value::Object(obj) = value else {
        input.issue(&path, "invalid_type", "Invalid input: expected object");
        return None;
    };
    let mut patch = PipelineSettingsPatch {
        assign_contact_on_assign: input.bool_at(
            &child(&path, "assignContactOnAssign"),
            obj.get("assignContactOnAssign"),
            false,
        ),
        ..PipelineSettingsPatch::default()
    };
    match obj.get("maxOpenItemsPerContact") {
        None => {}
        Some(Value::Null) => patch.max_open_items_per_contact = Some(None),
        Some(value) => {
            patch.max_open_items_per_contact = input
                .number_at(
                    &child(&path, "maxOpenItemsPerContact"),
                    Some(value),
                    Num::int(1.0, 100.0),
                    true,
                )
                .map(|n| Some(n as i64));
        }
    }
    let group = |input: &mut Input, name: &str, keys: &[&str]| -> Vec<Option<bool>> {
        let group_path = child(&path, name);
        match obj.get(name) {
            None => vec![None; keys.len()],
            Some(Value::Object(map)) => keys
                .iter()
                .map(|k| input.bool_at(&child(&group_path, k), map.get(*k), false))
                .collect(),
            Some(_) => {
                input.issue(
                    &group_path,
                    "invalid_type",
                    "Invalid input: expected object",
                );
                vec![None; keys.len()]
            }
        }
    };
    let v = group(input, "visibleFeatures", &["priority", "value", "dueDate"]);
    patch.visible_features = VisibleFeaturesPatch {
        priority: v[0],
        value: v[1],
        due_date: v[2],
    };
    let c = group(
        input,
        "cardElements",
        &[
            "contactPhone",
            "contactEmail",
            "createdAt",
            "updatedAt",
            "assignees",
            "comments",
            "cardId",
            "priority",
            "value",
            "dueDate",
        ],
    );
    patch.card_elements = CardElementsPatch {
        contact_phone: c[0],
        contact_email: c[1],
        created_at: c[2],
        updated_at: c[3],
        assignees: c[4],
        comments: c[5],
        card_id: c[6],
        priority: c[7],
        value: c[8],
        due_date: c[9],
    };
    let labels_path = child(&path, "statusLabels");
    match obj.get("statusLabels") {
        None => {}
        Some(Value::Object(map)) => {
            let label = |input: &mut Input, k: &str| {
                input.string_at(&child(&labels_path, k), map.get(k), LABEL, false)
            };
            patch.status_labels = StatusLabelsPatch {
                open: label(input, "open"),
                won: label(input, "won"),
                lost: label(input, "lost"),
            };
        }
        Some(_) => input.issue(
            &labels_path,
            "invalid_type",
            "Invalid input: expected object",
        ),
    }
    Some(patch)
}

fn nullish_at(input: &mut Input, name: &str, spec: Str) -> Option<Option<String>> {
    input.nullish_string(name, spec)
}

/// `stageValues` (every key optional).
fn stage_values(input: &mut Input) -> StageValues {
    StageValues {
        name: input.opt_string("name", NAME),
        description: nullish_at(input, "description", STAGE_DESCRIPTION),
        text_color: nullish_at(input, "textColor", COLOR),
        background_color: nullish_at(input, "backgroundColor", COLOR),
    }
}

/// `z.array(zodBigintIdAsString()).min(1).max(max)`.
fn id_list(input: &mut Input, name: &str, max: usize) -> Option<Vec<i64>> {
    let path = key(name);
    let value = input.get(name).cloned();
    let Some(Value::Array(items)) = value else {
        input.issue(&path, "invalid_type", "Invalid input: expected array");
        return None;
    };
    if items.is_empty() {
        input.issue(
            &path,
            "too_small",
            "Too small: expected array to have >=1 items",
        );
        return None;
    }
    if items.len() > max {
        input.issue(
            &path,
            "too_big",
            &format!("Too big: expected array to have <={max} items"),
        );
        return None;
    }
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let mut item_path = path.clone();
        item_path.push(Seg::Index(index));
        out.push(input.id_at(&item_path, Some(item), true));
    }
    out.into_iter().collect()
}

fn ok(status: u16, body: Value) -> Reply {
    Ok(json(status, body))
}

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "pipelines.list",
    "pipelines.create",
    "pipelines.get",
    "pipelines.update",
    "pipelines.delete",
    "pipelines.duplicate",
    "pipelines.createStage",
    "pipelines.updateStage",
    "pipelines.deleteStage",
    "pipelines.reorderStages",
    "pipelines.listFields",
    "pipelines.createField",
    "pipelines.updateField",
    "pipelines.deleteField",
    "pipelines.createItem",
    "pipelines.getItem",
    "pipelines.updateItem",
    "pipelines.moveItem",
    "pipelines.transferItem",
    "pipelines.deleteItem",
    "pipelines.listMembers",
    "pipelines.setMember",
    "pipelines.removeMember",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let mutating = |e| business_error(e, MUTATING_PIPELINE);
    match route_id {
        "pipelines.list" => {
            let entries = service::list_with_permissions(db, ws, TOKEN)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let items: Vec<Value> = entries
                .iter()
                .map(|entry| {
                    let mut out = pipeline_json(&entry.pipeline);
                    out["itemsCount"] = json!(entry.items_count);
                    out
                })
                .collect();
            ok(200, json!({ "items": items }))
        }
        "pipelines.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let kind = input.enum_at(
                &key("type"),
                input.get("type").cloned().as_ref(),
                PIPELINE_TYPES,
                true,
            );
            let name = input.string("name", NAME);
            input.finish()?;
            let view = service::create(
                db,
                CreatePipeline {
                    workspace_id: ws,
                    pipeline_type: pipeline_type(kind.as_deref().unwrap_or_default()),
                    name: name.unwrap_or_default(),
                    scope: TOKEN,
                    actor_user_id: None,
                },
            )
            .await
            .map_err(mutating)?;
            let detail = service::get(db, ws, view.pipeline.id, TOKEN)
                .await
                .map_err(mutating)?;
            ok(
                201,
                detail_json(&detail.pipeline, &detail.stages, &detail.fields),
            )
        }
        "pipelines.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let pipeline_id = input.id("id");
            input.finish()?;
            let detail = service::get(db, ws, pipeline_id.unwrap_or_default(), TOKEN)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            ok(
                200,
                detail_json(&detail.pipeline, &detail.stages, &detail.fields),
            )
        }
        "pipelines.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let pipeline_id = input.id("id");
            let name = input.opt_string("name", NAME);
            let reference_name = input.opt_string("referenceName", REFERENCE_NAME);
            let currency = input.nullish_string("currency", CURRENCY);
            let settings = settings_patch(&mut input);
            input.finish()?;
            let view = service::update(
                db,
                UpdatePipeline {
                    workspace_id: ws,
                    id: pipeline_id.unwrap_or_default(),
                    scope: TOKEN,
                    name,
                    reference_name,
                    currency,
                    settings,
                },
            )
            .await
            .map_err(mutating)?;
            ok(200, pipeline_json(&view))
        }
        "pipelines.delete" => {
            let mut input = delete_input(&call);
            let pipeline_id = input.id("id");
            input.finish()?;
            service::delete(db, ws, pipeline_id.unwrap_or_default(), TOKEN)
                .await
                .map_err(mutating)?;
            Ok(no_content())
        }
        "pipelines.duplicate" => {
            let mut input = Input::from_body(call.params, call.body);
            let pipeline_id = input.id("id");
            input.finish()?;
            let view = service::duplicate(db, ws, pipeline_id.unwrap_or_default(), TOKEN, None)
                .await
                .map_err(mutating)?;
            let detail = service::get(db, ws, view.pipeline.id, TOKEN)
                .await
                .map_err(mutating)?;
            ok(
                201,
                detail_json(&detail.pipeline, &detail.stages, &detail.fields),
            )
        }
        "pipelines.createStage" => {
            let mut input = Input::from_body(call.params, call.body);
            let pipeline_id = input.id("id");
            let name = input.string("name", NAME);
            let mut values = stage_values(&mut input);
            values.name = name;
            input.finish()?;
            let stage =
                structure::create_stage(db, ws, pipeline_id.unwrap_or_default(), TOKEN, values)
                    .await
                    .map_err(mutating)?;
            ok(201, stage_json(&stage))
        }
        "pipelines.updateStage" => {
            let mut input = Input::from_body(call.params, call.body);
            let pipeline_id = input.id("id");
            let stage_id = input.id("stageId");
            let values = stage_values(&mut input);
            input.finish()?;
            let stage = structure::update_stage(
                db,
                ws,
                pipeline_id.unwrap_or_default(),
                stage_id.unwrap_or_default(),
                TOKEN,
                values,
            )
            .await
            .map_err(mutating)?;
            ok(200, stage_json(&stage))
        }
        "pipelines.deleteStage" => {
            let mut input = delete_input(&call);
            let pipeline_id = input.id("id");
            let stage_id = input.id("stageId");
            let move_to = match input.get("moveItemsToStageId").cloned() {
                None => None,
                Some(value) => input.id_at(&key("moveItemsToStageId"), Some(&value), true),
            };
            input.finish()?;
            let result = structure::delete_stage(
                db,
                ws,
                pipeline_id.unwrap_or_default(),
                stage_id.unwrap_or_default(),
                move_to,
                TOKEN,
                token_actor(),
            )
            .await
            .map_err(mutating)?;
            ok(
                200,
                json!({ "queued": result.queued, "jobId": result.job_id }),
            )
        }
        "pipelines.reorderStages" => {
            let mut input = Input::from_body(call.params, call.body);
            let pipeline_id = input.id("id");
            let stage_ids = id_list(&mut input, "stageIds", 200);
            input.finish()?;
            structure::reorder_stages(
                db,
                ws,
                pipeline_id.unwrap_or_default(),
                TOKEN,
                stage_ids.unwrap_or_default(),
            )
            .await
            .map_err(mutating)?;
            Ok(no_content())
        }
        "pipelines.listFields" => {
            let mut input = Input::from_query(call.params, call.query);
            let pipeline_id = input.id("id");
            input.finish()?;
            let fields = service::list_fields(db, ws, pipeline_id.unwrap_or_default(), TOKEN)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            ok(
                200,
                json!({ "items": fields.iter().map(field_json).collect::<Vec<_>>() }),
            )
        }
        "pipelines.createField" => {
            let mut input = Input::from_body(call.params, call.body);
            let pipeline_id = input.id("id");
            let name = input.string("name", NAME);
            let reference_name = input.string("referenceName", REFERENCE_NAME);
            let field_type = input.enum_at(
                &key("type"),
                input.get("type").cloned().as_ref(),
                CUSTOM_FIELD_TYPES,
                true,
            );
            let required = input.opt_bool("required");
            input.finish()?;
            let field = structure::create_field(
                db,
                ws,
                pipeline_id.unwrap_or_default(),
                TOKEN,
                FieldValues {
                    name: name.unwrap_or_default(),
                    reference_name: reference_name.unwrap_or_default(),
                    field_type: field_type.unwrap_or_default(),
                    required: required.unwrap_or(false),
                },
            )
            .await
            .map_err(mutating)?;
            ok(201, field_json(&field))
        }
        "pipelines.updateField" => {
            let mut input = Input::from_body(call.params, call.body);
            let pipeline_id = input.id("id");
            let field_id = input.id("fieldId");
            let patch = FieldPatch {
                name: input.opt_string("name", NAME),
                reference_name: input.opt_string("referenceName", REFERENCE_NAME),
                required: input.opt_bool("required"),
            };
            input.finish()?;
            let field = structure::update_field(
                db,
                ws,
                pipeline_id.unwrap_or_default(),
                field_id.unwrap_or_default(),
                TOKEN,
                patch,
            )
            .await
            .map_err(mutating)?;
            ok(200, field_json(&field))
        }
        "pipelines.deleteField" => {
            let mut input = delete_input(&call);
            let pipeline_id = input.id("id");
            let field_id = input.id("fieldId");
            input.finish()?;
            structure::delete_field(
                db,
                ws,
                pipeline_id.unwrap_or_default(),
                field_id.unwrap_or_default(),
                TOKEN,
            )
            .await
            .map_err(mutating)?;
            Ok(no_content())
        }
        "pipelines.createItem"
        | "pipelines.getItem"
        | "pipelines.updateItem"
        | "pipelines.moveItem"
        | "pipelines.transferItem"
        | "pipelines.deleteItem" => handle_item(state, route_id, call).await,
        "pipelines.listMembers" => {
            let mut input = Input::from_query(call.params, call.query);
            let pipeline_id = input.id("id");
            input.finish()?;
            let rows = members::list_members(db, ws, pipeline_id.unwrap_or_default(), TOKEN)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            ok(
                200,
                json!({ "items": rows.iter().map(member_json).collect::<Vec<_>>() }),
            )
        }
        "pipelines.setMember" => {
            let mut input = Input::from_body(call.params, call.body);
            let pipeline_id = input.id("id");
            let member_id = input.id("workspaceMemberId");
            let permissions = PipelinePermissions {
                can_view: input.bool("canView").unwrap_or(false),
                can_edit: input.opt_bool("canEdit").unwrap_or(false),
                can_delete: input.opt_bool("canDelete").unwrap_or(false),
                can_manage_settings: input.opt_bool("canManageSettings").unwrap_or(false),
            };
            input.finish()?;
            let row = members::set_member(
                db,
                ws,
                pipeline_id.unwrap_or_default(),
                member_id.unwrap_or_default(),
                TOKEN,
                permissions,
            )
            .await
            .map_err(mutating)?;
            ok(200, member_json(&row))
        }
        "pipelines.removeMember" => {
            let mut input = delete_input(&call);
            let pipeline_id = input.id("id");
            let member_id = input.id("workspaceMemberId");
            input.finish()?;
            members::remove_member(
                db,
                ws,
                pipeline_id.unwrap_or_default(),
                member_id.unwrap_or_default(),
                TOKEN,
            )
            .await
            .map_err(mutating)?;
            Ok(no_content())
        }
        _ => Err(super::internal_error(route_id)),
    }
}

// --- items ----------------------------------------------------------------

/// `PIPELINE_LIMITS.itemValue` (major units).
const ITEM_VALUE_MAX: f64 = 999_999_999_999_999.0;
const TITLE: Str = Str {
    pattern: Some(no_nul),
    ..Str::trimmed(1, 200)
};
const ITEM_DESCRIPTION: Str = Str {
    pattern: Some(no_nul),
    ..Str::max(5000)
};
const IDEMPOTENCY_KEY: Str = Str {
    trim: false,
    min: Some(1),
    max: Some(128),
    pattern: Some(no_nul),
};
const FIELD_VALUE: Str = Str {
    pattern: Some(no_nul),
    ..Str::max(MAX_FIELD_VALUE_LENGTH)
};
/// `MAX_FIELD_VALUES` / `MAX_FIELD_VALUE_LENGTH`.
const MAX_FIELD_VALUES: usize = 100;
const MAX_FIELD_VALUE_LENGTH: usize = 2000;
const PRIORITIES: &[&str] = &["low", "medium", "high"];
const STATUSES: &[&str] = &["open", "won", "lost"];
/// `boundedDate`: 1900-01-01 .. 2200-01-01.
const DATE_MIN: i64 = -2_208_988_800_000;
const DATE_MAX: i64 = 7_258_118_400_000;

/// Major units -> the stored x 10,000 integer (`storedValue`).
fn stored_value(value: f64) -> i64 {
    (value * 10_000.0).round() as i64
}

fn bounded_date_at(input: &mut Input, name: &str) -> Option<Option<i64>> {
    let path = key(name);
    match input.get(name).cloned() {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => {
            let text = input.string_at(&path, Some(&value), Str::default(), true)?;
            match chatbotx_db::repo::parse_iso(&text) {
                Some(ms) if (DATE_MIN..DATE_MAX).contains(&ms) => Some(Some(ms)),
                Some(_) => {
                    input.issue(&path, "custom", "Date is out of range");
                    None
                }
                None => {
                    input.issue(&path, "invalid_format", "Invalid ISO datetime");
                    None
                }
            }
        }
    }
}

fn opt_id_list(input: &mut Input, name: &str, max: usize) -> Option<Vec<i64>> {
    if !input.has(name) {
        return None;
    }
    let path = key(name);
    let Some(Value::Array(items)) = input.get(name).cloned() else {
        input.issue(&path, "invalid_type", "Invalid input: expected array");
        return None;
    };
    if items.len() > max {
        input.issue(
            &path,
            "too_big",
            &format!("Too big: expected array to have <={max} items"),
        );
        return None;
    }
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let mut item_path = path.clone();
        item_path.push(Seg::Index(index));
        out.push(input.id_at(&item_path, Some(item), true));
    }
    out.into_iter().collect()
}

fn field_values_at(input: &mut Input) -> Option<BTreeMap<String, Option<String>>> {
    let path = key("fieldValues");
    let value = input.get("fieldValues").cloned()?;
    let Value::Object(map) = value else {
        input.issue(&path, "invalid_type", "Invalid input: expected record");
        return None;
    };
    let mut out = BTreeMap::new();
    let mut ok = true;
    for (field_id, raw) in &map {
        let key_path = child(&path, field_id);
        if !super::input::is_bigint_id(field_id) {
            input.issue(&key_path, "invalid_key", "Invalid key in record");
            ok = false;
            continue;
        }
        match raw {
            Value::Null => {
                out.insert(field_id.clone(), None);
            }
            other => match input.string_at(&key_path, Some(other), FIELD_VALUE, true) {
                Some(text) => {
                    out.insert(field_id.clone(), Some(text));
                }
                None => ok = false,
            },
        }
    }
    if map.len() > MAX_FIELD_VALUES {
        input.issue(&path, "custom", "Too many field values");
        ok = false;
    }
    ok.then_some(out)
}

/// `itemWriteFields` (+ `title`).
fn item_fields(input: &mut Input, title_required: bool) -> ItemFields {
    let title = if title_required {
        input.string("title", TITLE)
    } else {
        input.opt_string("title", TITLE)
    };
    let value = match input.get("value").cloned() {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => input
            .number_at(
                &key("value"),
                Some(&v),
                Num::range(0.0, ITEM_VALUE_MAX),
                true,
            )
            .map(|n| Some(stored_value(n))),
    };
    let priority = input.enum_at(
        &key("priority"),
        input.get("priority").cloned().as_ref(),
        PRIORITIES,
        false,
    );
    let status = input.enum_at(
        &key("status"),
        input.get("status").cloned().as_ref(),
        STATUSES,
        false,
    );
    let stage_id = match input.get("stageId").cloned() {
        None => None,
        Some(v) => input.id_at(&key("stageId"), Some(&v), true),
    };
    let timezone = input.opt_string("fieldValuesTimezone", Str::max(64));
    if let Some(zone) = &timezone
        && !chatbotx_core::utils::tz::is_intl_time_zone(zone)
    {
        input.issue(
            &key("fieldValuesTimezone"),
            "custom",
            "Invalid IANA time zone",
        );
    }
    ItemFields {
        title,
        description: input.nullish_string("description", ITEM_DESCRIPTION),
        value,
        status,
        priority,
        due_date: bounded_date_at(input, "dueDate"),
        stage_id,
        field_values: field_values_at(input),
        field_values_timezone: timezone,
        assignee_member_ids: opt_id_list(input, "assigneeMemberIds", 20),
        assignee_inbox_team_ids: opt_id_list(input, "assigneeInboxTeamIds", 20),
    }
}

/// `tokenActor`.
fn token_actor() -> PipelineActor {
    PipelineActor {
        kind: "workspaceToken".to_string(),
        user_id: None,
        source_id: None,
    }
}

fn money(value: Option<i64>) -> Value {
    value.map_or(Value::Null, |v| number(major_units(v)))
}

/// `toPublicItem`.
pub fn item_json(view: &ItemView) -> Value {
    let item = &view.item;
    json!({
        "id": id(item.id),
        "pipelineId": id(item.pipeline_id),
        "stageId": id(item.stage_id),
        "contactId": id(item.contact_id),
        "contactName": view.contact.as_ref().and_then(|c| c.full_name.clone()),
        "number": item.number,
        "title": item.title,
        "description": item.description,
        "value": money(item.value),
        "status": item.status,
        "priority": item.priority,
        "dueDate": opt_date(item.due_date),
        "fieldValues": item.field_values.0,
        "assigneeMemberIds": view.assignees.workspace_member_ids.iter().map(|v| id(*v)).collect::<Vec<_>>(),
        "assigneeInboxTeamIds": view.assignees.inbox_team_ids.iter().map(|v| id(*v)).collect::<Vec<_>>(),
        "commentCount": view.comment_count,
        "createdById": opt_id(item.created_by_id),
        "stageEnteredAt": date(item.stage_entered_at),
        "closedAt": opt_date(item.closed_at),
        "lastCommentAt": opt_date(item.last_comment_at),
        "createdAt": date(item.created_at),
        "updatedAt": date(item.updated_at),
    })
}

/// `toPublicAttachment` (attachments whose file is gone are skipped).
fn attachment_json(row: &AttachmentWithFile) -> Option<Value> {
    let file = row.media_library_file.as_ref()?;
    Some(json!({
        "mediaLibraryFileId": id(file.id),
        "name": file.name,
        "mimeType": file.mime_type,
        "size": file.size,
        "path": file.path,
    }))
}

/// `readItem`: the item as `getItem` shows it, without attachments.
async fn read_item(state: &ApiState, ws: i64, pipeline_id: i64, item_id: i64) -> Reply {
    let detail = item::get_item(&state.db, ws, pipeline_id, item_id, TOKEN)
        .await
        .map_err(|e| business_error(e, MUTATING_PIPELINE))?;
    ok(200, item_json(&detail.view))
}

async fn handle_item(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let mutating = |e| business_error(e, MUTATING_PIPELINE);
    match route_id {
        "pipelines.createItem" => {
            let mut input = Input::from_body(call.params, call.body);
            let pipeline_id = input.id("id");
            let contact_id = input.id("contactId");
            let fields = item_fields(&mut input, true);
            let files = opt_id_list(&mut input, "mediaLibraryFileIds", 20);
            let idempotency_key = input.opt_string("idempotencyKey", IDEMPOTENCY_KEY);
            input.finish()?;
            let pipeline_id = pipeline_id.unwrap_or_default();
            let created = item::create(
                db,
                CreateItem {
                    workspace_id: ws,
                    pipeline_id,
                    contact_id: contact_id.unwrap_or_default(),
                    scope: TOKEN,
                    actor: token_actor(),
                    fields,
                    media_library_file_ids: files.unwrap_or_default(),
                    idempotency_key,
                },
            )
            .await
            .map_err(mutating)?;
            let mut out = read_item(state, ws, pipeline_id, created.value.id).await?;
            *out.status_mut() = axum::http::StatusCode::CREATED;
            Ok(out)
        }
        "pipelines.getItem" => {
            let mut input = Input::from_query(call.params, call.query);
            let pipeline_id = input.id("id");
            let item_id = input.id("itemId");
            input.finish()?;
            let detail = item::get_item(
                db,
                ws,
                pipeline_id.unwrap_or_default(),
                item_id.unwrap_or_default(),
                TOKEN,
            )
            .await
            .map_err(|e| business_error(e, FINDING))?;
            let mut out = item_json(&detail.view);
            out["attachments"] = Value::Array(
                detail
                    .attachments
                    .iter()
                    .filter_map(attachment_json)
                    .collect(),
            );
            ok(200, out)
        }
        "pipelines.updateItem" | "pipelines.moveItem" => {
            let mut input = Input::from_body(call.params, call.body);
            let pipeline_id = input.id("id").unwrap_or_default();
            let item_id = input.id("itemId").unwrap_or_default();
            let fields = if route_id == "pipelines.moveItem" {
                ItemFields {
                    stage_id: input.id("stageId"),
                    ..ItemFields::default()
                }
            } else {
                item_fields(&mut input, false)
            };
            input.finish()?;
            item::update(
                db,
                UpdateItem {
                    workspace_id: ws,
                    pipeline_id,
                    item_id,
                    scope: TOKEN,
                    actor: token_actor(),
                    fields,
                },
            )
            .await
            .map_err(mutating)?;
            read_item(state, ws, pipeline_id, item_id).await
        }
        "pipelines.transferItem" => {
            let mut input = Input::from_body(call.params, call.body);
            let pipeline_id = input.id("id").unwrap_or_default();
            let item_id = input.id("itemId").unwrap_or_default();
            let destination = input.id("destinationPipelineId").unwrap_or_default();
            let destination_stage_id = match input.get("destinationStageId").cloned() {
                None => None,
                Some(v) => input.id_at(&key("destinationStageId"), Some(&v), true),
            };
            input.finish()?;
            let moved = item::transfer(
                db,
                TransferItem {
                    workspace_id: ws,
                    pipeline_id,
                    item_id,
                    destination_pipeline_id: destination,
                    destination_stage_id,
                    scope: TOKEN,
                    actor: token_actor(),
                },
            )
            .await
            .map_err(mutating)?;
            read_item(state, ws, moved.value.pipeline_id, moved.value.id).await
        }
        "pipelines.deleteItem" => {
            let mut input = delete_input(&call);
            let pipeline_id = input.id("id").unwrap_or_default();
            let item_id = input.id("itemId").unwrap_or_default();
            input.finish()?;
            item::delete(db, ws, pipeline_id, item_id, TOKEN, token_actor())
                .await
                .map_err(mutating)?;
            Ok(no_content())
        }
        _ => Err(super::internal_error(route_id)),
    }
}

/// `toPublicMember`.
fn member_json(row: &PipelineMember) -> Value {
    json!({
        "workspaceMemberId": id(row.workspace_member_id),
        "canView": row.can_view,
        "canEdit": row.can_edit,
        "canDelete": row.can_delete,
        "canManageSettings": row.can_manage_settings,
    })
}
