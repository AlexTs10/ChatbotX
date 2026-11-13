//! `apps/builder/src/features/custom-fields/api/public.ts`
//! (`customFieldsPublicRouter`).

use chatbotx_business::custom_field::{
    self as service, CreateCustomField, ListCustomFields, UpdateCustomField,
};
use chatbotx_db::repo::SortItem;
use chatbotx_db::repo::models::CustomField;
use serde_json::{Value, json};

use super::input::{Input, Str, key};
use super::qr_codes::{public_paging, sort_query};
use super::{
    ApiState, Call, ErrorMap, FINDING, LISTING, MUTATING, Reply, business_error, date, id, json,
    no_content, opt_id,
};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "customFields.list",
    "customFields.create",
    "customFields.get",
    "customFields.update",
    "customFields.delete",
];

/// `customFieldTypes`.
pub const CUSTOM_FIELD_TYPES: &[&str] = &[
    "shortText",
    "email",
    "phoneNumber",
    "number",
    "date",
    "datetime",
    "boolean",
    "longText",
];

/// `RESERVED_FIELD_REFERENCE_PREFIX` (`bot_field:`).
const RESERVED_PREFIX: &str = "bot_field:";

/// `possibleErrorsOnCreatingInFolder`.
const CREATING_IN_FOLDER: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("notFound", 404),
];

/// `publicCustomFieldDefinitionResource`.
pub fn custom_field_json(field: &CustomField) -> Value {
    json!({
        "id": id(field.id),
        "name": field.name,
        "type": field.r#type,
        "description": field.description,
        "folderId": opt_id(field.folder_id),
        "showInInbox": field.show_in_inbox,
        "createdAt": date(field.created_at),
        "updatedAt": date(field.updated_at),
    })
}

/// `zodFieldName()`: trimmed, 1..=255, not `bot_field:`.
fn field_name(input: &mut Input) -> Option<String> {
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

/// `zodBigintAsString()` (digits, any length) at a top-level key. A value
/// beyond `i64` is `-1` (matches no row).
fn digits(input: &mut Input, name: &str, required: bool) -> Option<i64> {
    let value = input.get(name).cloned();
    let text = input.string_at(&key(name), value.as_ref(), Str::default(), required)?;
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

/// A `.nullish()` [`digits`] key: absent = `None`, `null` = `Some(None)`.
fn nullish_digits(input: &mut Input, name: &str) -> Option<Option<i64>> {
    match input.get(name) {
        None => None,
        Some(Value::Null) => Some(None),
        Some(_) => digits(input, name, true).map(Some),
    }
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "customFields.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let name = input.nullish_string("name", Str::max(usize::MAX)).flatten();
            let folder_id = nullish_digits(&mut input, "folderId").flatten();
            let sort = sort_query(&mut input, Some(&["name", "createdAt", "updatedAt"]));
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
                ListCustomFields {
                    folder_id,
                    name,
                    page: Some(page),
                    per_page: Some(per_page),
                    sort: Some(sort),
                },
            )
            .await
            .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = page.data.iter().map(custom_field_json).collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "customFields.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = field_name(&mut input);
            let kind = input.enum_at(
                &key("type"),
                input.get("type").cloned().as_ref(),
                CUSTOM_FIELD_TYPES,
                true,
            );
            let folder_id = nullish_digits(&mut input, "folderId").flatten();
            let description = input
                .nullish_string("description", Str::max(usize::MAX))
                .flatten();
            input.finish()?;
            service::create(
                db,
                ws,
                CreateCustomField {
                    name: name.unwrap_or_default(),
                    r#type: kind.unwrap_or_default(),
                    description,
                    folder_id,
                },
            )
            .await
            .map(|field| json(201, custom_field_json(&field)))
            .map_err(|e| business_error(e, CREATING_IN_FOLDER))
        }
        "customFields.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let field_key = input.string("idOrName", Str::max(usize::MAX));
            input.finish()?;
            service::find_by_key_or_fail(db, ws, &field_key.unwrap_or_default())
                .await
                .map(|field| json(200, custom_field_json(&field)))
                .map_err(|e| business_error(e, FINDING))
        }
        "customFields.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = field_name(&mut input);
            let description = input.opt_string("description", Str::max(usize::MAX));
            let folder_id = nullish_digits(&mut input, "folderId");
            let field_id = digits(&mut input, "id", true);
            input.finish()?;
            service::update(
                db,
                ws,
                &field_id.unwrap_or_default().to_string(),
                UpdateCustomField {
                    name,
                    description: description.map(Some),
                    folder_id,
                },
            )
            .await
            .map(|field| json(200, custom_field_json(&field)))
            .map_err(|e| business_error(e, MUTATING))
        }
        "customFields.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let field_id = digits(&mut input, "id", true);
            input.finish()?;
            service::delete(db, ws, &[field_id.unwrap_or_default()])
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
