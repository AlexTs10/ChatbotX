//! `apps/builder/src/features/contacts/api/public/custom-fields.ts`
//! (`contactsCustomFieldsPublicRouter`, P3.6-D2 slice contact custom-field
//! values), with `schema/public/custom-fields.ts`,
//! `lib/public-api/field-operation.ts` and `lib/public-api/iana-timezone.ts`.

use chatbotx_business::contact_custom_field::{
    self as service, FieldOperation, FieldOperationType,
};
use chatbotx_business::{BusinessError, custom_field};
use chatbotx_core::flow_config::steps::is_field_operand_missing;
use chatbotx_core::utils::custom_field_operation::{
    field_operation_api_names, field_operation_by_api_name,
};
use chatbotx_core::utils::tz::is_intl_time_zone;
use chatbotx_db::repo::contact::ContactCustomFieldWithDefinition;
use serde_json::{Value, json};

use super::input::{Input, Seg, Str, child, key};
use super::{ApiState, Call, ErrorMap, FINDING, MUTATING, Reply, business_error, id, no_content};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "contacts.listCustomFields",
    "contacts.getCustomField",
    "contacts.setCustomField",
    "contacts.setCustomFieldLegacy",
    "contacts.applyCustomFieldOperations",
    "contacts.clearCustomField",
    "contacts.clearCustomFields",
];

/// `possibleErrorsOnWritingContactFields`.
const WRITING_FIELDS: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("invalidCustomFieldValue", 400),
    ("invalidFieldOperation", 400),
];

/// `z.string().min(1)`.
const NON_EMPTY: Str = Str {
    trim: false,
    min: Some(1),
    max: None,
    pattern: None,
};
/// `z.string().trim()`.
const TRIMMED: Str = Str {
    trim: true,
    min: None,
    max: None,
    pattern: None,
};

/// At most 20 operations per request.
const MAX_OPERATIONS: usize = 20;

/// `publicContactCustomFieldResource`.
fn value_json(row: &ContactCustomFieldWithDefinition) -> Value {
    json!({
        "id": id(row.custom_field.id),
        "name": row.custom_field.name,
        "type": row.custom_field.r#type,
        "description": row.custom_field.description,
        "value": row.value.value,
    })
}

/// `ianaTimezoneSchema.optional()` at `path`.
pub(super) fn client_timezone(input: &mut Input) -> Option<String> {
    let path = key("clientTimezone");
    let value = input.get("clientTimezone").cloned();
    let zone = input.string_at(&path, value.as_ref(), Str::trimmed(1, 255), false)?;
    if !is_intl_time_zone(&zone) {
        input.issue(&path, "custom", "Invalid IANA timezone");
        return None;
    }
    Some(zone)
}

/// The `idOrName` path parameter (`z.string().min(1)`).
fn id_or_name(input: &mut Input) -> Option<String> {
    input.string("idOrName", NON_EMPTY)
}

