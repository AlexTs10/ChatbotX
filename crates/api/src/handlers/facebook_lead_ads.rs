//! `features/facebook-lead-ad-automation/api/public.ts`: `facebookLeadAds.*`
//! (scope `automation`).

use chatbotx_business::facebook_lead_ads::{self as service, CreateInput};
use serde_json::{Map, Value, json};

use super::contacts::sort_items;
use super::input::{Input, Str, key};
use super::qr_codes::public_paging;
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
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let valid = ["key", "label", "type"]
            .iter()
            .all(|k| item.get(k).is_some_and(Value::is_string))
            && item
                .get("target")
                .is_some_and(|t| t.is_string() || t.is_null());
        if !valid {
            input.issue(&key("fieldMapping"), "invalid_type", "Invalid input");
            return None;
        }
        let mut entry = Map::new();
        for k in ["key", "label", "type", "target"] {
            entry.insert(k.to_owned(), item[k].clone());
        }
        out.push(Value::Object(entry));
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
            let sort = sort_items(input.get("sort"));
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
