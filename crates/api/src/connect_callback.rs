//! `GET /integrations/{provider}/callback` for a connect session state
//! (`handleConnectSessionCallback` / `runConnectSessionCallback` of
//! `apps/builder/src/app/integrations/[...integration]/callback.ts`). The
//! legacy JSON-state flows of that route are not ported: such a state
//! answers 404. No relay to `originHost`: a session only stores the app's own
//! host (`toSessionReturn`), so the relay is always a no-op here.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chatbotx_business::BusinessError;
use chatbotx_business::connect_session::{self, SessionRow};
use chatbotx_business::connection::credentials::HttpUpstream;
use chatbotx_business::connection::oauth;
use chatbotx_business::google_oauth::{self, Kind};

use crate::handlers::ApiState;

/// The route.
pub fn router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/integrations/{provider}/callback", get(handle))
        .with_state(state)
}

/// `CONNECT_SESSION_STATE_PATTERN` (`^\d+\.[A-Za-z0-9_-]+$`).
fn split_state(state: &str) -> Option<(i64, &str)> {
    let (id, nonce) = state.split_once('.')?;
    let id_ok = !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit());
    let nonce_ok = !nonce.is_empty()
        && nonce
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !(id_ok && nonce_ok) {
        return None;
    }
    Some((id.parse().ok()?, nonce))
}

const PENDING: &[&str] = &["pending"];

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

/// `resolveReturnUrl`: the stored relative return URL, else the completion
/// page.
fn return_url(session: &SessionRow) -> String {
    session
        .return_url
        .clone()
        .unwrap_or_else(|| format!("/connect/{}", session.id))
}

/// `appendConnectError` for a relative return URL.
fn append_connect_error(url: &str, code: &str) -> String {
    append_param(url, "connect_error", code)
}

/// `url` with `key=value` set (any old `key` dropped); relative only.
fn append_param(url: &str, key: &str, code: &str) -> String {
    let base = url::Url::parse("http://relative.invalid").expect("static url");
    let Ok(mut parsed) = base.join(url) else {
        return format!("/?{key}={code}");
    };
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| k != key)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    parsed
        .query_pairs_mut()
        .clear()
        .extend_pairs(pairs)
        .append_pair(key, code);
    let mut out = parsed.path().to_string();
    if let Some(query) = parsed.query() {
        out.push('?');
        out.push_str(query);
    }
    if let Some(fragment) = parsed.fragment() {
        out.push('#');
        out.push_str(fragment);
    }
    // `isSafeRelativeRedirect`: never a protocol-relative URL.
    if out.starts_with("//") || out.starts_with("/\\") {
        return format!("/?{key}={code}");
    }
    out
}

