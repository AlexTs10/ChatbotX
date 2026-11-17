//! `features/inboxes/api/public.ts` (`inboxesPublicRouter`, scope `inbox`):
//! `GET /v1/inboxes`, `PATCH /v1/inboxes/{id}` and the deprecated
//! `GET /v1/channels`.

use chatbotx_business::inbox::service;
use chatbotx_db::repo::models::Inbox;
use serde_json::{Value, json};

use super::contacts::bracket_query;
use super::input::{Input, Num};
use super::tags::array;
use super::{ApiState, Call, LISTING, MUTATING, Reply, business_error, id, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &["inboxes.list", "inboxes.update", "inboxes.listChannels"];

/// `publicInboxResource`.
pub fn inbox_json(inbox: &Inbox) -> Value {
    json!({
        "id": id(inbox.id),
        "name": inbox.name,
        "channel": inbox.channel,
        "status": inbox.status,
        "sourceId": inbox.source_id,
        "markReadOnOutbound": inbox.mark_read_on_outbound,
    })
}

/// `publishInboxesRequest`: `includes?: "integration"[]`, `page?`, `perPage?`
/// (`z.coerce.number().int().min(1)`, no max).
fn list_input(call: &Call<'_>) -> Result<(Option<i64>, Option<i64>), crate::orpc::OrpcError> {
    let mut input = bracket_query(call.params, call.query);
    if input.has("includes") {
        array(&mut input, "includes", 0, usize::MAX, |input, path, value| {
            if value.as_str() == Some("integration") {
                Some(())
            } else {
                input.issue(
                    path,
                    "invalid_value",
                    "Invalid input: expected \"integration\"",
                );
                None
            }
        });
    }
    let min1 = Num {
        min: Some(1.0),
        ..Num::default()
    };
    let page = input.opt_coerce_int("page", min1);
    let per_page = input.opt_coerce_int("perPage", min1);
    input.finish()?;
    Ok((page, per_page))
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    match route_id {
        "inboxes.list" | "inboxes.listChannels" => {
            let (page, per_page) = list_input(&call)?;
            let (rows, page_count) = service::list(&state.db, ws, page, per_page)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = if route_id == "inboxes.list" {
                rows.iter().map(inbox_json).collect()
            } else {
                // `publicListInboxesResponse`: the external `sourceId` as `id`.
                rows.iter()
                    .map(|inbox| {
                        json!({
                            "id": inbox.source_id,
                            "name": inbox.name,
                            "channel": inbox.channel,
                            "status": inbox.status,
                        })
                    })
                    .collect()
            };
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "inboxes.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let inbox_id = input.digits_id("id");
            let enabled = input.opt_bool("markReadOnOutbound");
            input.finish()?;
            let inbox_id = inbox_id.unwrap_or_default();
            let inbox = match enabled {
                None => service::find_by_id_or_fail(&state.db, ws, inbox_id).await,
                Some(enabled) => {
                    service::update_mark_read_on_outbound(&state.db, ws, inbox_id, enabled).await
                }
            }
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, inbox_json(&inbox)))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
