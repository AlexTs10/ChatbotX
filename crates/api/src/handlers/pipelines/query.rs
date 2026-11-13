//! `listItemsPublicRequest` / `stageTotalsPublicRequest` + `toItemFilter` /
//! `toItemListQuery` (`lib/list-items-query.ts`) and the routes
//! `pipelines.listItems`, `stageTotals`, `listContactItems`. Money amounts
//! become the stored x 10,000 integer; dates unix ms.

use std::collections::BTreeMap;

use chatbotx_business::pipeline::query::{self as item_query, ItemListQuery};
use chatbotx_core::utils::tz::is_intl_time_zone;
use chatbotx_db::repo::partials::pipeline::{PIPELINE_ACTIVITY_KINDS_BY_GROUP, activity_kinds_of};
use chatbotx_db::repo::pipeline_field_condition::{PipelineFieldCondition, PipelineFieldOperator};
use chatbotx_db::repo::pipeline_item_cursor::{PipelineItemSort, PipelineItemSortKey};
use chatbotx_db::repo::pipeline_item_filter::{
    ClosedAs, DateRange, PipelineActivityMatch, PipelineItemFilter, PipelineValueFilter,
    PipelineValueOperator,
};
use chatbotx_db::repo::utils::SortDir;
use serde_json::{Value, json};

use super::super::input::{Input, Num, Seg, Str, child, key};
use super::super::{ApiState, Call, FINDING, Reply, business_error, contacts};
use super::{
    PRIORITIES, STATUSES, TOKEN, bounded_date_at, item_json, money, no_nul, ok, stored_value,
};
use crate::orpc::OrpcError;

/// `PIPELINE_LIMITS.stagesPerPipeline`.
const MAX_STAGE_IDS: usize = 200;
const MAX_ASSIGNEE_FILTER_IDS: usize = 100;
const MAX_ACTIVITY_ACTORS: usize = 100;
/// `MAX_FIELD_FILTERS`.
const MAX_FIELD_FILTERS: usize = 20;
/// `MAX_FIELD_VALUE_LENGTH` of `schema/bounds.ts`.
const MAX_OPERAND_LENGTH: usize = 5000;
const AUTOMATION_ACTOR: &str = "automation";
const SEARCH: Str = Str {
    trim: true,
    min: None,
    max: Some(200),
    pattern: Some(no_nul),
};
const OPERAND: Str = Str {
    pattern: Some(no_nul),
    ..Str::max(MAX_OPERAND_LENGTH)
};
const VALUE_OPERATORS: &[&str] = &["eq", "gt", "lt", "ne", "between"];
const FIELD_OPERATORS: &[&str] = &[
    "is",
    "isNot",
    "hasValue",
    "noValue",
    "contains",
    "notContains",
    "startsWith",
    "endsWith",
    "gt",
    "lt",
    "between",
];
const SORT_KEYS: &[&str] = &["createdAt", "updatedAt", "value", "dueDate", "priority"];

fn parse_value_operator(name: &str) -> Option<PipelineValueOperator> {
    Some(match name {
        "eq" => PipelineValueOperator::Eq,
        "gt" => PipelineValueOperator::Gt,
        "lt" => PipelineValueOperator::Lt,
        "ne" => PipelineValueOperator::Ne,
        "between" => PipelineValueOperator::Between,
        _ => return None,
    })
}

fn parse_field_operator(name: &str) -> Option<PipelineFieldOperator> {
    use PipelineFieldOperator::*;
    Some(match name {
        "is" => Is,
        "isNot" => IsNot,
        "hasValue" => HasValue,
        "noValue" => NoValue,
        "contains" => Contains,
        "notContains" => NotContains,
        "startsWith" => StartsWith,
        "endsWith" => EndsWith,
        "gt" => Gt,
        "lt" => Lt,
        "between" => Between,
        _ => return None,
    })
}

