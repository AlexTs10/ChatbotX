//! `features/connections/api/public.ts` (`connectSessionsPublicRouter`):
//! `connectSessions.get / cancel / connectTargets`.

use chatbotx_business::BusinessError;
use chatbotx_business::connect_session::{self, SessionRow};
use chatbotx_business::connection::credentials::HttpUpstream;
use chatbotx_business::connection::oauth;
use chatbotx_business::connection::providers;
use serde_json::{Value, json};

use super::connections::{connection_json, session_json};
use super::input::{Input, Seg, Str, key};
use super::{ApiState, Call, ErrorMap, Principal, Reply, business_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "connectSessions.get",
    "connectSessions.cancel",
    "connectSessions.connectTargets",
];

const IDEMPOTENCY: [(&str, u16); 3] = [
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];
/// `possibleErrorsOnFindingConnectSession`.
const FINDING: ErrorMap = &[("notFound", 404), ("businessError", 400)];
/// `possibleErrorsOnConnectingSessionTargets`.
const TARGETS: ErrorMap = &[
    ("connectionProviderDedicatedOnly", 403),
    ("notFound", 404),
    ("businessError", 400),
    ("connectSessionExpired", 400),
    IDEMPOTENCY[0],
    IDEMPOTENCY[1],
    IDEMPOTENCY[2],
];
/// `possibleErrorsOnCancelingConnectSession`.
const CANCELING: ErrorMap = &[
    ("connectionProviderDedicatedOnly", 403),
    ("notFound", 404),
    ("businessError", 400),
    IDEMPOTENCY[0],
    IDEMPOTENCY[1],
    IDEMPOTENCY[2],
];

/// `loadScopedSession`.
async fn load_scoped(
    state: &ApiState,
    principal: &Principal,
    id: i64,
    workspace_id: i64,
) -> Result<SessionRow, BusinessError> {
    let row = connect_session::find_for_workspace(&state.db, id, workspace_id)
        .await?
        .ok_or_else(|| BusinessError::not_found("Connect session not found"))?;
    providers::assert_token_scope_for_provider(principal.scopes.as_deref(), &row.provider)?;
    Ok(row)
}

/// `targetIds: z.array(z.string()).min(1)`.
fn target_ids(input: &mut Input) -> Option<Vec<String>> {
    let path = key("targetIds");
    let items = match input.get("targetIds").cloned() {
        Some(Value::Array(items)) => items,
        other => {
            let received = match other {
                None => "undefined",
                Some(Value::Null) => "null",
                Some(Value::Bool(_)) => "boolean",
                Some(Value::Number(_)) => "number",
                Some(Value::String(_)) => "string",
                Some(_) => "object",
            };
            input.issue(
                &path,
                "invalid_type",
                &format!("Invalid input: expected array, received {received}"),
            );
            return None;
        }
    };
    if items.is_empty() {
        input.issue(
            &path,
            "too_small",
            "Too small: expected array to have >=1 items",
        );
        return None;
    }
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match item {
            Value::String(text) => out.push(text.clone()),
            _ => input.issue(
                &[Seg::Key("targetIds".into()), Seg::Index(index)],
                "invalid_type",
                "Invalid input: expected string",
            ),
        }
    }
    (out.len() == items.len()).then_some(out)
}

/// Runs one of [`ROUTES`].
pub async fn handle(
    state: &ApiState,
    route_id: &str,
    call: Call<'_>,
    principal: &Principal,
) -> Reply {
    let ws = call.workspace_id;
    let mut input = if route_id == "connectSessions.connectTargets" {
        Input::from_body(call.params, call.body)
    } else {
        Input::from_query(call.params, call.query)
    };
    // `connectSessionIdField` is any string: a non-id never matches.
    let id = input
        .string("id", Str::default())
        .unwrap_or_default()
        .parse::<i64>()
        .unwrap_or(-1);
    let targets = (route_id == "connectSessions.connectTargets").then(|| target_ids(&mut input));
    input.finish()?;
    match route_id {
        "connectSessions.get" => load_scoped(state, principal, id, ws)
            .await
            .map(|row| json(200, session_json(&row)))
            .map_err(|e| business_error(e, FINDING)),
        "connectSessions.cancel" => async {
            let row = load_scoped(state, principal, id, ws).await?;
            providers::assert_generic_provider_allowed(&row.provider)?;
            connect_session::cancel(&state.db, row.id, ws).await
        }
        .await
        .map(|row| json(200, session_json(&row)))
        .map_err(|e| business_error(e, CANCELING)),
        _ => async {
            let row = load_scoped(state, principal, id, ws).await?;
            providers::assert_generic_provider_allowed(&row.provider)?;
            let Some(encryption) = state.encryption.as_deref() else {
                return Err(oauth::not_configured(&row.provider));
            };
            let ids = targets.flatten().unwrap_or_default();
            let result = oauth::connect_targets(&state.db, encryption, &HttpUpstream, row.id, ws, &ids, row.actor_user_id).await?;
            if let Some(jobs) = state.jobs.as_deref() {
                chatbotx_business::tag_sync::enqueue_new_zalo_scans(&state.db, jobs, &result.connections).await;
            }
            Ok(result)
        }
        .await
        .map(|result| {
            json(
                200,
                json!({
                    "session": session_json(&result.session),
                    "connections": result.connections.iter().map(connection_json).collect::<Vec<_>>(),
                    "outcomes": result.outcomes,
                }),
            )
        })
        .map_err(|e| business_error(e, TARGETS)),
    }
}
