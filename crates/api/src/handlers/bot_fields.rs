//! `apps/builder/src/features/bot-fields/api/public.ts`
//! (`botFieldsPublicRouter`, P3.6-D2 slice bot fields), with
//! `schema/action.ts` and `schema/query.ts`.

use chatbotx_business::BusinessError;
use chatbotx_business::bot_field::{
    self as service, BotField, CreateBotField, ListBotFields, UpdateBotField, ValueOperation,
};
use chatbotx_core::flow_config::steps::is_field_operand_missing;
use chatbotx_core::utils::custom_field_operation::{
    field_operation_api_names, field_operation_by_api_name,
};
use chatbotx_db::repo::SortItem;
use serde_json::{Map, Value, json};

use super::contact_custom_fields::{client_timezone, received};
use super::custom_fields::CUSTOM_FIELD_TYPES;
use super::input::{Input, Seg, Str, key};
use super::qr_codes::{public_paging, sort_query};
use super::{
    ApiState, Call, ErrorMap, FINDING, MUTATING, Reply, business_error, date, id, json, no_content,
    opt_id,
};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "botFields.list",
    "botFields.create",
    "botFields.get",
    "botFields.set",
    "botFields.update",
    "botFields.setMany",
    "botFields.bulkUpdate",
    "botFields.applyOperation",
    "botFields.reset",
    "botFields.resetMany",
    "botFields.delete",
];

/// `possibleErrorsOnCreatingBotField`.
const CREATING: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("notFound", 404),
    ("invalidFieldOperation", 400),
    ("invalidCustomFieldValue", 400),
];
/// `possibleErrorsOnSettingBotField`.
const SETTING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("invalidFieldOperation", 400),
    ("invalidCustomFieldValue", 400),
];
/// `possibleErrorsOnDeletingTemplateResource`.
const DELETING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("templateAllowDeleteViolation", 400),
];

const RESERVED_PREFIX: &str = "bot_field:";

/// `publicBotFieldResource`.
pub fn bot_field_json(field: &BotField) -> Value {
    json!({
        "id": id(field.id),
        "name": field.name,
        "type": field.r#type,
        "value": field.value,
        "description": field.description,
        "createdAt": date(field.created_at),
        "updatedAt": date(field.updated_at),
        "folderId": opt_id(field.folder_id),
    })
}

/// `z.string().max(255)` (`idOrName`).
fn id_or_name(input: &mut Input) -> String {
    input.string("idOrName", Str::max(255)).unwrap_or_default()
}

/// `zodFieldName()` at a top-level key (`None` = absent when optional).
fn field_name(input: &mut Input, required: bool) -> Option<String> {
    if !required && !input.has("name") {
        return None;
    }
    let name = input.string("name", Str::trimmed(1, 255))?;
    if name.starts_with(RESERVED_PREFIX) {
        input.issue(
            &key("name"),
            "custom",
            "Name must not start with the reserved \"bot_field:\" prefix.",
        );
        return None;
    }
    Some(name)
}

/// `zodBigintAsString()` digits at `path`; beyond `i64` = `-1`.
fn digits_at(input: &mut Input, path: &[Seg], value: Option<&Value>) -> Option<i64> {
    let text = input.string_at(path, value, Str::default(), true)?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        input.issue(
            path,
            "invalid_format",
            "Invalid string: must match pattern /^\\d+$/",
        );
        return None;
    }
    Some(text.parse().unwrap_or(-1))
}

/// A `.nullish()` digits key: absent = `None`, `null` = `Some(None)`.
fn nullish_digits(input: &mut Input, name: &str) -> Option<Option<i64>> {
    match input.get(name).cloned() {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => digits_at(input, &key(name), Some(&value)).map(Some),
    }
}

/// The create / update body (`createBotFieldRequest`, `.partial()` when
/// `partial`).
fn field_patch(input: &mut Input, partial: bool) -> UpdateBotField {
    let name = field_name(input, !partial);
    let kind = if partial && !input.has("type") {
        None
    } else {
        let value = input.get("type").cloned();
        input.enum_at(&key("type"), value.as_ref(), CUSTOM_FIELD_TYPES, true)
    };
    let value = input.nullish_string("value", Str::trimmed(0, 1000));
    let description = input.nullish_string("description", Str::max(1000));
    let folder_id = nullish_digits(input, "folderId");
    UpdateBotField {
        name,
        r#type: kind,
        value,
        description,
        folder_id,
    }
}

