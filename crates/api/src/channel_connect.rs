//! The builder-UI channel connect starts (Q.D12-channels):
//! `app/(no-sidebar)/channels/create/messenger/route.ts`,
//! `channels/instagram/route.ts`, `channels/instagram-facebook/route.ts`
//! (`startChannelConnect`). Each opens a `connect` session for the signed-in
//! user and redirects to the provider dialog; the callback
//! (`connect_callback.rs`) then sends the user to the session's
//! `/channels/<channel>/select` page.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chatbotx_business::connect_session;
use chatbotx_business::connection::credentials::HttpUpstream;
use chatbotx_business::connection::oauth::{self, Actor, StartSession};
use chatbotx_business::{BusinessError, tenant, tx};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::Value;

use crate::handlers::ApiState;
use crate::rpc::find_rpc_session;
use crate::session::AuthState;

/// `DEFAULT_WORKSPACE_NAME` of `create-first-workspace.ts`.
const DEFAULT_WORKSPACE_NAME: &str = "New Workspace";

#[derive(Clone)]
pub struct ConnectStartState {
    pub auth: Arc<AuthState>,
    pub api: Arc<ApiState>,
}

pub fn router(state: ConnectStartState) -> Router {
    Router::new()
        .route(
            "/channels/create/messenger",
            get(|s: State<ConnectStartState>, h: HeaderMap, q: RawQuery| {
                start(s, h, q, "messenger")
            }),
        )
        .route(
            "/channels/instagram",
            get(|s: State<ConnectStartState>, h: HeaderMap, q: RawQuery| {
                start(s, h, q, "instagram")
            }),
        )
        .route(
            "/channels/instagram-facebook",
            get(|s: State<ConnectStartState>, h: HeaderMap, q: RawQuery| {
                start(s, h, q, "instagramFacebook")
            }),
        )
        .with_state(state)
}

/// The `/channels/<channel>/select` page of a provider.
pub fn select_path(provider: &str, session_id: i64) -> String {
    let channel = match provider {
        "instagramFacebook" => "instagram-facebook",
        other => other,
    };
    format!("/channels/{channel}/select?session={session_id}")
}

/// `createChannelPath({ error, workspaceId })`.
pub fn create_channel_path(error: Option<&str>, workspace_id: Option<i64>) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    if let Some(ws) = workspace_id {
        query.append_pair("workspaceId", &ws.to_string());
    }
    if let Some(error) = error {
        query.append_pair("error", error);
    }
    let search = query.finish();
    if search.is_empty() {
        "/channels/create".into()
    } else {
        format!("/channels/create?{search}")
    }
}

/// `CREATE_CHANNEL_ERROR_MESSAGE_KEYS` keys.
pub const CREATE_CHANNEL_ERROR_CODES: &[&str] = &[
    "workspaceLimitReached",
    "trialExpired",
    "macLimitReached",
    "sessionExpired",
    "noPages",
    "noInstagramAccounts",
    "exchangeFailed",
    "providerError",
    "notMember",
    "connectFailed",
    "tooManyAttempts",
];

/// `startFailureErrorCode`.
fn start_failure_code(error: &BusinessError) -> &'static str {
    match error {
        BusinessError::Exception { code, .. } if code == "connectSessionLimitReached" => {
            "tooManyAttempts"
        }
        BusinessError::Exception { code, .. } => CREATE_CHANNEL_ERROR_CODES
            .iter()
            .find(|known| **known == code.as_str() && **known != "sessionExpired")
            .copied()
            .unwrap_or("connectFailed"),
        _ => "connectFailed",
    }
}

fn not_found() -> Response {
    StatusCode::NOT_FOUND.into_response()
}

/// Next's `redirect()` (307).
fn redirect(target: &str) -> Response {
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(header::LOCATION, target.to_string())],
    )
        .into_response()
}

