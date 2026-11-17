//! The pipeline board of the SPA: `features/pipelines/api/private.ts`
//! (`privateGetBoardAPI`, `privateListStagePageAPI`, `privateGetItemAPI`,
//! `privateListItemCommentsAPI`, `privateListItemActivityAPI`,
//! `privateListPipelineMembersAPI`, `privateGetStatsAPI`,
//! `privateListContactItemsAPI`) and the stage / item / field / member actions
//! (`actions/{stage,item,field-member}.action.ts`). The member scope and the
//! user as the actor (`pipelineActionCaller`); the inputs are the public
//! request schemas (`id` = the pipeline), the errors the services' own.

use std::collections::BTreeMap;

use chatbotx_business::pipeline::content::{self, ActivityView, ItemRef};
use chatbotx_business::pipeline::item::{
    self, CreateItem, ItemFields, ItemMutation, ItemView, TransferItem, UpdateItem,
};
use chatbotx_business::pipeline::mutation::PipelineActor;
use chatbotx_business::pipeline::query::{self as item_query, ItemListQuery};
use chatbotx_business::pipeline::structure::{self, FieldPatch, FieldValues};
use chatbotx_business::pipeline::{PipelinePermissions, analytics, members};
use chatbotx_db::repo::models::PipelineItem;
use chatbotx_db::repo::partials::pipeline::{PIPELINE_ACTIVITY_KINDS_BY_GROUP, activity_kinds_of};
use chatbotx_db::repo::pipeline_field_condition::PipelineFieldCondition;
use chatbotx_db::repo::pipeline_item_cursor::PipelineItemSort;
use chatbotx_db::repo::pipeline_item_filter::{
    ClosedAs, DateRange, PIPELINE_ACTIVITY_KINDS, PipelineActivityMatch, PipelineItemFilter,
    PipelineValueFilter,
};
use chatbotx_db::repo::utils::SortDir;
use chatbotx_db::rusqlite::OptionalExtension;
use serde_json::{Map, Value, json};

use super::pipelines::member_scope;
use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid, schema_errors};
use crate::handlers::custom_fields::CUSTOM_FIELD_TYPES;
use crate::handlers::input::{Input, key};
use crate::handlers::pipelines::{
    COMMENT, DATE_MAX, DATE_MIN, IDEMPOTENCY_KEY, NAME, PRIORITIES, REFERENCE_NAME, STATUSES,
    item_fields, money, opt_id_list, parse_field_operator, parse_value_operator, publish, sort_key,
    stage_values, stats_json, stats_query, stored_value,
};
use crate::orpc::OrpcError;

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    Some(match path {
        "pipelinesAPI.privateGetBoardAPI" => board(ctx, input).await,
        "pipelinesAPI.privateListStagePageAPI" => stage_page(ctx, input).await,
        "pipelinesAPI.privateGetItemAPI" => get_item(ctx, input).await,
        "pipelinesAPI.privateListItemCommentsAPI" => comments(ctx, input).await,
        "pipelinesAPI.privateListItemActivityAPI" => activity(ctx, input).await,
        "pipelinesAPI.privateListPipelineMembersAPI" => list_members(ctx, input).await,
        "pipelinesAPI.privateGetStatsAPI" => stats(ctx, input).await,
        "pipelinesAPI.privateListContactItemsAPI" => contact_items(ctx, input).await,
        _ => return None,
    })
}

pub(super) const ACTIONS: &[&str] = &[
    "createStageAction",
    "updateStageAction",
    "deleteStageAction",
    "deleteStageMovingItemsAction",
    "createFieldAction",
    "updateFieldAction",
    "deleteFieldAction",
    "setMemberAction",
    "removeMemberAction",
    "createItemAction",
    "updateItemAction",
    "moveItemAction",
    "transferItemAction",
    "deleteItemAction",
    "addCommentAction",
    "deleteCommentAction",
    "addAttachmentsAction",
    "removeAttachmentAction",
];

