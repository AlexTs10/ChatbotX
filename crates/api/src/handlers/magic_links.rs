//! `features/magic-links/api/public.ts` (`magicLinksPublicRouter`, scope
//! `automation`), plus the request checks the `/rpc` actions share.

use chatbotx_business::magic_link as service;
use chatbotx_db::repo::models::MagicLink;
use serde_json::{Value, json};

use super::input::{Input, Str, key};
use super::qr_codes::public_paging;
use super::tags::digits_at;
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "magicLinks.list",
    "magicLinks.create",
    "magicLinks.get",
    "magicLinks.update",
    "magicLinks.delete",
];

/// `/^([a-zA-Z0-9._-]+\/)*[a-zA-Z0-9._-]+$/`.
fn is_path_segment(text: &str) -> bool {
    !text.is_empty()
        && text.split('/').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
}

/// `createMagicLinkRequest` (`partial` for an update): `(name, url)`.
pub fn request(input: &mut Input, partial: bool) -> (Option<String>, Option<String>) {
    let name_spec = Str {
        min: Some(1),
        max: Some(128),
        pattern: Some(is_path_segment),
        ..Str::default()
    };
    let name = if partial && !input.has("name") {
        None
    } else {
        input.string("name", name_spec)
    };
    let url = if partial && !input.has("url") {
        None
    } else {
        input.string("url", Str::max(1000))
    };
    if let Some(text) = &url
        && url::Url::parse(text).is_err()
    {
        input.issue(&key("url"), "invalid_format", "Invalid URL");
    }
    (name, url)
}

/// `buildMagicLinkUrl`: `<appUrl>/r/<workspaceId>/<name>`.
pub fn link_url(app_url: &str, workspace_id: i64, name: &str) -> String {
    url::Url::parse(app_url)
        .and_then(|base| base.join(&format!("/r/{workspace_id}/{name}")))
        .map_or_else(
            |_| format!("{}/r/{workspace_id}/{name}", app_url.trim_end_matches('/')),
            String::from,
        )
}

/// `magicLinkPublicResource`: no `workspaceId`, `url` is the public link.
pub fn public_json(state: &ApiState, row: &MagicLink) -> Value {
    let mut out = row.to_json();
    if let Some(obj) = out.as_object_mut() {
        obj.remove("workspaceId");
        obj.insert(
            "url".into(),
            Value::String(link_url(&state.app_url, row.workspace_id, &row.name)),
        );
    }
    out
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
        "magicLinks.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            // `.nullish()`: a JSON-less query sends text only.
            let keyword = input.opt_string("keyword", Str::default());
            let sort = super::qr_codes::sort_query(&mut input, None);
            input.finish()?;
            let (rows, page_count) = service::list(db, ws, keyword, sort, page, per_page)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = rows.iter().map(|r| public_json(state, r)).collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "magicLinks.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = path_id(&mut input);
            input.finish()?;
            service::find_or_fail(db, ws, id)
                .await
                .map(|row| json(200, public_json(state, &row)))
                .map_err(|e| business_error(e, FINDING))
        }
        "magicLinks.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let (name, url) = request(&mut input, false);
            input.finish()?;
            service::create(db, ws, name.unwrap_or_default(), url.unwrap_or_default())
                .await
                .map(|row| json(201, public_json(state, &row)))
                .map_err(|e| business_error(e, CREATING))
        }
        "magicLinks.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let (name, url) = request(&mut input, true);
            let id = path_id(&mut input);
            input.finish()?;
            service::update(db, ws, id, name, url)
                .await
                .map(|row| json(200, public_json(state, &row)))
                .map_err(|e| business_error(e, MUTATING))
        }
        "magicLinks.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = path_id(&mut input);
            input.finish()?;
            service::delete(db, ws, id)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
