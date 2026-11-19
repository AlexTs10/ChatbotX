//! The AI Router connections settings page (`features/decision-connections`):
//! the page list and the create / update / toggle / test actions, all behind
//! `requireWorkspacePermission(workspaceId, "superAdmin")` (a miss is a 404).

use chatbotx_business::BusinessError;
use chatbotx_business::decision_connection::{self as decision, HttpDecisionTransport};
use chatbotx_business::net::ssrf_guard::HttpDoh;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, invalid};
use crate::orpc::OrpcError;

pub(super) const ACTIONS: &[&str] = &[
    "createDecisionConnectionAction",
    "updateDecisionConnectionAction",
    "toggleDecisionConnectionAction",
    "testDecisionConnectionAction",
    "decisionConnectionsPage",
];

/// `z.string().regex(/^\d+$/)`.
fn digits_id(input: &Value) -> Result<i64, OrpcError> {
    input["id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|id| id.parse().ok())
        .ok_or_else(invalid)
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, arg(args, 1)).await)
}

async fn run(ctx: &Ctx<'_>, name: &str, input: &Value) -> Result<Value, OrpcError> {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    // The input schema runs before the permission check (as the original).
    let parsed = match name {
        "createDecisionConnectionAction" => Some((
            None,
            decision::parse_save_input(input, &[]).map_err(|_| invalid())?,
        )),
        "updateDecisionConnectionAction" => Some((
            Some(digits_id(input)?),
            decision::parse_save_input(input, &["id"]).map_err(|_| invalid())?,
        )),
        _ => None,
    };
    let enabled = match name {
        "toggleDecisionConnectionAction" => {
            if input
                .as_object()
                .is_some_and(|o| o.keys().any(|k| k != "id" && k != "enabled"))
            {
                return Err(invalid());
            }
            Some((
                digits_id(input)?,
                In(input).opt_bool("enabled")?.ok_or_else(invalid)?,
            ))
        }
        _ => None,
    };
    if name == "testDecisionConnectionAction"
        && input
            .as_object()
            .is_some_and(|o| o.keys().any(|k| k != "id"))
    {
        return Err(invalid());
    }
    if !ctx.has("superAdmin") {
        return Err(action_fail(BusinessError::not_found("Not found")));
    }
    let result = match (name, parsed, enabled) {
        ("createDecisionConnectionAction", Some((_, save)), _) => {
            if save.credential.as_deref().is_none_or(str::is_empty) {
                Err(BusinessError::exception(
                    "Decision credential is required",
                    "INTERNAL_SERVER_ERROR",
                    500,
                ))
            } else {
                decision::create(db, &HttpDoh, ws, save).await
            }
        }
        ("updateDecisionConnectionAction", Some((Some(id), save)), _) => {
            decision::update(db, &HttpDoh, ws, id, save).await
        }
        ("toggleDecisionConnectionAction", _, Some((id, enabled))) => {
            decision::set_enabled(db, ws, id, enabled).await
        }
        ("testDecisionConnectionAction", _, _) => {
            let id = digits_id(input)?;
            match ctx.api.kv.as_deref() {
                Some(kv) => {
                    decision::test(db, &kv.durable, &HttpDecisionTransport(HttpDoh), ws, id).await
                }
                None => Err(BusinessError::exception(
                    "Decision connection test is unavailable",
                    "INTERNAL_SERVER_ERROR",
                    500,
                )),
            }
        }
        _ => decision::list_safe(db, ws)
            .await
            .map(|connections| json!({ "connections": connections })),
    };
    result.map_err(action_fail)
}