pub(super) fn received(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// One `contactCustomFieldOperationPublicRequest` (with
/// `requireOperandForOperation`).
fn operation_at(
    input: &mut Input,
    path: &[Seg],
    item: &Value,
) -> Option<(String, FieldOperationType, String)> {
    let Value::Object(obj) = item else {
        input.issue(
            path,
            "invalid_type",
            &format!(
                "Invalid input: expected object, received {}",
                received(item)
            ),
        );
        return None;
    };
    let field = input.string_at(
        &child(path, "customFieldId"),
        obj.get("customFieldId"),
        NON_EMPTY,
        true,
    );
    let names = field_operation_api_names();
    let operation = input.enum_at(
        &child(path, "operation"),
        obj.get("operation"),
        &names,
        true,
    );
    let value = match obj.get("value") {
        None => Some(String::new()),
        some => input.string_at(&child(path, "value"), some, TRIMMED, true),
    };
    let (field, operation, value) = (field?, operation?, value?);
    let code = field_operation_by_api_name(&operation)?;
    if is_field_operand_missing(code, Some(&value)) {
        input.issue(
            &child(path, "value"),
            "custom",
            &format!("A value is required for `{operation}`."),
        );
        return None;
    }
    Some((field, code, value))
}

/// `addContactCustomFieldOperationsPublicRequest.operations`.
fn operations(input: &mut Input) -> Option<Vec<(String, FieldOperationType, String)>> {
    let path = key("operations");
    let Some(value) = input.get("operations").cloned() else {
        input.issue(
            &path,
            "invalid_type",
            "Invalid input: expected array, received undefined",
        );
        return None;
    };
    let Value::Array(items) = value else {
        input.issue(
            &path,
            "invalid_type",
            &format!(
                "Invalid input: expected array, received {}",
                received(&value)
            ),
        );
        return None;
    };
    let mut out = Vec::new();
    let mut ok = true;
    for (index, item) in items.iter().enumerate() {
        let mut item_path = path.clone();
        item_path.push(Seg::Index(index));
        match operation_at(input, &item_path, item) {
            Some(op) => out.push(op),
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
    if items.len() > MAX_OPERATIONS {
        input.issue(
            &path,
            "too_big",
            &format!("Too big: expected array to have <={MAX_OPERATIONS} items"),
        );
        ok = false;
    }
    ok.then_some(out)
}

fn not_found(message: &str, errors: ErrorMap) -> crate::orpc::OrpcError {
    business_error(BusinessError::not_found(message), errors)
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "contacts.listCustomFields" => {
            let contact_id = super::contacts::resolve(state, &call, FINDING).await?;
            let rows = service::list_with_definition_by_contact(db, ws, contact_id)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            let data: Vec<Value> = rows.iter().map(value_json).collect();
            Ok(super::json(200, json!({ "data": data })))
        }
        "contacts.getCustomField" => {
            let mut input = Input::from_query(call.params, call.query);
            let key = id_or_name(&mut input);
            input.finish()?;
            let contact_id = super::contacts::resolve(state, &call, FINDING).await?;
            let field = custom_field::find_by_key_or_fail(db, ws, &key.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, FINDING))?;
            let row = service::find_with_definition(db, ws, contact_id, field.id)
                .await
                .map_err(|e| business_error(e, FINDING))?
                .ok_or_else(|| not_found("Contact custom field not found", FINDING))?;
            Ok(super::json(200, value_json(&row)))
        }
        "contacts.setCustomField" => {
            let mut input = Input::from_body(call.params, call.body);
            let key = id_or_name(&mut input);
            let value = input.string("value", TRIMMED);
            let zone = client_timezone(&mut input);
            input.finish()?;
            let contact_id = super::contacts::resolve(state, &call, WRITING_FIELDS).await?;
            let field = custom_field::find_by_key_or_fail(db, ws, &key.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, WRITING_FIELDS))?;
            service::set_value_for_contact(
                db,
                ws,
                contact_id,
                field.id,
                &value.unwrap_or_default(),
                zone.as_deref(),
            )
            .await
            .map(|_| no_content())
            .map_err(|e| business_error(e, WRITING_FIELDS))
        }
        "contacts.setCustomFieldLegacy" => {
            let mut input = Input::from_body(call.params, call.body);
            let field = bigint_param(&mut input, "customFieldId");
            let value = input.string("value", TRIMMED);
            input.finish()?;
            let contact_id = super::contacts::resolve(state, &call, MUTATING).await?;
            service::set_value_for_contact(
                db,
                ws,
                contact_id,
                field.unwrap_or(-1),
                &value.unwrap_or_default(),
                None,
            )
            .await
            .map(|_| no_content())
            .map_err(|e| business_error(e, MUTATING))
        }
        "contacts.applyCustomFieldOperations" => {
            let mut input = Input::from_body(call.params, call.body);
            let requested = operations(&mut input);
            let zone = client_timezone(&mut input);
            input.finish()?;
            let requested = requested.unwrap_or_default();
            let contact_id = super::contacts::resolve(state, &call, WRITING_FIELDS).await?;
            let keys: Vec<String> = requested.iter().map(|(key, ..)| key.clone()).collect();
            let fields = chatbotx_business::tx::read(db, move |conn| {
                custom_field::find_many_by_keys_in(conn, ws, &keys)
            })
            .await
            .map_err(|e| business_error(e, WRITING_FIELDS))?;
            let mut ops = Vec::with_capacity(requested.len());
            for (key, operation, value) in requested {
                let field = fields
                    .get(&key)
                    .ok_or_else(|| not_found("Custom field not found", WRITING_FIELDS))?;
                ops.push(FieldOperation {
                    custom_field_id: field.id,
                    operation,
                    value,
                });
            }
            service::apply_operations(db, ws, contact_id, ops, zone.as_deref())
                .await
                .map(|_| no_content())
                .map_err(|e| business_error(e, WRITING_FIELDS))
        }
        "contacts.clearCustomField" => {
            let mut input = Input::from_query(call.params, call.query);
            let key = id_or_name(&mut input);
            input.finish()?;
            let contact_id = super::contacts::resolve(state, &call, MUTATING).await?;
            service::delete_by_key(db, ws, contact_id, &key.unwrap_or_default(), false)
                .await
                .map(|_| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "contacts.clearCustomFields" => {
            let contact_id = super::contacts::resolve(state, &call, MUTATING).await?;
            service::clear_by_contact_id(db, ws, contact_id)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}

/// `zodBigintAsString()`: digits only; beyond `i64` = `-1` (no row).
fn bigint_param(input: &mut Input, name: &str) -> Option<i64> {
    let value = input.get(name).cloned();
    let text = input.string_at(&key(name), value.as_ref(), Str::default(), true)?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        input.issue(
            &key(name),
            "invalid_format",
            "Invalid string: must match pattern /^\\d+$/",
        );
        return None;
    }
    Some(text.parse().unwrap_or(-1))
}