/// `z.union([z.string(), z.number()]).transform(String)` (`value`).
fn value_text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Number(n)) => Some(super::number(n.as_f64()?).to_string()),
        _ => None,
    }
}

/// One `setMany` entry: `{ id, value } | { name, value } | { key, value }`.
fn entry_key(obj: &Map<String, Value>) -> Option<String> {
    let id = match obj.get("id") {
        Some(Value::String(text))
            if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) =>
        {
            Some(text.clone())
        }
        Some(Value::Number(n)) => n.as_u64().filter(|v| *v > 0).map(|v| v.to_string()),
        _ => None,
    };
    if id.is_some() {
        return id;
    }
    for name in ["name", "key"] {
        if let Some(Value::String(text)) = obj.get(name) {
            return (text.chars().count() <= 255).then(|| text.clone());
        }
    }
    None
}

/// `setMany` / `bulkUpdate` `fields`.
fn fields(input: &mut Input) -> Option<Vec<(String, String)>> {
    let path = key("fields");
    let value = input.get("fields").cloned();
    let Some(Value::Array(items)) = value else {
        let got = value.as_ref().map_or("undefined", received);
        input.issue(
            &path,
            "invalid_type",
            &format!("Invalid input: expected array, received {got}"),
        );
        return None;
    };
    let mut out = Vec::new();
    let mut ok = true;
    for (index, item) in items.iter().enumerate() {
        let parsed = item
            .as_object()
            .and_then(|obj| Some((entry_key(obj)?, value_text(obj.get("value"))?)));
        match parsed {
            Some(entry) => out.push(entry),
            None => {
                let mut item_path = path.clone();
                item_path.push(Seg::Index(index));
                input.issue(&item_path, "invalid_union", "Invalid input");
                ok = false;
            }
        }
    }
    ok.then_some(out)
}

/// `resetBotFieldsRequest.ids`.
fn reset_ids(input: &mut Input) -> Option<Vec<i64>> {
    let path = key("ids");
    let value = input.get("ids").cloned();
    let Some(Value::Array(items)) = value else {
        let got = value.as_ref().map_or("undefined", received);
        input.issue(
            &path,
            "invalid_type",
            &format!("Invalid input: expected array, received {got}"),
        );
        return None;
    };
    let mut out = Vec::new();
    let mut ok = true;
    for (index, item) in items.iter().enumerate() {
        let mut item_path = path.clone();
        item_path.push(Seg::Index(index));
        match digits_at(input, &item_path, Some(item)) {
            Some(id) => out.push(id),
            None => ok = false,
        }
    }
    if items.is_empty() {
        input.issue(
            &path,
            "too_small",
            "Too small: expected array to have >=1 items",
        );
        ok = false;
    }
    if items.len() > 100 {
        input.issue(
            &path,
            "too_big",
            "Too big: expected array to have <=100 items",
        );
        ok = false;
    }
    ok.then_some(out)
}