// ---- JSON -------------------------------------------------------------------

/// `PipelineItemModel` (money in major units).
fn item_model(item: &PipelineItem) -> Value {
    let mut out = item.to_json();
    out["value"] = money(item.value);
    out
}

/// `PipelineItemView`. The contact's `channel` is not resolved (`null`).
fn view_json(view: &ItemView) -> Value {
    json!({
        "item": item_model(&view.item),
        "contact": view.contact.as_ref().map(|c| json!({
            "id": c.id.to_string(),
            "fullName": c.full_name,
            "avatar": c.avatar,
            "email": c.email,
            "phoneNumber": c.phone_number,
            "channel": null,
        })),
        "assignees": {
            "workspaceMemberIds": view.assignees.workspace_member_ids.iter().map(i64::to_string).collect::<Vec<_>>(),
            "inboxTeamIds": view.assignees.inbox_team_ids.iter().map(i64::to_string).collect::<Vec<_>>(),
        },
        "commentCount": view.comment_count,
    })
}

fn views_json(views: &[ItemView]) -> Value {
    Value::Array(views.iter().map(view_json).collect())
}

/// `PipelineActivityView`.
fn activity_json(view: &ActivityView) -> Value {
    let mut out = view.row.to_json();
    out["fromValue"] = money(view.row.from_value);
    out["toValue"] = money(view.row.to_value);
    out["fromStageName"] = json!(view.from_stage_name);
    out["toStageName"] = json!(view.to_stage_name);
    out["fromPipelineName"] = json!(view.from_pipeline_name);
    out["pipelineName"] = json!(view.pipeline_name);
    out
}

// ---- filter -------------------------------------------------------------------

fn ids_of(value: &Value, max: usize) -> Result<Vec<i64>, OrpcError> {
    let wrapper = json!({ "v": value });
    let ids = In(&wrapper).ids("v")?;
    if ids.len() > max {
        return Err(invalid());
    }
    Ok(ids)
}

fn enums_of(value: &Value, options: &[&str], max: usize) -> Result<Vec<String>, OrpcError> {
    let items = value.as_array().ok_or_else(invalid)?;
    if items.len() > max {
        return Err(invalid());
    }
    items
        .iter()
        .map(|item| match item.as_str() {
            Some(text) if options.contains(&text) => Ok(text.to_string()),
            _ => Err(invalid()),
        })
        .collect()
}

/// `boundedDate()`: an ISO string (a `Date` on the wire) or ms.
fn date_of(value: &Value) -> Result<i64, OrpcError> {
    let ms = match value {
        Value::String(text) => chatbotx_db::repo::parse_iso(text),
        Value::Number(n) => n.as_f64().map(|n| n as i64),
        _ => None,
    };
    ms.filter(|ms| (DATE_MIN..=DATE_MAX).contains(ms))
        .ok_or_else(invalid)
}

/// `dateRange` (`{ from?, to? }`, optional).
fn range_of(value: Option<&Value>) -> Result<Option<DateRange>, OrpcError> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let object = value.as_object().ok_or_else(invalid)?;
    let part = |name: &str| {
        object
            .get(name)
            .filter(|v| !v.is_null())
            .map(date_of)
            .transpose()
    };
    Ok(Some(DateRange {
        from: part("from")?,
        to: part("to")?,
    }))
}

fn amount_of(value: Option<&Value>) -> Result<Option<i64>, OrpcError> {
    match value.filter(|v| !v.is_null()) {
        None => Ok(None),
        Some(Value::Number(n)) => Ok(n.as_f64().map(stored_value)),
        Some(_) => Err(invalid()),
    }
}

