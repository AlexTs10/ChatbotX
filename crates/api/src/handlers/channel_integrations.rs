//! `integration-tiktok/api/public.ts` (`tiktokChannels.*`) and
//! `integration-zalo/api/public.ts` (`zaloChannels.*`), with the shared
//! `createChannelReadRoutes` list / get (Q.I2).

use chatbotx_business::BusinessError;
use chatbotx_business::channel_integration::{self, TIKTOK_BUSINESS_API_URL};
use chatbotx_business::connection::credentials::HttpUpstream;
use serde_json::{Value, json};

use super::input::Input;
use super::{ApiState, Call, Reply, business_error, json, opt_date};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "tiktokChannels.list",
    "tiktokChannels.get",
    "tiktokChannels.getCommentToMessage",
    "tiktokChannels.updateCommentToMessage",
    "zaloChannels.list",
    "zaloChannels.get",
    "zaloChannels.updateTagSync",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let (group, action) = route_id.split_once('.').unwrap_or_default();
    let channel = if group == "tiktokChannels" {
        "tiktok"
    } else {
        "zalo"
    };
    match action {
        "list" => {
            Input::from_query(call.params, call.query).finish()?;
            let rows = channel_integration::list(db, channel, ws, None)
                .await
                .map_err(|e| business_error(e, super::LISTING))?;
            Ok(json(200, Value::Array(rows.unwrap_or_default())))
        }
        "get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.id("id");
            input.finish()?;
            let row = channel_integration::list(db, channel, ws, id)
                .await
                .and_then(|rows| {
                    rows.and_then(|mut rows| rows.pop())
                        .ok_or_else(|| BusinessError::not_found("Channel integration not found"))
                })
                .map_err(|e| business_error(e, super::FINDING))?;
            Ok(json(200, row))
        }
        "getCommentToMessage" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.id("id");
            input.finish()?;
            let status = channel_integration::refresh_comment_to_message(
                db,
                &HttpUpstream,
                TIKTOK_BUSINESS_API_URL,
                ws,
                id.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, super::FINDING))?;
            Ok(json(200, json!({ "status": status })))
        }
        "updateCommentToMessage" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.id("id");
            let enabled = input.bool("enabled");
            input.finish()?;
            let status = channel_integration::set_comment_to_message(
                db,
                &HttpUpstream,
                &state.audit,
                TIKTOK_BUSINESS_API_URL,
                ws,
                id.unwrap_or_default(),
                enabled.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, super::MUTATING))?;
            Ok(json(200, json!({ "status": status })))
        }
        _ => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.id("id");
            let enabled = input.bool("enabled");
            input.finish()?;
            let at = channel_integration::update_zalo_tag_sync(
                db,
                ws,
                id.unwrap_or_default(),
                enabled.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, super::MUTATING))?;
            Ok(json(200, json!({ "syncTagEnabledAt": opt_date(at) })))
        }
    }
}
