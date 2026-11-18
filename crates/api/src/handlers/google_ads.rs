//! `features/integration-google-ads/api/public.ts`: `googleAds.*` (scope `ads`).
//! Read-only on purpose, as the original.

use chatbotx_business::google_ads::{self as service, EVENT_STATUSES, StatsInput};
use chatbotx_core::utils::google_click::GOOGLE_ADS_CHANNEL_VALUES;
use chatbotx_db::repo::google_ads_conversion_event::ListInput;
use serde_json::json;

use super::broadcasts::coerce_date;
use super::input::{Input, Str, key};
use super::qr_codes::public_paging;
use super::{ApiState, Call, LISTING, Reply, business_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "googleAds.getStats",
    "googleAds.listEvents",
    "googleAds.getConnection",
];

fn date_key(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn pattern(check: fn(&str) -> bool) -> Str {
    Str {
        pattern: Some(check),
        ..Str::default()
    }
}

/// `channel` + `conversionActionId` filters (both routes).
fn filters(input: &mut Input) -> (Option<String>, Option<String>) {
    let value = input.get("channel").cloned();
    let channel = input.enum_at(
        &key("channel"),
        value.as_ref(),
        GOOGLE_ADS_CHANNEL_VALUES,
        false,
    );
    let action = input.opt_string("conversionActionId", pattern(digits));
    (channel, action)
}

fn opt_date(input: &mut Input, name: &str) -> Option<i64> {
    let value = input.get(name)?.clone();
    let parsed = coerce_date(&value);
    if parsed.is_none() {
        input.issue(
            &key(name),
            "invalid_type",
            "Invalid input: expected date, received Date",
        );
    }
    parsed
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let mut input = Input::from_query(call.params, call.query);
    let body = match route_id {
        "googleAds.getStats" => {
            let from = input.string("from", pattern(date_key));
            let to = input.string("to", pattern(date_key));
            let tz = input.opt_string("tz", Str::default());
            let (channel, conversion_action_id) = filters(&mut input);
            input.finish()?;
            service::get_stats(
                db,
                StatsInput {
                    workspace_id: ws,
                    from: from.unwrap_or_default(),
                    to: to.unwrap_or_default(),
                    tz: tz.unwrap_or_default(),
                    channel,
                    conversion_action_id,
                },
            )
            .await
        }
        "googleAds.listEvents" => {
            let (page, per_page) = public_paging(&mut input);
            let value = input.get("status").cloned();
            let status = input.enum_at(&key("status"), value.as_ref(), EVENT_STATUSES, false);
            let (channel, conversion_action_id) = filters(&mut input);
            let since = opt_date(&mut input, "since");
            let until = opt_date(&mut input, "until");
            input.finish()?;
            service::list_events(
                db,
                ListInput {
                    workspace_id: ws,
                    status,
                    channel,
                    conversion_action_id,
                    since,
                    until,
                    page,
                    per_page,
                },
            )
            .await
            .map(|(data, page_count)| json!({ "data": data, "pageCount": page_count }))
        }
        "googleAds.getConnection" => {
            input.finish()?;
            service::get_connection(db, ws).await
        }
        _ => return Err(crate::routes::not_ported_error()),
    };
    body.map(|b| json(200, b))
        .map_err(|e| business_error(e, LISTING))
}