/// `pipelineItemFilterRequest` as the repository filter (`toItemFilter`).
fn filter_of(value: Option<&Value>) -> Result<PipelineItemFilter, OrpcError> {
    let mut filter = PipelineItemFilter::default();
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(filter);
    };
    let object = value.as_object().ok_or_else(invalid)?;
    let get = |name: &str| object.get(name).filter(|v| !v.is_null());
    if let Some(v) = get("stageIds") {
        filter.stage_ids = ids_of(v, 200)?;
    }
    if let Some(v) = get("statuses") {
        filter.statuses = enums_of(v, STATUSES, 3)?;
    }
    if let Some(v) = get("priorities") {
        filter.priorities = enums_of(v, PRIORITIES, 3)?;
    }
    if let Some(v) = get("assigneeMemberIds") {
        filter.assignee_member_ids = ids_of(v, 100)?;
    }
    if let Some(v) = get("assigneeTeamIds") {
        filter.assignee_team_ids = ids_of(v, 100)?;
    }
    if let Some(v) = get("search") {
        let text = v.as_str().ok_or_else(invalid)?.trim();
        if text.chars().count() > 200 || text.contains('\0') {
            return Err(invalid());
        }
        filter.search = Some(text.to_string());
    }
    if let Some(v) = get("value") {
        let value = v.as_object().ok_or_else(invalid)?;
        let operator = match value.get("operator").filter(|v| !v.is_null()) {
            None => None,
            Some(op) => Some(
                op.as_str()
                    .and_then(parse_value_operator)
                    .ok_or_else(invalid)?,
            ),
        };
        filter.value = Some(PipelineValueFilter {
            min: amount_of(value.get("min"))?,
            max: amount_of(value.get("max"))?,
            operator,
            amount: amount_of(value.get("amount"))?,
        });
    }
    filter.due_date = range_of(get("dueDate"))?;
    filter.created_at = range_of(get("createdAt"))?;
    filter.updated_at = range_of(get("updatedAt"))?;
    filter.closed_at = range_of(get("closedAt"))?;
    filter.last_comment_at = range_of(get("lastCommentAt"))?;
    filter.status_changed_at = range_of(get("statusChangedAt"))?;
    filter.stage_entered_at = range_of(get("stageEnteredAt"))?;
    if let Some(v) = get("closedAs") {
        let closed = v.as_object().ok_or_else(invalid)?;
        if closed.keys().any(|k| k != "won" && k != "lost") {
            return Err(invalid());
        }
        filter.closed_as = Some(ClosedAs {
            won: range_of(closed.get("won"))?,
            lost: range_of(closed.get("lost"))?,
        });
    }
    if let Some(v) = get("fieldConditions") {
        let map = v.as_object().ok_or_else(invalid)?;
        if map.len() > 20 {
            return Err(invalid());
        }
        let operand = |entry: &Map<String, Value>, name: &str| match entry
            .get(name)
            .filter(|v| !v.is_null())
        {
            None => Ok(None),
            Some(Value::String(text)) if text.chars().count() <= 5000 && !text.contains('\0') => {
                Ok(Some(text.clone()))
            }
            Some(_) => Err(invalid()),
        };
        for (field_id, entry) in map {
            if !crate::handlers::input::is_bigint_id(field_id) {
                return Err(invalid());
            }
            let entry = entry.as_object().ok_or_else(invalid)?;
            let operator = entry
                .get("operator")
                .and_then(Value::as_str)
                .and_then(parse_field_operator)
                .ok_or_else(invalid)?;
            filter.field_conditions.insert(
                field_id.clone(),
                PipelineFieldCondition {
                    operator,
                    value: operand(entry, "value")?,
                    value_to: operand(entry, "valueTo")?,
                },
            );
        }
    }
    if let Some(v) = get("timezone") {
        let zone = v.as_str().ok_or_else(invalid)?;
        if !chatbotx_core::utils::tz::is_intl_time_zone(zone) {
            return Err(invalid());
        }
        filter.timezone = Some(zone.to_string());
    }
    if let Some(v) = get("activity") {
        let map = v.as_object().ok_or_else(invalid)?;
        let mut out = BTreeMap::new();
        for (kind, range) in map {
            if !PIPELINE_ACTIVITY_KINDS.contains(&kind.as_str()) {
                return Err(invalid());
            }
            if let Some(range) = range_of(Some(range))? {
                out.insert(kind.clone(), range);
            }
        }
        filter.activity = out;
    }
    if let Some(v) = get("activityMatch") {
        let m = v.as_object().ok_or_else(invalid)?;
        let kinds = match m.get("kinds").filter(|v| !v.is_null()) {
            None => Vec::new(),
            Some(v) => enums_of(v, &PIPELINE_ACTIVITY_KINDS, PIPELINE_ACTIVITY_KINDS.len())?,
        };
        let actor_user_ids = match m.get("actorUserIds").filter(|v| !v.is_null()) {
            None => Vec::new(),
            Some(v) => ids_of(v, 100)?,
        };
        let automation = match m.get("automation").filter(|v| !v.is_null()) {
            None => false,
            Some(Value::Bool(flag)) => *flag,
            Some(_) => return Err(invalid()),
        };
        filter.activity_match = Some(PipelineActivityMatch {
            kinds,
            actor_user_ids,
            automation,
            occurred_at: range_of(m.get("occurredAt"))?,
        });
    }
    Ok(filter)
}