fn sort_key(name: &str) -> PipelineItemSortKey {
    match name {
        "updatedAt" => PipelineItemSortKey::UpdatedAt,
        "value" => PipelineItemSortKey::Value,
        "dueDate" => PipelineItemSortKey::DueDate,
        "priority" => PipelineItemSortKey::Priority,
        _ => PipelineItemSortKey::CreatedAt,
    }
}

/// `commaSeparated`: the trimmed non-empty parts; `None` when there are none.
fn comma_parts(input: &Input, name: &str) -> Option<Vec<Value>> {
    let parts: Vec<Value> = match input.get(name)? {
        Value::String(text) => text
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| Value::String(part.to_string()))
            .collect(),
        Value::Array(items) => items.clone(),
        _ => Vec::new(),
    };
    (!parts.is_empty()).then_some(parts)
}

fn too_many(input: &mut Input, path: &[Seg], max: usize) {
    input.issue(
        path,
        "too_big",
        &format!("Too big: expected array to have <={max} items"),
    );
}

/// `commaList(zodBigintIdAsString(), max)`.
fn comma_ids(input: &mut Input, name: &str, max: usize) -> Vec<i64> {
    let Some(parts) = comma_parts(input, name) else {
        return Vec::new();
    };
    let path = key(name);
    if parts.len() > max {
        too_many(input, &path, max);
        return Vec::new();
    }
    let mut out = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        let mut item_path = path.clone();
        item_path.push(Seg::Index(index));
        out.extend(input.id_at(&item_path, Some(part), true));
    }
    out
}

/// `commaList(z.enum(options), max)`.
fn comma_enums(input: &mut Input, name: &str, options: &[&str], max: usize) -> Vec<String> {
    let Some(parts) = comma_parts(input, name) else {
        return Vec::new();
    };
    let path = key(name);
    if parts.len() > max {
        too_many(input, &path, max);
        return Vec::new();
    }
    let mut out = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        let mut item_path = path.clone();
        item_path.push(Seg::Index(index));
        out.extend(input.enum_at(&item_path, Some(part), options, true));
    }
    out
}

/// `activityActor`: user ids and `automation`.
fn activity_actors(input: &mut Input) -> (Vec<i64>, bool) {
    let Some(parts) = comma_parts(input, "activityActor") else {
        return (Vec::new(), false);
    };
    let path = key("activityActor");
    if parts.len() > MAX_ACTIVITY_ACTORS {
        too_many(input, &path, MAX_ACTIVITY_ACTORS);
        return (Vec::new(), false);
    }
    let mut ids = Vec::new();
    let mut automation = false;
    for (index, part) in parts.iter().enumerate() {
        if part.as_str() == Some(AUTOMATION_ACTOR) {
            automation = true;
            continue;
        }
        let mut item_path = path.clone();
        item_path.push(Seg::Index(index));
        let is_id = part.as_str().is_some_and(super::super::input::is_bigint_id);
        if is_id {
            ids.extend(input.id_at(&item_path, Some(part), true));
        } else {
            input.issue(&item_path, "invalid_union", "Invalid input");
        }
    }
    (ids, automation)
}

fn opt_id(input: &mut Input, name: &str) -> Option<i64> {
    let value = input.get(name).cloned()?;
    input.id_at(&key(name), Some(&value), true)
}

fn date(input: &mut Input, name: &str) -> Option<i64> {
    bounded_date_at(input, name).flatten()
}

/// `rangeOf(<prefix>From, <prefix>To)`.
fn range(input: &mut Input, prefix: &str) -> Option<DateRange> {
    let from = date(input, &format!("{prefix}From"));
    let to = date(input, &format!("{prefix}To"));
    (from.is_some() || to.is_some()).then_some(DateRange { from, to })
}

/// `ianaTimezoneSchema.optional()`.
fn timezone(input: &mut Input) -> Option<String> {
    let path = key("timezone");
    let value = input.get("timezone").cloned();
    let zone = input.string_at(&path, value.as_ref(), Str::default(), false)?;
    if !is_intl_time_zone(&zone) {
        input.issue(&path, "custom", "Invalid IANA timezone");
        return None;
    }
    Some(zone)
}

