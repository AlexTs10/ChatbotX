//! `features/saved-replies/api/public.ts` (`savedRepliesPublicRouter`, scope
//! `inbox`), plus the request checks the `/rpc` actions share.
//!
//! ponytail: no `assertWorkspaceNotBlocked` call: the workspace-token gate
//! already refuses POST / PUT for a blocked owner (`isCloud()` only).

use chatbotx_business::saved_reply::{self as service, Update};
use serde_json::{Value, json};

use super::input::{Input, Seg, Str, key};
use super::qr_codes::public_paging;
use super::tags::{array, digits_at};
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, json, no_content,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "savedReplies.list",
    "savedReplies.create",
    "savedReplies.get",
    "savedReplies.update",
    "savedReplies.delete",
];

/// `savedReplyFields` (+ the create `refine` when `create`):
/// `(shortcut, text, imageFileIds)`.
pub fn fields(input: &mut Input, create: bool) -> (String, Option<String>, Option<Vec<i64>>) {
    let shortcut = input.string("shortcut", Str::trimmed(1, 100));
    let text = input.opt_string(
        "text",
        Str {
            trim: true,
            ..Str::max(2000)
        },
    );
    let images = input.has("imageFileIds").then(|| {
        array(input, "imageFileIds", 0, 10, |input, path, value| {
            digits_at(input, path, Some(value), true)
        })
    });
    let images = images.flatten();
    if create
        && !input.failed()
        && text.as_deref().is_none_or(str::is_empty)
        && images.as_ref().is_none_or(Vec::is_empty)
    {
        input.issue(
            &key("text"),
            "custom",
            "A saved reply needs text or at least one image",
        );
    }
    (shortcut.unwrap_or_default(), text, images)
}

fn reply_id(input: &mut Input) -> i64 {
    let value = input.get("id").cloned();
    digits_at(input, &[Seg::Key("id".into())], value.as_ref(), true).unwrap_or_default()
}

/// `{ data }` of `listSavedReplies`.
pub async fn list(state: &ApiState, workspace_id: i64) -> chatbotx_business::Result<Vec<Value>> {
    let storage = super::contacts::tenant_urls(state).storage_url;
    Ok(service::list_by_workspace_id(&state.db, &storage, workspace_id)
        .await?
        .iter()
        .map(service::WithImages::to_json)
        .collect())
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let storage = super::contacts::tenant_urls(state).storage_url;
    match route_id {
        "savedReplies.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let rows = list(state, ws)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let start = ((page - 1) * per_page).max(0) as usize;
            let page_count = ((rows.len() as i64 + per_page - 1) / per_page).max(1);
            let data: Vec<Value> = rows.into_iter().skip(start).take(per_page as usize).collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "savedReplies.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = reply_id(&mut input);
            input.finish()?;
            service::find_by_id_or_fail(db, &storage, ws, id)
                .await
                .map(|row| json(200, row.to_json()))
                .map_err(|e| business_error(e, FINDING))
        }
        "savedReplies.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let (shortcut, text, images) = fields(&mut input, true);
            input.finish()?;
            service::create(db, &storage, ws, shortcut, text, images.unwrap_or_default())
                .await
                .map(|row| json(201, row.to_json()))
                .map_err(|e| business_error(e, CREATING))
        }
        "savedReplies.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let (shortcut, text, image_file_ids) = fields(&mut input, false);
            let id = reply_id(&mut input);
            input.finish()?;
            let data = Update {
                shortcut,
                text,
                image_file_ids,
            };
            service::update(db, &storage, ws, id, data)
                .await
                .map(|row| json(200, row.to_json()))
                .map_err(|e| business_error(e, MUTATING))
        }
        "savedReplies.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = reply_id(&mut input);
            input.finish()?;
            service::delete(db, ws, id)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