/// `ensureConnectTargetForOwner`: the user's oldest owned workspace that is
/// not scheduled for deletion and has no inbox yet, else a new one.
async fn connect_target_workspace(api: &ApiState, user_id: i64) -> Result<i64, BusinessError> {
    tx::write(&api.db, move |conn| {
        let existing: Option<i64> = conn
            .query_row(
                r#"SELECT "id" FROM "Workspace" w WHERE "ownerId" = ?1 AND "scheduledDeletionAt" IS NULL
                   AND NOT EXISTS (SELECT 1 FROM "Inbox" i WHERE i."workspaceId" = w."id")
                   ORDER BY "id" LIMIT 1"#,
                params![user_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(chatbotx_db::repo::Error::from)?;
        match existing {
            Some(id) => Ok(id),
            None => Ok(crate::spa::insert_workspace(conn, user_id, DEFAULT_WORKSPACE_NAME, None)
                .map_err(|e| chatbotx_db::repo::Error::msg(e.to_string()))?),
        }
    })
    .await
}

/// `requireWorkspacePermission(workspaceId, "superAdmin")`.
async fn is_super_admin(api: &ApiState, workspace_id: i64, user_id: i64) -> bool {
    let permissions: Option<String> = tx::read(&api.db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT "permissions" FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
                params![workspace_id, user_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(chatbotx_db::repo::Error::from)?)
    })
    .await
    .ok()
    .flatten();
    permissions
        .and_then(|p| serde_json::from_str::<Value>(&p).ok())
        .is_some_and(|p| p["superAdmin"] == true)
}

async fn start(
    State(state): State<ConnectStartState>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    provider: &'static str,
) -> Response {
    let query: HashMap<String, String> =
        url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let api = &state.api;
    let Ok(Some(session)) = find_rpc_session(&state.auth, &headers).await else {
        return not_found();
    };
    let user_id = session.user_id;
    let workspace_id = match query.get("workspaceId").filter(|w| !w.is_empty()) {
        Some(text) => match text.parse::<i64>() {
            Ok(id) if is_super_admin(api, id, user_id).await => Some(id),
            _ => return not_found(),
        },
        None => None,
    };
    let Some(encryption) = api.encryption.as_deref() else {
        return not_found();
    };
    let domain = headers
        .get("x-domain")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let owner = match tenant::resolve_platform_owner_id(
        &api.db,
        api.edition,
        domain,
        user_id,
        workspace_id,
    )
    .await
    {
        Ok(owner) => owner,
        Err(error) => {
            tracing::error!(error = %error, provider, "Failed to start a channel connect session");
            return redirect(&create_channel_path(Some("connectFailed"), workspace_id));
        }
    };
    let credential =
        match oauth::resolve_credential(&api.db, encryption, &api.app_url, provider, Some(owner))
            .await
        {
            Ok(Some(credential)) => credential,
            _ => return not_found(),
        };
    let target = match workspace_id {
        Some(id) => id,
        None => match connect_target_workspace(api, user_id).await {
            Ok(id) => id,
            Err(error) => {
                tracing::error!(error = %error, provider, "Failed to start a channel connect session");
                return redirect(&create_channel_path(Some(start_failure_code(&error)), None));
            }
        },
    };
    if provider == "messenger"
        && let Some(session) = oauth::start_messenger_with_sso(
            &api.db,
            encryption,
            &HttpUpstream,
            &credential,
            target,
            user_id,
            owner,
        )
        .await
    {
        return redirect(&select_path(provider, session.id));
    }
    let origin_host = url::Url::parse(&api.app_url).ok().and_then(|u| {
        u.host_str().map(|h| match u.port() {
            Some(port) => format!("{h}:{port}"),
            None => h.to_string(),
        })
    });
    let session = match oauth::start_session(
        &api.db,
        &credential,
        StartSession {
            workspace_id: target,
            provider: provider.to_string(),
            purpose: "connect",
            target_connection_id: None,
            platform_owner_id: Some(owner),
            origin_host,
            return_url: None,
            actor: Some(Actor::User(user_id)),
        },
    )
    .await
    {
        Ok(session) => session,
        Err(error) => {
            tracing::error!(error = %error, provider, workspace_id, "Failed to start a channel connect session");
            return redirect(&create_channel_path(
                Some(start_failure_code(&error)),
                workspace_id,
            ));
        }
    };
    if let Err(error) =
        connect_session::update_return_url(&api.db, session.id, select_path(provider, session.id))
            .await
    {
        tracing::error!(error = %error, session_id = session.id, provider, "Failed to set the connect session return URL");
        connect_session::fail(&api.db, &session, "internal_error", None).await;
        return redirect(&create_channel_path(Some("connectFailed"), Some(target)));
    }
    match session
        .next_action
        .as_ref()
        .filter(|a| a["type"] == "open_url")
        .and_then(|a| a["url"].as_str())
    {
        Some(url) => redirect(url),
        None => {
            tracing::error!(
                session_id = session.id,
                provider,
                "Unexpected connect next action"
            );
            connect_session::fail(&api.db, &session, "internal_error", None).await;
            redirect(&create_channel_path(Some("connectFailed"), Some(target)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_the_builder() {
        assert_eq!(create_channel_path(None, None), "/channels/create");
        assert_eq!(
            create_channel_path(Some("noPages"), Some(5)),
            "/channels/create?workspaceId=5&error=noPages"
        );
        assert_eq!(
            select_path("instagramFacebook", 9),
            "/channels/instagram-facebook/select?session=9"
        );
    }

    #[test]
    fn start_failures_keep_only_known_codes() {
        let code = |c: &str| start_failure_code(&BusinessError::exception("x", c, 400));
        assert_eq!(code("connectSessionLimitReached"), "tooManyAttempts");
        assert_eq!(code("workspaceLimitReached"), "workspaceLimitReached");
        assert_eq!(code("sessionExpired"), "connectFailed");
        assert_eq!(code("other"), "connectFailed");
    }
}