fn operand_at(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Option<String> {
    input.string_at(path, value, OPERAND, false)
}

/// `fieldConditions`: a JSON object (in a query string) keyed by field id.
fn field_conditions(input: &mut Input) -> BTreeMap<String, PipelineFieldCondition> {
    let mut out = BTreeMap::new();
    let Some(raw) = input.get("fieldConditions").cloned() else {
        return out;
    };
    let value = match raw {
        Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::String(text)),
        other => other,
    };
    let path = key("fieldConditions");
    let Value::Object(map) = value else {
        input.issue(&path, "invalid_type", "Invalid input: expected record");
        return out;
    };
    for (field_id, entry) in &map {
        let entry_path = child(&path, field_id);
        if !super::super::input::is_bigint_id(field_id) {
            input.issue(&entry_path, "invalid_key", "Invalid key in record");
            continue;
        }
        let Value::Object(entry) = entry else {
            input.issue(
                &entry_path,
                "invalid_type",
                "Invalid input: expected object",
            );
            continue;
        };
        let operator = input
            .enum_at(
                &child(&entry_path, "operator"),
                entry.get("operator"),
                FIELD_OPERATORS,
                true,
            )
            .and_then(|name| parse_field_operator(&name));
        let value = operand_at(input, &child(&entry_path, "value"), entry.get("value"));
        let value_to = operand_at(input, &child(&entry_path, "valueTo"), entry.get("valueTo"));
        if let Some(operator) = operator {
            out.insert(
                field_id.clone(),
                PipelineFieldCondition {
                    operator,
                    value,
                    value_to,
                },
            );
        }
    }
    if !input.failed() && map.len() > MAX_FIELD_FILTERS {
        input.issue(&path, "custom", "Too many field filters");
    }
    out
}

fn opt_amount(input: &mut Input, name: &str) -> Option<i64> {
    let value = input.get(name).cloned();
    input
        .coerce_number_at(&key(name), value.as_ref(), Num::default(), false)
        .map(stored_value)
}

/// `refineItemFilter`: a comparison without its operand is a 422.
fn refine(input: &mut Input, operands: &Operands) {
    if let Some(operator) = &operands.value_operator {
        if operator != "between" && operands.value_amount.is_none() {
            input.issue(
                &key("valueAmount"),
                "custom",
                &format!("valueOperator {operator} needs valueAmount"),
            );
        }
        if operator == "between" && operands.value_min.is_none() && operands.value_max.is_none() {
            input.issue(
                &key("valueMin"),
                "custom",
                "valueOperator between needs valueMin, valueMax or both",
            );
        }
    }
    let Some(operator) = &operands.field_operator else {
        return;
    };
    if operands.field_id.is_none() {
        input.issue(&key("fieldId"), "custom", "fieldOperator needs fieldId");
    }
    let has = |value: &Option<String>| value.as_deref().is_some_and(|v| !v.is_empty());
    let has_value = has(&operands.field_value);
    let lacks = if operator == "between" {
        !(has_value || has(&operands.field_value_to))
    } else {
        !(has_value || operator == "hasValue" || operator == "noValue")
    };
    if lacks {
        let needs = if operator == "between" {
            "fieldValue or fieldValueTo"
        } else {
            "fieldValue"
        };
        input.issue(
            &key("fieldValue"),
            "custom",
            &format!("fieldOperator {operator} needs {needs}"),
        );
    }
}

struct Operands {
    value_operator: Option<String>,
    value_amount: Option<i64>,
    value_min: Option<i64>,
    value_max: Option<i64>,
    field_id: Option<i64>,
    field_operator: Option<String>,
    field_value: Option<String>,
    field_value_to: Option<String>,
}

