//! `features/realtime/api/public.ts` (`realtimePublicRouter`, Q.A1):
//! `GET /v1/realtime/connect-token` mints the 60-second member connect token
//! of the signed-in mobile member.

use serde_json::json;

use super::{ApiState, Call, FINDING, Principal, Reply, business_error, internal_error, json};
use crate::orpc::OrpcError;

/// The routes of this module.
pub const ROUTES: &[&str] = &["realtime.getConnectToken"];

/// `REALTIME_SOCKET_PATH`.
const REALTIME_SOCKET_PATH: &str = "rt/workspaces";

/// `toSocketUrl`: `https://host/ws/` -> `wss://host/ws/rt/workspaces/<id>`.
fn socket_url(public_realtime_url: &str, workspace_id: i64) -> String {
    let joined = format!("{public_realtime_url}{REALTIME_SOCKET_PATH}/{workspace_id}");
    match joined.split_once(':') {
        Some(("http", rest)) => format!("ws:{rest}"),
        Some((_, rest)) => format!("wss:{rest}"),
        None => joined,
    }
}

pub async fn handle(
    state: &ApiState,
    _route_id: &str,
    call: Call<'_>,
    principal: &Principal,
) -> Reply {
    // A workspace API token or MCP token has no member identity.
    let Some(member) = principal.mobile.as_ref() else {
        return Err(OrpcError::new(
            "FORBIDDEN",
            None,
            Some("A signed-in member session is required"),
        ));
    };
    let Some(realtime) = state.realtime.as_ref() else {
        return Err(internal_error("REALTIME_BROADCAST_SECRET is not set"));
    };
    let issued_at_ms = principal
        .issued_at_ms
        .unwrap_or_else(chatbotx_db::repo::now_ms);
    let token = crate::rpc::procedures::mint_member_connect_token(
        &state.db,
        &realtime.config().secret,
        call.workspace_id,
        member.user_id,
        &member.permissions,
        issued_at_ms,
    )
    .await
    .map_err(|error| business_error(error, FINDING))?;
    // `resolveTenantSettings(...).publicRealtimeUrl` (`deriveUrls`: `<appUrl>/ws/`).
    let url = format!("{}/ws/", state.app_url);
    Ok(json(
        200,
        json!({
            "token": token,
            "socketUrl": socket_url(&url, call.workspace_id),
            "url": url,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::socket_url;

    #[test]
    fn the_socket_url_swaps_the_scheme_for_ws_or_wss() {
        assert_eq!(
            socket_url("http://app/ws/", 7),
            "ws://app/ws/rt/workspaces/7"
        );
        assert_eq!(
            socket_url("https://x.io/ws/", 7),
            "wss://x.io/ws/rt/workspaces/7"
        );
    }
}