async fn handle(
    State(state): State<Arc<ApiState>>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let query: HashMap<String, String> =
        url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let raw = query.get("state").map(String::as_str).unwrap_or_default();
    let Some((session_id, nonce)) = split_state(raw) else {
        return not_found();
    };
    let kind = Kind::from_slug(&provider);
    let expected = kind.map_or(oauth::provider_for_slug(&provider), |kind| kind.provider());
    let session = match connect_session::find_by_nonce(&state.db, nonce).await {
        Ok(Some(row)) if row.id == session_id && row.provider == expected => row,
        Ok(_) => return not_found(),
        Err(error) => {
            tracing::error!(%error, "connect session lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if let Some(kind) = kind {
        return google_callback(&state, kind, &session, &query, &headers).await;
    }
    let return_to = return_url(&session);
    if let Some(error) = query.get("error") {
        let code = if error == "access_denied" {
            "provider_denied"
        } else {
            "provider_error"
        };
        connect_session::fail(&state.db, &session, code, Some(PENDING)).await;
        return redirect(&return_to);
    }
    let credential = match (
        state.encryption.as_deref(),
        oauth::adapter(&session.provider),
    ) {
        (Some(encryption), Some(_)) => {
            let owner = session
                .platform_owner_id
                .as_deref()
                .and_then(|id| id.parse::<i64>().ok());
            oauth::resolve_credential(
                &state.db,
                encryption,
                &state.app_url,
                &session.provider,
                owner,
            )
            .await
            .ok()
            .flatten()
            .map(|credential| (encryption, credential))
        }
        _ => None,
    };
    let Some((encryption, credential)) = credential.filter(|_| session.platform_owner_id.is_some())
    else {
        tracing::error!(session_id, provider = %session.provider, "connect session provider is not OAuth-configured");
        connect_session::fail(&state.db, &session, "internal_error", Some(PENDING)).await;
        return not_found();
    };
    let code = query.get("code").map(String::as_str).unwrap_or_default();
    let completed = oauth::complete_authorization(
        &state.db,
        encryption,
        &HttpUpstream,
        session.id,
        nonce,
        code,
        &credential,
    )
    .await;
    // A non-multi-account grant has one selectable target: an API / MCP
    // session (no `actorUserId`) connects it now; a builder session's picker
    // page does it from its "Continue" button.
    let completed = match completed {
        Ok(done)
            if done.status == "awaiting_selection"
                && done.actor_user_id.is_none()
                && oauth::adapter(&done.provider).is_some_and(|a| !a.multi_account) =>
        {
            let only = done.targets.get(0).filter(|t| t["selectable"] == true);
            match only.and_then(|t| t["id"].as_str()) {
                Some(id) => oauth::connect_targets(
                    &state.db,
                    encryption,
                    &HttpUpstream,
                    done.id,
                    done.workspace_id,
                    &[id.to_string()],
                    None,
                )
                .await
                .map(|r| r.session),
                None => Ok(done),
            }
        }
        other => other,
    };
    match completed {
        Ok(_) => redirect(&return_to),
        Err(error) => match connect_error_code(&error) {
            Outcome::Ignore => redirect(&return_to),
            Outcome::Retryable => {
                redirect(&append_connect_error(&return_to, "provider_unavailable"))
            }
            Outcome::Failed(code) => {
                tracing::error!(%error, session_id, "connect session completeAuthorization failed");
                connect_session::fail(&state.db, &session, "internal_error", None).await;
                match code {
                    Some(code) => redirect(&append_connect_error(&return_to, code)),
                    None => redirect(&return_to),
                }
            }
        },
    }
}

/// The `googleCalendar` / `googleSheets` cases: the session must belong to
/// the signed-in user ([`google_oauth::complete`]). Calendar answers with
/// `externalCalendarConnect=success|error` on the return URL (as the
/// original); Sheets with `connect_error` on a failure.
async fn google_callback(
    state: &ApiState,
    kind: Kind,
    session: &SessionRow,
    query: &HashMap<String, String>,
    headers: &HeaderMap,
) -> Response {
    let return_to = session.return_url.clone().unwrap_or_else(|| match kind {
        Kind::Calendar => format!(
            "/space/{}/appointment-calendars/external",
            session.workspace_id
        ),
        Kind::Sheets => format!(
            "/space/{}/settings/integrations/google-sheets",
            session.workspace_id
        ),
    });
    let finish = |ok: bool| match (kind, ok) {
        (Kind::Calendar, ok) => redirect(&append_param(
            &return_to,
            "externalCalendarConnect",
            if ok { "success" } else { "error" },
        )),
        (Kind::Sheets, true) => redirect(&return_to),
        (Kind::Sheets, false) => redirect(&append_connect_error(&return_to, "provider_error")),
    };
    if let Some(error) = query.get("error") {
        let code = if error == "access_denied" {
            "provider_denied"
        } else {
            "provider_error"
        };
        connect_session::fail(&state.db, session, code, Some(PENDING)).await;
        return finish(false);
    }
    let user_id = match state.sessions.as_deref() {
        Some(auth) => crate::rpc::find_rpc_session(auth, headers)
            .await
            .ok()
            .flatten()
            .map(|s| s.user_id),
        None => None,
    };
    let google = crate::handlers::appointments::externals(state);
    let code = query.get("code").map(String::as_str).unwrap_or_default();
    match google_oauth::complete(&state.db, &google, &state.app_url, session, user_id, code).await {
        Ok(_) => {
            if kind == Kind::Sheets {
                state
                    .audit
                    .record(chatbotx_business::audit::AuditRecordInput {
                        user_id: user_id.map(|id| id.to_string()),
                        workspace_id: Some(session.workspace_id.to_string()),
                        ip_address: Some(crate::auth::guest_client_ip(headers)),
                        user_agent: headers
                            .get(header::USER_AGENT)
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_string),
                        ..chatbotx_business::audit::AuditRecordInput::new(
                            "connect",
                            "connected a new Google Sheets integration".to_string(),
                        )
                    })
                    .await;
            }
            finish(true)
        }
        // A callback for another user (or a replay) is not this session's.
        Err(error) if error.code() == Some("connectionStateMismatch") => not_found(),
        Err(error) => {
            tracing::error!(%error, session_id = session.id, "Failed to connect Google from OAuth callback");
            finish(false)
        }
    }
}

enum Outcome {
    /// `isBenignReplay`.
    Ignore,
    /// `isRetryable`.
    Retryable,
    /// `unexpectedConnectErrorCode` (`None` = the session's own code).
    Failed(Option<&'static str>),
}

fn connect_error_code(error: &BusinessError) -> Outcome {
    match error.code() {
        Some("connectionStateMismatch" | "connectSessionExpired") => Outcome::Ignore,
        Some("connectionProviderUnavailable") => Outcome::Retryable,
        Some("connectionNoCandidates") => Outcome::Failed(Some("no_candidates")),
        Some("connectionCredentialsRejected") => Outcome::Failed(None),
        _ => Outcome::Failed(Some("internal_error")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_pattern() {
        assert_eq!(split_state("12.ab_C-9"), Some((12, "ab_C-9")));
        assert_eq!(split_state("12."), None);
        assert_eq!(split_state(".ab"), None);
        assert_eq!(split_state("1a.ab"), None);
        assert_eq!(split_state("12.a.b"), None);
        assert_eq!(split_state("{\"x\":1}"), None);
    }

    #[test]
    fn appends_connect_error() {
        assert_eq!(
            append_connect_error("/connect/5", "no_candidates"),
            "/connect/5?connect_error=no_candidates"
        );
        assert_eq!(
            append_connect_error("/s?tab=a#x", "internal_error"),
            "/s?tab=a&connect_error=internal_error#x"
        );
    }
}