/// `itemFilterQuery` + `toItemFilter`; the refinement runs after the
/// whole list query is read (zod `superRefine` after the object).
fn read_filter(input: &mut Input) -> (PipelineItemFilter, Operands) {
    let _pipeline_id = input.id("id");
    let stage_ids = comma_ids(input, "stageId", MAX_STAGE_IDS);
    let contact_id = opt_id(input, "contactId");
    let statuses = comma_enums(input, "status", STATUSES, STATUSES.len());
    let priorities = comma_enums(input, "priority", PRIORITIES, PRIORITIES.len());
    let assignee_member_ids = comma_ids(input, "assigneeMemberId", MAX_ASSIGNEE_FILTER_IDS);
    let assignee_team_ids = comma_ids(input, "assigneeInboxTeamId", MAX_ASSIGNEE_FILTER_IDS);
    let search = input.opt_string("search", SEARCH);
    let value_operator = {
        let value = input.get("valueOperator").cloned();
        input.enum_at(
            &key("valueOperator"),
            value.as_ref(),
            VALUE_OPERATORS,
            false,
        )
    };
    let value_amount = opt_amount(input, "valueAmount");
    let value_min = opt_amount(input, "valueMin");
    let value_max = opt_amount(input, "valueMax");
    let due_date = range(input, "due");
    let created_at = range(input, "created");
    let updated_at = range(input, "updated");
    let last_comment_at = range(input, "lastComment");
    let status_changed_at = range(input, "statusChanged");
    let stage_entered_at = range(input, "stageEntered");
    let won = range(input, "wonClosed");
    let lost = range(input, "lostClosed");
    let groups = comma_enums(
        input,
        "activityGroups",
        &PIPELINE_ACTIVITY_KINDS_BY_GROUP
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>(),
        PIPELINE_ACTIVITY_KINDS_BY_GROUP.len(),
    );
    let (actor_user_ids, automation) = activity_actors(input);
    let occurred_at = range(input, "activity");
    let mut conditions = field_conditions(input);
    let field_id = opt_id(input, "fieldId");
    let field_operator = {
        let value = input.get("fieldOperator").cloned();
        input.enum_at(
            &key("fieldOperator"),
            value.as_ref(),
            FIELD_OPERATORS,
            false,
        )
    };
    let field_value = input.opt_string("fieldValue", OPERAND);
    let field_value_to = input.opt_string("fieldValueTo", OPERAND);
    let timezone = timezone(input);

    if let (Some(id), Some(operator)) = (
        field_id,
        field_operator.as_deref().and_then(parse_field_operator),
    ) {
        conditions.insert(
            id.to_string(),
            PipelineFieldCondition {
                operator,
                value: field_value.clone(),
                value_to: field_value_to.clone(),
            },
        );
    }
    let group_names: Vec<&str> = groups.iter().map(String::as_str).collect();
    let kinds = activity_kinds_of(Some(&group_names));
    let activity_match =
        (kinds.is_some() || !actor_user_ids.is_empty() || automation || occurred_at.is_some())
            .then(|| PipelineActivityMatch {
                kinds: kinds
                    .unwrap_or_default()
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                actor_user_ids,
                automation,
                occurred_at,
            });
    let has_range = value_min.is_some() || value_max.is_some();
    let value = (value_operator.is_some() || has_range).then(|| PipelineValueFilter {
        operator: value_operator.as_deref().and_then(parse_value_operator),
        amount: value_amount,
        min: value_min,
        max: value_max,
    });
    let closed_as = (won.is_some() || lost.is_some()).then_some(ClosedAs { won, lost });
    let filter = PipelineItemFilter {
        stage_ids,
        statuses,
        priorities,
        contact_id,
        assignee_member_ids,
        assignee_team_ids,
        search,
        value,
        due_date,
        created_at,
        updated_at,
        field_conditions: conditions,
        timezone,
        last_comment_at,
        status_changed_at,
        stage_entered_at,
        activity_match,
        closed_as,
        ..PipelineItemFilter::default()
    };
    let operands = Operands {
        value_operator,
        value_amount,
        value_min,
        value_max,
        field_id,
        field_operator,
        field_value,
        field_value_to,
    };
    (filter, operands)
}