/// The `applyOperation` body (`publicFieldOperationFields` +
/// `requireOperandForOperation` + `clientTimezone`).
fn operation_body(input: &mut Input) -> Option<ValueOperation> {
    let names = field_operation_api_names();
    let raw = input.get("operation").cloned();
    let operation = input.enum_at(&key("operation"), raw.as_ref(), &names, true);
    let value = match input.get("value").cloned() {
        None => Some(String::new()),
        Some(value) => input.string_at(
            &key("value"),
            Some(&value),
            Str::trimmed(0, usize::MAX),
            true,
        ),
    };
    let zone = client_timezone(input);
    let (operation, value) = (operation?, value?);
    let code = field_operation_by_api_name(&operation)?;
    if is_field_operand_missing(code, Some(&value)) {
        input.issue(
            &key("value"),
            "custom",
            &format!("A value is required for `{operation}`."),
        );
        return None;
    }
    Some(ValueOperation {
        source_timezone_override: zone,
        ..ValueOperation::new("", code, &value)
    })
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "botFields.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let name = input.nullish_string("name", Str::max(usize::MAX)).flatten();
            let folder_id = nullish_digits(&mut input, "folderId").flatten();
            let sort = sort_query(&mut input, None);
            input.finish()?;
            let sort = sort.unwrap_or_else(|| {
                vec![SortItem {
                    id: "createdAt".into(),
                    desc: true,
                }]
            });
            let page = service::list(
                db,
                ws,
                ListBotFields {
                    folder_id,
                    name,
                    page: Some(page),
                    per_page: Some(per_page),
                    sort: Some(sort),
                },
            )
            .await
            .map_err(|e| business_error(e, FINDING))?;
            let data: Vec<Value> = page.data.iter().map(bot_field_json).collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "botFields.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let patch = field_patch(&mut input, false);
            input.finish()?;
            service::create(
                db,
                ws,
                CreateBotField {
                    name: patch.name.unwrap_or_default(),
                    r#type: patch.r#type.unwrap_or_default(),
                    value: patch.value.flatten(),
                    description: patch.description.flatten(),
                    folder_id: patch.folder_id.flatten(),
                },
            )
            .await
            .map(|field| json(201, bot_field_json(&field)))
            .map_err(|e| business_error(e, CREATING))
        }
        "botFields.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let field_key = id_or_name(&mut input);
            input.finish()?;
            service::find_by_key_or_fail(db, ws, &field_key)
                .await
                .map(|field| json(200, bot_field_json(&field)))
                .map_err(|e| business_error(e, FINDING))
        }
        "botFields.set" => {
            let mut input = Input::from_body(call.params, call.body);
            let field_key = id_or_name(&mut input);
            let value = input.string("value", Str::max(1000));
            input.finish()?;
            let patch = UpdateBotField {
                value: Some(value),
                ..UpdateBotField::default()
            };
            service::update_by_key(db, ws, &field_key, patch)
                .await
                .map(|field| json(200, bot_field_json(&field)))
                .map_err(|e| business_error(e, SETTING))
        }
        "botFields.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let field_key = id_or_name(&mut input);
            let patch = field_patch(&mut input, true);
            input.finish()?;
            let result = if patch.is_empty() {
                service::find_by_key_or_fail(db, ws, &field_key).await
            } else {
                service::update_by_key(db, ws, &field_key, patch).await
            };
            result
                .map(|field| json(200, bot_field_json(&field)))
                .map_err(|e| business_error(e, SETTING))
        }
        "botFields.setMany" | "botFields.bulkUpdate" => {
            let mut input = Input::from_body(call.params, call.body);
            let updates = fields(&mut input);
            input.finish()?;
            service::bulk_update_by_keys(db, ws, updates.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, SETTING))
        }
        "botFields.applyOperation" => {
            let mut input = Input::from_body(call.params, call.body);
            let field_key = id_or_name(&mut input);
            let operation = operation_body(&mut input);
            input.finish()?;
            let Some(operation) = operation else {
                return Err(business_error(
                    BusinessError::exception("Unknown operation", "invalidFieldOperation", 400),
                    SETTING,
                ));
            };
            let operation = ValueOperation {
                key: field_key,
                ..operation
            };
            service::apply_value_operation(db, ws, operation)
                .await
                .map(|field| json(200, bot_field_json(&field)))
                .map_err(|e| business_error(e, SETTING))
        }
        "botFields.reset" => {
            let mut input = Input::from_body(call.params, call.body);
            let field_key = id_or_name(&mut input);
            input.finish()?;
            service::clear_value_by_key(db, ws, &field_key)
                .await
                .map(|field| json(200, bot_field_json(&field)))
                .map_err(|e| business_error(e, MUTATING))
        }
        "botFields.resetMany" => {
            let mut input = Input::from_body(call.params, call.body);
            let ids = reset_ids(&mut input);
            input.finish()?;
            service::bulk_clear_values(db, ws, &ids.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "botFields.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let field_key = id_or_name(&mut input);
            input.finish()?;
            service::delete_by_key(db, ws, &field_key)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, DELETING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
