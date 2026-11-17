//! `analyticsRoutes.*` (`packages/analytics-nextjs/src/routes`): the
//! dashboard chart procedures. Each one calls the same analytics service as
//! the ported public `analytics.*` route with the same `{ data }` answer, so
//! it runs that route ([`super::public`]). Access is the `/rpc` workspace
//! membership check (`workspaceAuthorizedMidddleware`), as in the builder.

use serde_json::Value;

use super::{Ctx, Outcome, public};

/// The private procedure name → the public route id.
fn route(path: &str) -> Option<String> {
    let name = path.strip_prefix("analyticsRoutes.")?;
    if name == "macActiveContactCountByWorkspaceAPI" {
        return Some("analytics.macActiveContactCount".into());
    }
    let name = name.strip_suffix("AnalyticsAPI")?;
    if name.starts_with("get") {
        return None; // broadcast / sequence stats: other inputs, not ported here.
    }
    let name = name.replace("AIProviders", "AiProviders");
    let id = format!("analytics.{name}");
    crate::handlers::analytics::ROUTES
        .contains(&id.as_str())
        .then_some(id)
}

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    let route_id = route(path)?;
    let query = query(input);
    // A schema failure is the private route's oRPC 400, not the public 422.
    Some(
        public(ctx, &route_id, &[], Some(&query), None)
            .await
            .map_err(|error| match error.status {
                422 => super::invalid(),
                _ => error,
            }),
    )
}

/// The input as a GET query (arrays as repeated keys, as the oRPC OpenAPI
/// query of the original); `workspaceId` is the `/rpc` workspace.
pub(super) fn query(input: &Value) -> String {
    let mut pairs = url::form_urlencoded::Serializer::new(String::new());
    let Some(object) = input.as_object() else {
        return String::new();
    };
    for (key, value) in object {
        if key == "workspaceId" {
            continue;
        }
        let values = match value {
            Value::Array(items) => items.iter().collect(),
            other => vec![other],
        };
        for value in values {
            match value {
                Value::String(text) => pairs.append_pair(key, text),
                Value::Number(number) => pairs.append_pair(key, &number.to_string()),
                Value::Bool(flag) => pairs.append_pair(key, &flag.to_string()),
                _ => continue,
            };
        }
    }
    pairs.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_private_names_to_public_routes() {
        let cases = [
            (
                "analyticsRoutes.contactCountsPerDayAnalyticsAPI",
                Some("analytics.contactCountsPerDay"),
            ),
            (
                "analyticsRoutes.botMessagesAIProvidersAnalyticsAPI",
                Some("analytics.botMessagesAiProviders"),
            ),
            (
                "analyticsRoutes.macActiveContactCountByWorkspaceAPI",
                Some("analytics.macActiveContactCount"),
            ),
            ("analyticsRoutes.getBroadcastStatsAnalyticsAPI", None),
            ("analyticsRoutes.refLinkStats", None),
        ];
        for (path, want) in cases {
            assert_eq!(route(path).as_deref(), want, "{path}");
        }
    }

    #[test]
    fn query_repeats_array_keys() {
        let input = json!({ "workspaceId": "1", "from": "2026-01-01T00:00:00.000Z", "inboxIds": ["2", "3"] });
        assert_eq!(
            query(&input),
            "from=2026-01-01T00%3A00%3A00.000Z&inboxIds=2&inboxIds=3"
        );
    }
}