/// `pipelineItemSortRequest` (`createdAt` / `desc` defaults).
fn sort_of(value: Option<&Value>) -> Result<PipelineItemSort, OrpcError> {
    let object = value.and_then(Value::as_object);
    let text = |name: &str| object.and_then(|o| o.get(name)).filter(|v| !v.is_null());
    let key = match text("key") {
        None => "createdAt",
        Some(Value::String(k))
            if ["createdAt", "updatedAt", "value", "dueDate", "priority"].contains(&k.as_str()) =>
        {
            k.as_str()
        }
        Some(_) => return Err(invalid()),
    };
    let direction = match text("direction") {
        None => SortDir::Desc,
        Some(Value::String(d)) if d == "desc" => SortDir::Desc,
        Some(Value::String(d)) if d == "asc" => SortDir::Asc,
        Some(_) => return Err(invalid()),
    };
    Ok(PipelineItemSort {
        key: sort_key(key),
        direction,
    })
}

// ---- procedures ---------------------------------------------------------------

async fn board(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let pipeline_id = In(input).id("pipelineId")?;
    let filter = filter_of(input.get("filter"))?;
    let sort = sort_of(input.get("sort"))?;
    let scope = member_scope(ctx).await?;
    let board = item_query::list_board(
        &ctx.api.db,
        ctx.workspace_id,
        pipeline_id,
        scope,
        filter,
        sort,
    )
    .await
    .map_err(fail)?;
    let mut pipeline = board.pipeline.pipeline.to_json();
    pipeline["currency"] = json!(board.pipeline.currency);
    pipeline["currencyOverride"] = json!(board.pipeline.currency_override);
    Ok(json!({
        "pipeline": pipeline,
        "fields": board.fields.iter().map(|f| f.to_json()).collect::<Vec<_>>(),
        "stages": board.stages.iter().map(|s| json!({
            "stage": s.stage.to_json(),
            "count": s.count,
            "totalValue": money(Some(s.total_value)),
            "items": views_json(&s.items),
            "nextCursor": s.next_cursor,
        })).collect::<Vec<_>>(),
        "workspaceDefault": {
            "currency": board.workspace_default.currency,
            "country": board.workspace_default.country,
        },
    }))
}

async fn stage_page(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let args = In(input);
    let pipeline_id = args.id("pipelineId")?;
    let stage_id = args.id("stageId")?;
    let cursor = args.opt_str("cursor")?;
    let filter = filter_of(input.get("filter"))?;
    let sort = sort_of(input.get("sort"))?;
    let scope = member_scope(ctx).await?;
    let page = item_query::list_items(
        &ctx.api.db,
        ctx.workspace_id,
        pipeline_id,
        Some(stage_id),
        scope,
        ItemListQuery {
            filter,
            sort,
            limit: item_query::DEFAULT_PER_STAGE_LIMIT,
            cursor,
        },
    )
    .await
    .map_err(fail)?;
    Ok(json!({ "items": views_json(&page.items), "nextCursor": page.next_cursor }))
}

