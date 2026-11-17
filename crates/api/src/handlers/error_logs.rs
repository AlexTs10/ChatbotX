//! `features/error-logs/api/public.ts`: `GET /v1/error-logs` (Q.D11, scope
//! `analytics`).

use chatbotx_business::error_log::{self, ListInput};
use chatbotx_db::repo::SortItem;
use chatbotx_db::repo::models::ErrorLog;
use serde_json::{Value, json};

use super::input::{Input, Str};
use super::qr_codes::public_paging;
use super::{ApiState, Call, LISTING, Reply, business_error, date, id, json, opt_id};

/// The routes of this module.
pub const ROUTES: &[&str] = &["errorLogs.list"];

/// `publicListErrorLogsResponse` item.
pub fn public_error_log_json(row: &ErrorLog) -> Value {
    json!({
        "id": id(row.id),
        "workspaceId": id(row.workspace_id),
        "contactId": opt_id(row.contact_id),
        "action": row.action,
        "detail": row.detail,
        "httpCode": row.http_code,
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
    })
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    if route_id != ROUTES[0] {
        return Err(crate::routes::not_ported_error());
    }
    let mut input = Input::from_query(call.params, call.query);
    let (page, per_page) = public_paging(&mut input);
    let keyword = input.opt_string("keyword", Str::default());
    input.finish()?;
    let page = error_log::list(
        &state.db,
        ListInput {
            workspace_id: call.workspace_id,
            page: Some(page),
            per_page: Some(per_page),
            // The public route pins newest first.
            sort: Some(vec![SortItem {
                id: "createdAt".into(),
                desc: true,
            }]),
            keyword,
        },
    )
    .await
    .map_err(|e| business_error(e, LISTING))?;
    let data: Vec<Value> = page
        .data
        .iter()
        .map(|row| public_error_log_json(&row.log))
        .collect();
    Ok(json(
        200,
        json!({ "data": data, "pageCount": page.page_count }),
    ))
}
