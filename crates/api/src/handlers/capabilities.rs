//! `features/capabilities/api/public.ts` (Q.D11 slice 5, scope `contacts`):
//! `GET /v1/capabilities` and `GET /v1/schemas/flow-spec`; also
//! `features/token/api/public.ts` (`GET /v1/token`, same scope).

use std::sync::LazyLock;

use chatbotx_business::capabilities::{CAPABILITIES_INCLUDES, get_capabilities};
use chatbotx_core::flow_config::flow_spec_schema;
use chatbotx_core::flow_config::z::to_json_schema;
use chatbotx_core::utils::channel::ChannelType;
use serde_json::Value;

use super::input::{Input, Seg, key};
use super::{ApiState, Call, Principal, Reply, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &["capabilities.get", "schemas.flowSpec", "token.get"];

/// `includeQueryParam`: a comma list (trimmed, blanks dropped; none =
/// absent) of `CAPABILITIES_INCLUDES` values.
fn include_param(input: &mut Input) -> Option<Vec<String>> {
    let parts: Vec<Value> = match input.get("include").cloned()? {
        Value::String(text) => text
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| Value::String(p.to_string()))
            .collect(),
        Value::Array(items) => items,
        other => vec![other],
    };
    if parts.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        let mut path = key("include");
        path.push(Seg::Index(index));
        if let Some(value) = input.enum_at(&path, Some(part), CAPABILITIES_INCLUDES, true) {
            out.push(value);
        }
    }
    Some(out)
}

/// The flow-spec JSON Schema (`ZodToJsonSchemaConverter`, built once).
fn flow_spec_json_schema() -> &'static Value {
    static SCHEMA: LazyLock<Value> = LazyLock::new(|| to_json_schema(&flow_spec_schema()));
    &SCHEMA
}

/// Runs one of [`ROUTES`].
pub async fn handle(
    state: &ApiState,
    route_id: &str,
    call: Call<'_>,
    principal: &Principal,
) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    match route_id {
        "capabilities.get" => {
            let channels: Vec<&str> = ChannelType::ALL.iter().map(|c| c.as_str()).collect();
            let channel = {
                let value = input.get("channel").cloned();
                input.enum_at(&key("channel"), value.as_ref(), &channels, false)
            };
            let include = include_param(&mut input);
            input.finish()?;
            Ok(json(
                200,
                get_capabilities(
                    &state.db,
                    call.workspace_id,
                    include.as_deref(),
                    channel.as_deref(),
                )
                .await,
            ))
        }
        "schemas.flowSpec" => {
            input.finish()?;
            Ok(json(200, flow_spec_json_schema().clone()))
        }
        // `tokenPublicRouter.get`: the calling token's workspace, permission
        // and scopes (`null` = unrestricted).
        "token.get" => Ok(json(
            200,
            serde_json::json!({
                "workspaceId": call.workspace_id.to_string(),
                "permission": if principal.read_only { "read_only" } else { "full" },
                "scopes": principal.scopes,
            }),
        )),
        _ => Err(crate::routes::not_ported_error()),
    }
}