fn item_ref(
    ctx: &Ctx<'_>,
    input: &Value,
    scope: chatbotx_business::pipeline::PipelineAccessScope,
) -> Result<ItemRef, OrpcError> {
    Ok(ItemRef {
        workspace_id: ctx.workspace_id,
        pipeline_id: In(input).id("pipelineId")?,
        item_id: In(input).id("itemId")?,
        scope,
    })
}

async fn get_item(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let scope = member_scope(ctx).await?;
    let target = item_ref(ctx, input, scope)?;
    let ws = ctx.workspace_id;
    let detail = item::get_item(&ctx.api.db, ws, target.pipeline_id, target.item_id, scope)
        .await
        .map_err(fail)?;
    let contact_id = detail.view.item.contact_id;
    // `conversationService.findDMByContactIds`: the contact's conversation
    // without a source (not a comment thread).
    let conversation_id = chatbotx_business::tx::read(&ctx.api.db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT "id" FROM "Conversation" WHERE "workspaceId" = ?1 AND "contactId" = ?2 AND "sourceId" IS NULL ORDER BY "id" LIMIT 1"#,
                [ws, contact_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(chatbotx_db::repo::Error::from)?)
    })
    .await
    .map_err(fail)?;
    let mut out = view_json(&detail.view);
    out["attachments"] = detail
        .attachments
        .iter()
        .map(|row| {
            let mut attachment = row.attachment.to_json();
            attachment["mediaLibraryFile"] = row
                .media_library_file
                .as_ref()
                .map_or(Value::Null, |file| file.to_json());
            attachment
        })
        .collect();
    out["conversationId"] = json!(conversation_id.map(|id| id.to_string()));
    Ok(out)
}

async fn comments(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let scope = member_scope(ctx).await?;
    let target = item_ref(ctx, input, scope)?;
    let cursor = In(input).opt_str("cursor")?;
    let page = content::list_comments(&ctx.api.db, target, cursor)
        .await
        .map_err(fail)?;
    Ok(json!({
        "items": page.items.iter().map(|c| c.to_json()).collect::<Vec<_>>(),
        "nextCursor": page.next_cursor,
    }))
}

async fn activity(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let scope = member_scope(ctx).await?;
    let target = item_ref(ctx, input, scope)?;
    let cursor = In(input).opt_str("cursor")?;
    let options: Vec<&str> = PIPELINE_ACTIVITY_KINDS_BY_GROUP
        .iter()
        .map(|(name, _)| *name)
        .collect();
    let groups = match In(input).get("groups") {
        None => None,
        Some(v) => Some(enums_of(v, &options, options.len())?),
    };
    let names: Option<Vec<&str>> = groups
        .as_ref()
        .map(|groups| groups.iter().map(String::as_str).collect());
    let kinds = activity_kinds_of(names.as_deref())
        .map(|kinds| kinds.into_iter().map(str::to_string).collect());
    let page = content::list_activity(&ctx.api.db, target, cursor, kinds)
        .await
        .map_err(fail)?;
    Ok(json!({
        "items": page.items.iter().map(activity_json).collect::<Vec<_>>(),
        "nextCursor": page.next_cursor,
    }))
}

async fn list_members(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let pipeline_id = In(input).id("pipelineId")?;
    let scope = member_scope(ctx).await?;
    let rows = members::list_members(&ctx.api.db, ctx.workspace_id, pipeline_id, scope)
        .await
        .map_err(fail)?;
    Ok(rows.iter().map(|row| row.to_json()).collect())
}

