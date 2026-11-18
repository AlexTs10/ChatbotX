//! `features/external-webhooks/api/public.ts` (`externalWebhooksPublicRouter`,
//! scope `integrations`): Make / n8n hook registrations.

use chatbotx_business::external_webhook as service;
use chatbotx_db::repo::models::ExternalWebhook;
use serde_json::{Value, json};

use super::input::{Input, Str, key};
use super::qr_codes::public_paging;
use super::tags::digits_at;
use super::{ApiState, CREATING, Call, LISTING, MUTATING, Reply, business_error, json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "externalWebhooks.list",
    "externalWebhooks.create",
    "externalWebhooks.delete",
];

fn resource(row: &ExternalWebhook) -> Value {
    json!({
        "id": row.id.to_string(),
        "provider": row.provider,
        "event": row.event,
        "url": row.url,
    })
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "externalWebhooks.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let rows = service::list(db, ws)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let start = ((page - 1) * per_page).max(0) as usize;
            let page_count = ((rows.len() as i64 + per_page - 1) / per_page).max(1);
            let data: Vec<Value> = rows
                .iter()
                .skip(start)
                .take(per_page as usize)
                .map(resource)
                .collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "externalWebhooks.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let url = input.string(
                "url",
                Str {
                    trim: true,
                    ..Str::default()
                },
            );
            if let Some(text) = &url
                && url::Url::parse(text).is_err()
            {
                input.issue(&key("url"), "invalid_format", "Invalid URL");
            }
            let event = input.string("event", Str::trimmed(1, 100));
            let value = input.get("provider").cloned();
            let provider = match value {
                None => Some("make".to_string()),
                Some(v) => input.enum_at(&key("provider"), Some(&v), &["make", "n8n"], true),
            };
            input.finish()?;
            service::register(
                db,
                ws,
                provider.unwrap_or_default(),
                event.unwrap_or_default(),
                url.unwrap_or_default(),
            )
            .await
            .map(|row| json(201, resource(&row)))
            .map_err(|e| business_error(e, CREATING))
        }
        "externalWebhooks.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let value = input.get("id").cloned();
            let id = digits_at(&mut input, &key("id"), value.as_ref(), true).unwrap_or_default();
            input.finish()?;
            service::unregister(db, ws, id)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
