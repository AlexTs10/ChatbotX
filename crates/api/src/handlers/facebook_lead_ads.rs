//! `features/facebook-lead-ad-automation/api/public.ts`: `facebookLeadAds.*`
//! (scope `automation`).

use chatbotx_business::facebook_lead_ads::{self as service, CreateInput};
use serde_json::{Map, Value, json};

use super::input::{Input, Seg, Str, child, key, type_name};
use super::qr_codes::{public_paging, sort_query};
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "facebookLeadAds.list",
    "facebookLeadAds.create",
    "facebookLeadAds.get",
    "facebookLeadAds.update",
    "facebookLeadAds.delete",
    "facebookLeadAds.listPages",
    "facebookLeadAds.listForms",
];

/// `facebookLeadFieldMappingSchema[]`: `{ key, label, type, target: string | null }`
/// (unknown keys stripped). `None` (with an issue) when invalid.
fn field_mapping(input: &mut Input, value: &Value) -> Option<Value> {
    let Value::Array(items) = value else {
        input.issue(
            &key("fieldMapping"),
            "invalid_type",
            "Invalid input: expected array",
        );
        return None;
    };
    let before = input.issue_count();
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let path = vec![Seg::Key("fieldMapping".into()), Seg::Index(index)];
        let Value::Object(item) = item else {
            let message = format!(
                "Invalid input: expected object, received {}",
                type_name(Some(item))
            );
            input.issue(&path, "invalid_type", &message);
            continue;
        };
        let mut entry = Map::new();
        for k in ["key", "label", "type", "target"] {
            let value = item.get(k);
            if k == "target" && value.is_some_and(Value::is_null) {
                entry.insert(k.to_owned(), Value::Null);
            } else if let Some(text) =
                input.string_at(&child(&path, k), value, Str::default(), true)
            {
                entry.insert(k.to_owned(), Value::String(text));
            }
        }
        out.push(Value::Object(entry));
    }
    if input.issue_count() > before {
        return None;
    }
    Some(Value::Array(out))
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "facebookLeadAds.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let keyword = input.opt_string("keyword", Str::default());
            let sort = sort_query(&mut input, None);
            input.finish()?;
            let (rows, page_count) = service::list(db, ws, page, per_page, keyword, sort)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = rows.iter().map(service::resource).collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "facebookLeadAds.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.id("id");
            input.finish()?;
            service::get(db, ws, id.unwrap_or_default())
                .await
                .map(|row| json(200, service::resource(&row)))
                .map_err(|e| business_error(e, FINDING))
        }
        "facebookLeadAds.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.string(
                "name",
                Str {
                    min: Some(1),
                    max: Some(100),
                    ..Str::default()
                },
            );
            let page_id = input.string(
                "pageId",
                Str {
                    min: Some(1),
                    ..Str::default()
                },
            );
            let page_name = input.nullish_string("pageName", Str::default());
            let form_id = input.string(
                "formId",
                Str {
                    min: Some(1),
                    ..Str::default()
                },
            );
            let form_name = input.nullish_string("formName", Str::default());
            let mapping = match input.get("fieldMapping").cloned() {
                None => Some(json!([])),
                Some(value) => field_mapping(&mut input, &value),
            };
            input.finish()?;
            service::create(
                db,
                ws,
                CreateInput {
                    name: name.unwrap_or_default(),
                    page_id: page_id.unwrap_or_default(),
                    page_name: page_name.flatten(),
                    form_id: form_id.unwrap_or_default(),
                    form_name: form_name.flatten(),
                    field_mapping: mapping.unwrap_or_default(),
                },
            )
            .await
            .map(|row| json(201, service::resource(&row)))
            .map_err(|e| business_error(e, CREATING))
        }
        "facebookLeadAds.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.id("id");
            let name = input.opt_string(
                "name",
                Str {
                    min: Some(1),
                    max: Some(100),
                    ..Str::default()
                },
            );
            let mapping = match input.get("fieldMapping").cloned() {
                None => None,
                Some(value) => field_mapping(&mut input, &value),
            };
            input.finish()?;
            service::update(db, ws, id.unwrap_or_default(), name, mapping)
                .await
                .map(|row| json(200, service::resource(&row)))
                .map_err(|e| business_error(e, MUTATING))
        }
        "facebookLeadAds.delete" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.id("id");
            input.finish()?;
            service::delete(db, ws, id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "facebookLeadAds.listPages" => service::list_pages(db, ws)
            .await
            .map(|pages| json(200, json!({ "pages": pages })))
            .map_err(|e| business_error(e, LISTING)),
        "facebookLeadAds.listForms" => {
            let mut input = Input::from_query(call.params, call.query);
            let page_id = input.string("pageId", Str::default());
            input.finish()?;
            service::list_forms(db, ws, page_id.unwrap_or_default())
                .await
                .map(|forms| json(200, json!({ "forms": forms })))
                .map_err(|e| business_error(e, LISTING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