async fn stats(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let pipeline_id = In(input).id("pipelineId")?;
    // The serializer is not `Send`: build the text before any await.
    let query = {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        for name in ["from", "to", "timezone"] {
            if let Some(value) = input.get(name).and_then(Value::as_str) {
                query.append_pair(name, value);
            }
        }
        query.finish()
    };
    let params = [("id", pipeline_id.to_string())];
    let (from, to, timezone) = stats_query(&params, Some(&query)).map_err(|_| invalid())?;
    let scope = member_scope(ctx).await?;
    let stats = analytics::get_stats(
        &ctx.api.db,
        ctx.workspace_id,
        pipeline_id,
        from,
        to,
        timezone,
        scope,
    )
    .await
    .map_err(fail)?;
    Ok(stats_json(&stats))
}

async fn contact_items(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let contact_id = In(input).id("contactId")?;
    let scope = member_scope(ctx).await?;
    let rows = item_query::list_for_contact(&ctx.api.db, ctx.workspace_id, contact_id, scope)
        .await
        .map_err(fail)?;
    Ok(rows
        .iter()
        .map(|row| {
            let mut out = view_json(&row.view);
            let mut pipeline = row.pipeline.pipeline.to_json();
            pipeline["currency"] = json!(row.pipeline.currency);
            pipeline["currencyOverride"] = json!(row.pipeline.currency_override);
            out["pipeline"] = pipeline;
            out
        })
        .collect())
}

// ---- actions ------------------------------------------------------------------

/// `userPipelineActor`.
fn user_actor(ctx: &Ctx<'_>) -> PipelineActor {
    PipelineActor {
        kind: "user".to_string(),
        user_id: Some(ctx.user_id),
        source_id: None,
    }
}