fn finish_refined(mut input: Input, operands: &Operands) -> Result<(), OrpcError> {
    if !input.failed() {
        refine(&mut input, operands);
    }
    input.finish()
}

/// `listItemsPublicRequest.parse` + `toItemListQuery`.
pub fn item_list_query(
    params: &[(&'static str, String)],
    query: Option<&str>,
) -> Result<ItemListQuery, OrpcError> {
    let mut input = Input::from_query(params, query);
    let (filter, operands) = read_filter(&mut input);
    let sort = {
        let value = input.get("sort").cloned();
        input.enum_at(&key("sort"), value.as_ref(), SORT_KEYS, false)
    };
    let direction = {
        let value = input.get("direction").cloned();
        input.enum_at(&key("direction"), value.as_ref(), &["asc", "desc"], false)
    };
    let limit = input.opt_coerce_int("limit", Num::int(1.0, 100.0));
    let cursor = input.opt_string("cursor", Str::default());
    finish_refined(input, &operands)?;
    Ok(ItemListQuery {
        filter,
        sort: PipelineItemSort {
            key: sort_key(sort.as_deref().unwrap_or("createdAt")),
            direction: if direction.as_deref() == Some("asc") {
                SortDir::Asc
            } else {
                SortDir::Desc
            },
        },
        limit: limit.unwrap_or(50),
        cursor,
    })
}

/// `stageTotalsPublicRequest.parse` + `toItemFilter`.
pub fn item_filter_query(
    params: &[(&'static str, String)],
    query: Option<&str>,
) -> Result<PipelineItemFilter, OrpcError> {
    let mut input = Input::from_query(params, query);
    let (filter, operands) = read_filter(&mut input);
    finish_refined(input, &operands)?;
    Ok(filter)
}

fn path_id(call: &Call<'_>) -> i64 {
    call.params
        .iter()
        .find(|(name, _)| *name == "id")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or_default()
}

/// Runs `pipelines.listItems`, `stageTotals` or `listContactItems`.
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let finding = |e| business_error(e, FINDING);
    match route_id {
        "pipelines.listItems" => {
            let query = item_list_query(call.params, call.query)?;
            let page = item_query::list_items(db, ws, path_id(&call), None, TOKEN, query)
                .await
                .map_err(finding)?;
            ok(
                200,
                json!({
                    "items": page.items.iter().map(item_json).collect::<Vec<_>>(),
                    "nextCursor": page.next_cursor,
                }),
            )
        }
        "pipelines.stageTotals" => {
            let filter = item_filter_query(call.params, call.query)?;
            let rows = item_query::list_stage_totals(db, ws, path_id(&call), TOKEN, filter)
                .await
                .map_err(finding)?;
            let stages: Vec<Value> = rows
                .iter()
                .map(|row| {
                    json!({
                        "stageId": row.stage_id.to_string(),
                        "name": row.name,
                        "count": row.count,
                        "totalValue": money(Some(row.total_value)),
                    })
                })
                .collect();
            ok(200, json!({ "stages": stages }))
        }
        "pipelines.listContactItems" => {
            let contact_id = contacts::resolve(state, &call, FINDING).await?;
            let rows = item_query::list_for_contact(db, ws, contact_id, TOKEN)
                .await
                .map_err(finding)?;
            let items: Vec<Value> = rows
                .iter()
                .map(|row| {
                    let mut item = item_json(&row.view);
                    if let Value::Object(map) = &mut item {
                        map.insert("pipelineName".into(), json!(row.pipeline.pipeline.name));
                        map.insert("pipelineType".into(), json!(row.pipeline.pipeline.r#type));
                    }
                    item
                })
                .collect();
            ok(200, json!({ "items": items }))
        }
        _ => Err(super::super::internal_error(route_id)),
    }
}
