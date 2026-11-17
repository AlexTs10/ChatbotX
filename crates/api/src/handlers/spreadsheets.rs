//! `features/spreadsheets/api/public.ts` (`spreadsheetsPublicRouter`, scope
//! `automation`), plus the request check the `/rpc` actions share.
//! `listWorksheets` / `listWorksheetHeaders` need the Google Sheets client
//! (Q.I4) and answer "not ported" until then.

use chatbotx_business::spreadsheet as service;
use serde_json::json;

use super::input::{Input, Str, key};
use super::qr_codes::public_paging;
use super::tags::digits_at;
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "spreadsheets.list",
    "spreadsheets.create",
    "spreadsheets.get",
    "spreadsheets.update",
    "spreadsheets.delete",
];

/// `createSpreadsheetRequest`: `(name, url)`.
pub fn request(input: &mut Input) -> (String, String) {
    let name = input.string(
        "name",
        Str {
            min: Some(1),
            ..Str::max(255)
        },
    );
    let url = input.string("url", Str::default());
    if let Some(text) = &url {
        if url::Url::parse(text).is_err() {
            input.issue(&key("url"), "invalid_format", "Invalid URL");
        } else if !service::is_google_spreadsheet_url(text) {
            input.issue(
                &key("url"),
                "custom",
                "URL must be a valid Google Spreadsheet link",
            );
        }
    }
    (name.unwrap_or_default(), url.unwrap_or_default())
}

fn path_id(input: &mut Input) -> i64 {
    let value = input.get("id").cloned();
    digits_at(input, &key("id"), value.as_ref(), true).unwrap_or_default()
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "spreadsheets.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let name = input.opt_string("name", Str::default());
            input.finish()?;
            let (rows, page_count) = service::list(db, ws, name, page, Some(per_page))
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<_> = rows.iter().map(service::public_json).collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "spreadsheets.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = path_id(&mut input);
            input.finish()?;
            service::find_or_fail(db, ws, id)
                .await
                .map(|row| json(200, service::public_json(&row)))
                .map_err(|e| business_error(e, FINDING))
        }
        "spreadsheets.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let (name, url) = request(&mut input);
            input.finish()?;
            service::create(db, ws, name, url)
                .await
                .map(|id| json(201, json!({ "id": id.to_string() })))
                .map_err(|e| business_error(e, CREATING))
        }
        "spreadsheets.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let (name, url) = request(&mut input);
            let id = path_id(&mut input);
            input.finish()?;
            service::update(db, ws, id, name, url)
                .await
                .map(|row| json(200, service::public_json(&row)))
                .map_err(|e| business_error(e, MUTATING))
        }
        "spreadsheets.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = path_id(&mut input);
            input.finish()?;
            service::delete_many(db, ws, vec![id])
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