async fn published<T>(ctx: &Ctx<'_>, mutation: ItemMutation<T>) -> T {
    publish(ctx.api, &mutation).await;
    mutation.value
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let body = arg(args, 1).to_string();
    let mut input = Input::from_body(&[], body.as_bytes());
    let pipeline_id = input.id("id").unwrap_or_default();
    let scope = member_scope(ctx).await?;
    let (db, ws) = (&ctx.api.db, ctx.workspace_id);
    let actor = user_actor(ctx);
    let target = |item_id: Option<i64>| ItemRef {
        workspace_id: ws,
        pipeline_id,
        item_id: item_id.unwrap_or_default(),
        scope,
    };
    macro_rules! done {
        ($input:ident) => {
            $input.finish().map_err(schema_errors)?
        };
    }
    Ok(match name {
        "createStageAction" => {
            let name = input.string("name", NAME);
            let mut values = stage_values(&mut input);
            values.name = name;
            done!(input);
            let stage = structure::create_stage(db, ws, pipeline_id, scope, values)
                .await
                .map_err(action_fail)?;
            json!({ "id": stage.id.to_string() })
        }
        "updateStageAction" => {
            let stage_id = input.id("stageId");
            let values = stage_values(&mut input);
            done!(input);
            structure::update_stage(
                db,
                ws,
                pipeline_id,
                stage_id.unwrap_or_default(),
                scope,
                values,
            )
            .await
            .map_err(action_fail)?;
            Value::Null
        }
        "deleteStageAction" | "deleteStageMovingItemsAction" => {
            let stage_id = input.id("stageId");
            let move_to = (name == "deleteStageMovingItemsAction")
                .then(|| input.id("moveItemsToStageId"))
                .flatten();
            done!(input);
            let result = structure::delete_stage(
                db,
                ws,
                pipeline_id,
                stage_id.unwrap_or_default(),
                move_to,
                scope,
                actor,
            )
            .await
            .map_err(action_fail)?;
            json!({ "queued": result.queued, "jobId": result.job_id })
        }
        "createFieldAction" => {
            let name = input.string("name", NAME);
            let reference_name = input.string("referenceName", REFERENCE_NAME);
            let field_type = input.enum_at(
                &key("type"),
                input.get("type").cloned().as_ref(),
                CUSTOM_FIELD_TYPES,
                true,
            );
            let required = input.opt_bool("required");
            done!(input);
            let field = structure::create_field(
                db,
                ws,
                pipeline_id,
                scope,
                FieldValues {
                    name: name.unwrap_or_default(),
                    reference_name: reference_name.unwrap_or_default(),
                    field_type: field_type.unwrap_or_default(),
                    required: required.unwrap_or(false),
                },
            )
            .await
            .map_err(action_fail)?;
            json!({ "id": field.id.to_string() })
        }
        "updateFieldAction" => {
            let field_id = input.id("fieldId");
            let patch = FieldPatch {
                name: input.opt_string("name", NAME),
                reference_name: input.opt_string("referenceName", REFERENCE_NAME),
                required: input.opt_bool("required"),
            };
            done!(input);
            structure::update_field(
                db,
                ws,
                pipeline_id,
                field_id.unwrap_or_default(),
                scope,
                patch,
            )
            .await
            .map_err(action_fail)?;
            Value::Null
        }
        "deleteFieldAction" => {
            let field_id = input.id("fieldId");
            done!(input);
            structure::delete_field(db, ws, pipeline_id, field_id.unwrap_or_default(), scope)
                .await
                .map_err(action_fail)?;
            Value::Null
        }
        "setMemberAction" => {
            let member_id = input.id("workspaceMemberId");
            let permissions = PipelinePermissions {
                can_view: input.bool("canView").unwrap_or(false),
                can_edit: input.opt_bool("canEdit").unwrap_or(false),
                can_delete: input.opt_bool("canDelete").unwrap_or(false),
                can_manage_settings: input.opt_bool("canManageSettings").unwrap_or(false),
            };
            done!(input);
            members::set_member(
                db,
                ws,
                pipeline_id,
                member_id.unwrap_or_default(),
                scope,
                permissions,
            )
            .await
            .map_err(action_fail)?;
            Value::Null
        }
        "removeMemberAction" => {
            let member_id = input.id("workspaceMemberId");
            done!(input);
            members::remove_member(db, ws, pipeline_id, member_id.unwrap_or_default(), scope)
                .await
                .map_err(action_fail)?;
            Value::Null
        }
        "createItemAction" => {
            let contact_id = input.id("contactId");
            let fields = item_fields(&mut input, true);
            let files = opt_id_list(&mut input, "mediaLibraryFileIds", 20);
            let idempotency_key = input.opt_string("idempotencyKey", IDEMPOTENCY_KEY);
            done!(input);
            let created = item::create(
                db,
                CreateItem {
                    workspace_id: ws,
                    pipeline_id,
                    contact_id: contact_id.unwrap_or_default(),
                    scope,
                    actor,
                    fields,
                    media_library_file_ids: files.unwrap_or_default(),
                    idempotency_key,
                },
            )
            .await
            .map_err(action_fail)?;
            let item = published(ctx, created).await;
            json!({ "id": item.id.to_string() })
        }
        "updateItemAction" | "moveItemAction" => {
            let item_id = input.id("itemId").unwrap_or_default();
            let fields = if name == "moveItemAction" {
                ItemFields {
                    stage_id: input.id("stageId"),
                    ..ItemFields::default()
                }
            } else {
                item_fields(&mut input, false)
            };
            done!(input);
            let updated = item::update(
                db,
                UpdateItem {
                    workspace_id: ws,
                    pipeline_id,
                    item_id,
                    scope,
                    actor,
                    fields,
                },
            )
            .await
            .map_err(action_fail)?;
            published(ctx, updated).await;
            Value::Null
        }
        "transferItemAction" => {
            let item_id = input.id("itemId").unwrap_or_default();
            let destination = input.id("destinationPipelineId").unwrap_or_default();
            let destination_stage_id = match input.get("destinationStageId").cloned() {
                None => None,
                Some(v) => input.id_at(&key("destinationStageId"), Some(&v), true),
            };
            done!(input);
            let moved = item::transfer(
                db,
                TransferItem {
                    workspace_id: ws,
                    pipeline_id,
                    item_id,
                    destination_pipeline_id: destination,
                    destination_stage_id,
                    scope,
                    actor,
                },
            )
            .await
            .map_err(action_fail)?;
            published(ctx, moved).await;
            Value::Null
        }
        "deleteItemAction" => {
            let item_id = input.id("itemId").unwrap_or_default();
            done!(input);
            let deleted = item::delete(db, ws, pipeline_id, item_id, scope, actor)
                .await
                .map_err(action_fail)?;
            published(ctx, deleted).await;
            Value::Null
        }
        "addCommentAction" => {
            let item = target(input.id("itemId"));
            let text = input.string("text", COMMENT);
            let idempotency_key = input.opt_string("idempotencyKey", IDEMPOTENCY_KEY);
            done!(input);
            let added =
                content::add_comment(db, item, actor, text.unwrap_or_default(), idempotency_key)
                    .await
                    .map_err(action_fail)?;
            published(ctx, added).await;
            Value::Null
        }
        "deleteCommentAction" => {
            let item = target(input.id("itemId"));
            let comment_id = input.id("commentId");
            done!(input);
            let deleted = content::delete_comment(db, item, actor, comment_id.unwrap_or_default())
                .await
                .map_err(action_fail)?;
            published(ctx, deleted).await;
            Value::Null
        }
        "addAttachmentsAction" => {
            let item = target(input.id("itemId"));
            let files = opt_id_list(&mut input, "mediaLibraryFileIds", 20);
            if files.as_ref().is_some_and(Vec::is_empty) || !input.has("mediaLibraryFileIds") {
                input.issue(
                    &key("mediaLibraryFileIds"),
                    "too_small",
                    "Too small: expected array to have >=1 items",
                );
            }
            done!(input);
            let added = content::add_attachments(db, item, actor, files.unwrap_or_default())
                .await
                .map_err(action_fail)?;
            published(ctx, added).await;
            Value::Null
        }
        _ => {
            let item = target(input.id("itemId"));
            let file_id = input.id("mediaLibraryFileId");
            done!(input);
            let removed = content::remove_attachment(db, item, actor, file_id.unwrap_or_default())
                .await
                .map_err(action_fail)?;
            published(ctx, removed).await;
            Value::Null
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_parses_the_board_panel_shape() {
        let filter = filter_of(Some(&json!({
            "stageIds": ["3"],
            "statuses": ["open"],
            "search": "  acme ",
            "value": { "operator": "gt", "amount": 1.5 },
            "dueDate": { "from": "2026-01-01T00:00:00.000Z" },
            "closedAs": { "won": { "to": "2026-02-01T00:00:00.000Z" } },
            "fieldConditions": { "7": { "operator": "contains", "value": "x" } },
            "activityMatch": { "kinds": ["created"], "automation": true },
        })))
        .unwrap();
        assert_eq!(filter.stage_ids, vec![3]);
        assert_eq!(filter.search.as_deref(), Some("acme"));
        assert_eq!(filter.value.unwrap().amount, Some(15_000));
        assert_eq!(filter.due_date.unwrap().from, Some(1_767_225_600_000));
        assert!(filter.closed_as.unwrap().won.is_some());
        assert!(filter.field_conditions.contains_key("7"));
        assert!(filter.activity_match.unwrap().automation);
        assert!(filter_of(Some(&json!({ "statuses": ["nope"] }))).is_err());
        assert!(filter_of(Some(&json!({ "fieldConditions": { "x": {} } }))).is_err());
        assert!(sort_of(Some(&json!({ "key": "bad" }))).is_err());
        assert_eq!(sort_of(None).unwrap().direction, SortDir::Desc);
    }
}
