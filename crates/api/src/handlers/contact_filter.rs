//! `apps/builder/src/features/contact-filter/api/public.ts`
//! (`contactsFilterFieldsPublicRouter`, P3.6-D2 slice filter fields):
//! `contacts.listFilterFields` and `contacts.resolveFilterValueLabels`.

use chatbotx_business::contact::filter_fields::list_contact_filter_fields_for_api;
use chatbotx_business::contact_filter::Surface;
use chatbotx_business::tag::find_many_by_ids;
use serde_json::{Value, json};

use super::input::{Input, Seg, key};
use super::{ApiState, Call, ErrorMap, LISTING, Reply, business_error, id, json};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "contacts.listFilterFields",
    "contacts.resolveFilterValueLabels",
];

/// `MAX_FILTER_VALUE_LABEL_IDS`.
const MAX_FILTER_VALUE_LABEL_IDS: usize = 500;

/// `possibleErrorsOnListingResource` + `possibleIdempotencyErrors`.
const LISTING_IDEMPOTENT: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    match route_id {
        "contacts.listFilterFields" => list_filter_fields(state, call).await,
        "contacts.resolveFilterValueLabels" => resolve_filter_value_labels(state, call).await,
        _ => Err(crate::routes::not_ported_error()),
    }
}

/// `GET /v1/contacts/filter-fields` (`listContactFilterFieldsPublicRequest`).
async fn list_filter_fields(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let raw = input.get("surface").cloned();
    let surface = input.enum_at(
        &key("surface"),
        raw.as_ref(),
        &["contacts", "flowCondition"],
        false,
    );
    input.finish()?;
    let surface = match surface.as_deref() {
        Some("flowCondition") => Surface::FlowCondition,
        _ => Surface::Contacts,
    };
    let tracked = chatbotx_db::repo::keys::keys().contact_activity_filters_enabled;
    let body = list_contact_filter_fields_for_api(&state.db, call.workspace_id, surface, tracked)
        .await
        .map_err(|e| business_error(e, LISTING))?;
    Ok(json(200, body))
}

/// `isFilterValueId`: 1-19 digits, no leading zero, within the bigint range.
fn is_filter_value_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 19
        && text.bytes().all(|b| b.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'))
        && text.parse::<i64>().is_ok()
}

/// `POST /v1/contacts/filter-value-labels`
/// (`resolveFilterValueLabelsPublicRequest`: `tags` only; other keys are
/// stripped).
async fn resolve_filter_value_labels(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let path = key("tags");
    let mut ids: Vec<i64> = Vec::new();
    match input.get("tags").cloned() {
        None => {}
        Some(Value::Array(items)) => {
            for (index, item) in items.iter().enumerate() {
                let mut item_path = path.clone();
                item_path.push(Seg::Index(index));
                match item {
                    Value::String(text) if is_filter_value_id(text) => {
                        if let Ok(value) = text.parse::<i64>()
                            && !ids.contains(&value)
                        {
                            ids.push(value);
                        }
                    }
                    Value::String(_) => input.issue(&item_path, "custom", "Invalid id"),
                    other => input.issue(
                        &item_path,
                        "invalid_type",
                        &format!(
                            "Invalid input: expected string, received {}",
                            type_name(other)
                        ),
                    ),
                }
            }
            if items.len() > MAX_FILTER_VALUE_LABEL_IDS {
                input.issue(
                    &path,
                    "too_big",
                    &format!(
                        "Too big: expected array to have <={MAX_FILTER_VALUE_LABEL_IDS} items"
                    ),
                );
            }
        }
        Some(other) => input.issue(
            &path,
            "invalid_type",
            &format!(
                "Invalid input: expected array, received {}",
                type_name(&other)
            ),
        ),
    }
    input.finish()?;
    let tags = find_many_by_ids(&state.db, call.workspace_id, &ids)
        .await
        .map_err(|e| business_error(e, LISTING_IDEMPOTENT))?;
    let tags: Vec<Value> = tags
        .iter()
        .map(|tag| json!({ "id": id(tag.id), "name": tag.name }))
        .collect();
    Ok(json(200, json!({ "tags": tags })))
}

/// zod's `received` name of a JSON value.
fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
