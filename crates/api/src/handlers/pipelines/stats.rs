//! `pipelines.getStats` (`pipelineStatsPublicRequest`).

use chatbotx_business::pipeline::analytics::{self, StatsTotals};
use chatbotx_core::utils::tz::is_intl_time_zone;
use serde_json::{Value, json};

use super::super::input::{Input, Str, key};
use super::super::{ApiState, Call, FINDING, Reply, business_error, number};
use super::{TOKEN, money, ok};
use crate::orpc::OrpcError;

/// `DATE_KEY_PATTERN` (`/^\d{4}-\d{2}-\d{2}$/`).
fn is_date_key(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

fn date_key(input: &mut Input, name: &str) -> Option<String> {
    let text = input.string(name, Str::default())?;
    if !is_date_key(&text) {
        input.issue(&key(name), "invalid_format", "Use YYYY-MM-DD");
        return None;
    }
    Some(text)
}

/// `pipelineStatsPublicRequest.parse`: (`from`, `to`, `timezone`).
pub fn stats_query(
    params: &[(&'static str, String)],
    query: Option<&str>,
) -> Result<(String, String, Option<String>), OrpcError> {
    let mut input = Input::from_query(params, query);
    let _pipeline_id = input.id("id");
    let from = date_key(&mut input, "from");
    let to = date_key(&mut input, "to");
    let timezone = input.opt_string("timezone", Str::default());
    if let Some(zone) = &timezone
        && !is_intl_time_zone(zone)
    {
        input.issue(&key("timezone"), "custom", "Invalid IANA timezone");
    }
    input.finish()?;
    Ok((from.unwrap_or_default(), to.unwrap_or_default(), timezone))
}

fn totals_json(totals: &StatsTotals) -> serde_json::Map<String, Value> {
    let Value::Object(map) = json!({
        "created": totals.created,
        "won": totals.won,
        "wonValue": money(Some(totals.won_value)),
        "lost": totals.lost,
        "lostValue": money(Some(totals.lost_value)),
    }) else {
        return serde_json::Map::new();
    };
    map
}

fn change(value: Option<f64>) -> Value {
    value.map_or(Value::Null, number)
}

/// Runs `pipelines.getStats`.
pub async fn handle(state: &ApiState, call: Call<'_>) -> Reply {
    let pipeline_id = call
        .params
        .iter()
        .find(|(name, _)| *name == "id")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or_default();
    let (from, to, timezone) = stats_query(call.params, call.query)?;
    let stats = analytics::get_stats(
        &state.db,
        call.workspace_id,
        pipeline_id,
        from,
        to,
        timezone,
        TOKEN,
    )
    .await
    .map_err(|e| business_error(e, FINDING))?;
    let daily: Vec<Value> = stats
        .daily
        .iter()
        .map(|(day, totals)| {
            let mut map = serde_json::Map::new();
            map.insert("day".into(), json!(day));
            map.extend(totals_json(totals));
            Value::Object(map)
        })
        .collect();
    let by_actor: Vec<Value> = stats
        .by_actor
        .iter()
        .map(|actor| {
            let mut map = serde_json::Map::new();
            map.insert("actorKind".into(), json!(actor.actor_kind));
            map.insert(
                "actorUserId".into(),
                json!(actor.actor_user_id.map(|id| id.to_string())),
            );
            map.extend(totals_json(&actor.totals));
            Value::Object(map)
        })
        .collect();
    ok(
        200,
        json!({
            "range": { "from": stats.from, "to": stats.to, "timezone": stats.timezone },
            "current": totals_json(&stats.current),
            "previous": totals_json(&stats.previous),
            "change": {
                "created": change(stats.change.created),
                "won": change(stats.change.won),
                "wonValue": change(stats.change.won_value),
                "lost": change(stats.change.lost),
                "lostValue": change(stats.change.lost_value),
            },
            "daily": daily,
            "byActor": by_actor,
        }),
    )
}
