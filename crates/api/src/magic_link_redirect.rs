//! `apps/builder/src/app/r/[workspaceId]/[name]/route.ts`: the public magic
//! link. Redirects to the link's URL (query params interpolated) and, with a
//! button `code`, reports the click as `flow:clicked` on the flow bus.
//!
//! ponytail: no open-link interstitial (`wrapOpenLinkUrl` → `/go/…`): that
//! page is not served by the Rust server, so a deep-link destination is
//! redirected to directly.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chatbotx_business::{flow_version, magic_link, tx};
use chatbotx_core::flow_config::util::{decode_button_payload, resolve_flow_action_target};
use chatbotx_core::variables::text::interpolate;
use chatbotx_db::repo::contact_inbox;
use chatbotx_jobs::events::{EventBuses, emit};
use serde_json::{Value, json};

use crate::handlers::ApiState;

/// The router state: the API state and the durable event buses (`None` =
/// the events worker did not start; the click is then not reported).
#[derive(Clone)]
pub struct RedirectState {
    pub api: Arc<ApiState>,
    pub buses: Option<EventBuses>,
}

/// `GET /r/{workspaceId}/{name}`.
pub fn router(state: RedirectState) -> Router {
    Router::new()
        .route("/r/{workspace_id}/{name}", get(handle))
        .with_state(state)
}

fn message(status: StatusCode, text: &str) -> Response {
    (status, axum::Json(json!({ "message": text }))).into_response()
}

fn redirect(destination: &str) -> Response {
    if url::Url::parse(destination).is_err() {
        // `NextResponse.redirect` throws on a relative / invalid URL.
        return message(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
    }
    (
        StatusCode::FOUND,
        [(header::LOCATION, destination.to_string())],
    )
        .into_response()
}

/// `loadServableWorkspace(...).servable`: it exists and no deletion is scheduled.
async fn servable(api: &ApiState, workspace_id: i64) -> bool {
    tx::read(&api.db, move |c| {
        Ok(c.query_row(
            r#"SELECT count(*) FROM "Workspace" WHERE "id" = ?1 AND "scheduledDeletionAt" IS NULL"#,
            [workspace_id],
            |r| r.get::<_, i64>(0),
        )
        .map_err(chatbotx_db::repo::Error::from)?
            > 0)
    })
    .await
    .unwrap_or(false)
}

async fn handle(
    State(state): State<RedirectState>,
    Path((workspace, name)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let api = &state.api;
    let Some(workspace_id) = workspace
        .parse::<i64>()
        .ok()
        .filter(|_| workspace.bytes().all(|b| b.is_ascii_digit()))
    else {
        return message(StatusCode::NOT_FOUND, "Not found");
    };
    if !servable(api, workspace_id).await {
        return (
            StatusCode::GONE,
            axum::Json(json!({ "code": "workspaceScheduledDeletion" })),
        )
            .into_response();
    }
    let row = match magic_link::find_by_name(&api.db, workspace_id, name).await {
        Ok(Some(row)) => row,
        Ok(None) => return message(StatusCode::NOT_FOUND, "Not found"),
        Err(error) => {
            tracing::error!(err = ?error, "magic link lookup failed");
            return message(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
        }
    };
    let destination = interpolate(&row.url, &query, false);

    let Some(code) = query.get("code") else {
        return redirect(&destination);
    };
    let Some(button) = decode_button_payload(code, false) else {
        return message(StatusCode::BAD_REQUEST, "Invalid code");
    };
    let Some(contact_inbox_id) = button.contact_inbox_id.clone() else {
        return message(StatusCode::BAD_REQUEST, "Contact inbox ID is missing");
    };
    let inbox_id = contact_inbox_id.parse::<i64>().unwrap_or(-1);
    let found = tx::read(&api.db, move |c| {
        Ok(contact_inbox::find_by_id_with_conversation(c, inbox_id)?)
    })
    .await
    .ok()
    .flatten();
    // The conversation's workspace proves the inbox belongs to this link's
    // workspace; otherwise redirect without recording.
    let Some((inbox, conversation)) = found.and_then(|f| {
        f.conversation
            .filter(|c| c.workspace_id == workspace_id)
            .map(|c| (f.contact_inbox, c))
    }) else {
        tracing::warn!(
            workspace_id,
            magic_link_id = row.id,
            contact_inbox_id,
            "Magic link click could not be attributed: contact inbox missing or outside this workspace"
        );
        return redirect(&destination);
    };

    let flow_id = button.flow_id.parse::<i64>().unwrap_or(-1);
    let version_id = button
        .flow_version_id
        .as_deref()
        .and_then(|v| v.parse::<i64>().ok());
    let version = flow_version::find_for_button_payload(&api.db, workspace_id, flow_id, version_id)
        .await
        .ok()
        .flatten();
    let Some(nodes) = version.as_ref().and_then(|v| v.nodes.0.as_array().cloned()) else {
        tracing::warn!(
            workspace_id,
            magic_link_id = row.id,
            flow_id = %button.flow_id,
            "Magic link click could not be attributed: no flow version resolved"
        );
        return redirect(&destination);
    };
    let target = resolve_flow_action_target(&nodes, button.button_id.as_deref().unwrap_or(""));
    let Some(node_id) = target.and_then(|t| t.node_id) else {
        tracing::warn!(
            workspace_id,
            magic_link_id = row.id,
            flow_id = %button.flow_id,
            "Magic link click could not be attributed: button not in the live version"
        );
        return redirect(&destination);
    };

    if let Some(buses) = &state.buses {
        let payload = json!({
            "nodeId": node_id,
            "context": {
                "workspaceId": workspace_id.to_string(),
                "contactId": inbox.contact_id.to_string(),
                "conversationId": conversation.id.to_string(),
                "channel": inbox.channel,
                "contactInboxId": contact_inbox_id,
            },
            "action": {
                "flowId": button.flow_id,
                "buttonId": button.button_id,
                "broadcastId": button.broadcast_id,
                "sequenceStepId": button.sequence_step_id,
                "commentAutomationId": button.comment_automation_id,
                "magicLinkId": row.id.to_string(),
                "clickType": "magic_link",
            },
            "occurredAt": chatbotx_db::repo::iso(chatbotx_db::repo::now_ms()),
        });
        if let Some(sent) = emit(buses, "flow:clicked", strip_nulls(payload))
            && let Err(err) = sent.await
        {
            tracing::error!(err = %err, "flow:clicked emit failed");
        }
    }
    redirect(&destination)
}

/// `undefined` fields are left out of the emitted JSON.
fn strip_nulls(mut value: Value) -> Value {
    if let Some(action) = value.get_mut("action").and_then(Value::as_object_mut) {
        action.retain(|_, v| !v.is_null());
    }
    value
}
