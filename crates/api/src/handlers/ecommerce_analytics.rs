//! `features/ecommerce-analytics/api/public.ts`: the sales dashboard.

use chatbotx_business::ecommerce_analytics;

use super::input::{Input, Str};
use super::{ApiState, Call, Reply, business_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &["ecommerceAnalytics.get"];

/// `DAY_PATTERN` (`^\d{4}-\d{2}-\d{2}$`).
pub(crate) fn is_day(text: &str) -> bool {
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

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, _route_id: &str, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let day = Str {
        pattern: Some(is_day),
        ..Str::default()
    };
    let from = input.opt_string("from", day);
    let to = input.opt_string("to", day);
    input.finish()?;
    let data = ecommerce_analytics::get_dashboard(&state.db, call.workspace_id, from, to)
        .await
        .map_err(|e| business_error(e, super::FINDING))?;
    Ok(json(200, data))
}
