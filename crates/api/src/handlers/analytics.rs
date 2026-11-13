//! `apps/builder/src/features/analytics/api/public.ts`: the ported
//! `analytics.*` routes (`blockedContactsCount`; the rest later).

use chatbotx_db::repo::parse_iso;
use chatbotx_db::rusqlite::params_from_iter;
use chatbotx_db::rusqlite::types::Value as Sql;
use serde_json::json;

use super::input::{Input, Str};
use super::{ApiState, Call, Reply, internal_error, json};

/// The ported routes.
pub const ROUTES: &[&str] = &["analytics.blockedContactsCount"];

/// `FROM_FLOOR` (2020-01-01T00:00:00.000Z).
const FROM_FLOOR_MS: i64 = 1_577_836_800_000;

/// A comma-separated (or single) query list.
fn list(input: &Input, name: &str) -> Vec<String> {
    input
        .get(name)
        .and_then(|value| value.as_str())
        .map(|text| {
            text.split(',')
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    match route_id {
        "analytics.blockedContactsCount" => {
            let mut input = Input::from_query(call.params, call.query);
            let from = input.string("from", Str::default());
            let to = input.string("to", Str::default());
            let _timezone = input.opt_string("timezone", Str::default());
            let channels = list(&input, "channels");
            let inbox_ids = list(&input, "inboxIds");
            input.finish()?;
            // `new Date(val)`: an unparsable value is an Invalid Date, which
            // matches no row.
            let from = from
                .as_deref()
                .and_then(parse_iso)
                .map(|ms| ms.max(FROM_FLOOR_MS));
            let to = to.as_deref().and_then(parse_iso);
            let (Some(from), Some(to)) = (from, to) else {
                return Ok(json(200, json!({ "data": { "count": 0 } })));
            };
            let ws = call.workspace_id;
            let count = state
                .db
                .read(move |conn| {
                    let mut sql = String::from(
                        r#"SELECT COUNT(DISTINCT "contactId") FROM "AnalyticsContactEvent"
                           WHERE "workspaceId" = ? AND "occurredAt" >= ? AND "occurredAt" <= ?
                             AND "eventType" = 'contact_blocked'"#,
                    );
                    let mut binds = vec![Sql::Integer(ws), Sql::Integer(from), Sql::Integer(to)];
                    if !channels.is_empty() {
                        sql.push_str(&format!(
                            r#" AND "channel" IN ({})"#,
                            vec!["?"; channels.len()].join(", ")
                        ));
                        binds.extend(channels.into_iter().map(Sql::Text));
                    }
                    if !inbox_ids.is_empty() {
                        sql.push_str(&format!(
                            r#" AND "inboxId" IN ({})"#,
                            vec!["?"; inbox_ids.len()].join(", ")
                        ));
                        binds.extend(
                            inbox_ids
                                .iter()
                                .map(|id| Sql::Integer(id.parse().unwrap_or(-1))),
                        );
                    }
                    Ok(conn
                        .query_row(&sql, params_from_iter(binds.iter()), |r| r.get::<_, i64>(0))?)
                })
                .await
                .map_err(|e| internal_error(&e.to_string()))?;
            Ok(json(200, json!({ "data": { "count": count } })))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
