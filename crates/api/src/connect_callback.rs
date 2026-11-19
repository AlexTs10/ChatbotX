//! `GET /integrations/{provider}/callback` for a connect session state
//! (`handleConnectSessionCallback` / `runConnectSessionCallback` of
//! `apps/builder/src/app/integrations/[...integration]/callback.ts`). The
//! legacy JSON-state flows of that route answer 404; the Meta ads flows
//! (`facebookLeadAds`, `facebookMarketingMessages`, `messagingAds`) are
//! connect sessions too (`ads::connect`). No relay to `originHost`: a session only stores the app's own
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
use chatbotx_business::connection::{oauth, oauth_facebook_ads};
use chatbotx_business::google_oauth::{self, Kind};
use chatbotx_business::{payment_provider, tenant, tx};
use chatbotx_db::repo;
use chatbotx_db::rusqlite::OptionalExtension as _;
use serde_json::json;

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
        // The Facebook Ads grant returns through the Messenger callback (the
        // only `redirect_uri` the Facebook app registers).
        Ok(Some(row))
            if row.id == session_id
                && (row.provider == expected
                    || (expected == "messenger"
                        && row.provider == oauth_facebook_ads::PROVIDER)) =>
        {
            row
        }
        Ok(_) => return not_found(),
        Err(error) => {
            tracing::error!(%error, "connect session lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if let Some(kind) = kind {
        return google_callback(&state, kind, &session, &query, &headers).await;
    }
    if chatbotx_business::ads::connect::flow_of(&session).is_some() {
        return ads_callback(&state, &session, &query, &headers).await;
    }
    if session.provider == "stripe" {
        return stripe_callback(&state, session, &query, &headers).await;
    }
    if session.provider == oauth_facebook_ads::PROVIDER {
        return facebook_ads_callback(&state, &session, &query, &headers).await;
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

/// The `flow: "facebookAds"` branch of the Messenger callback: the session
/// must belong to the signed-in user; the token is stored and audited
/// (`integrationFacebookAdsService.upsert`), then back to the settings.
async fn facebook_ads_callback(
    state: &ApiState,
    session: &SessionRow,
    query: &HashMap<String, String>,
    headers: &HeaderMap,
) -> Response {
    let return_to = return_url(session);
    if let Some(error) = query.get("error") {
        let code = if error == "access_denied" {
            "provider_denied"
        } else {
            "provider_error"
        };
        connect_session::fail(&state.db, session, code, Some(PENDING)).await;
        return redirect(&append_connect_error(&return_to, code));
    }
    let Some(encryption) = state.encryption.as_deref() else {
        return not_found();
    };
    let user_id = match state.sessions.as_deref() {
        Some(auth) => crate::rpc::find_rpc_session(auth, headers)
            .await
            .ok()
            .flatten()
            .map(|s| s.user_id),
        None => None,
    };
    let code = query.get("code").map(String::as_str).unwrap_or_default();
    match oauth_facebook_ads::complete(
        &state.db,
        encryption,
        &HttpUpstream,
        &state.app_url,
        session,
        user_id,
        code,
    )
    .await
    {
        Ok(stored) => {
            let (action, detail) = stored.audit();
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
                    ..chatbotx_business::audit::AuditRecordInput::new(action, detail.to_string())
                })
                .await;
            redirect(&return_to)
        }
        Err(error) if error.code() == Some("connectionStateMismatch") => not_found(),
        Err(error) => {
            tracing::error!(%error, session_id = session.id, "Failed to store the Facebook Ads connection");
            redirect(&append_connect_error(&return_to, "provider_error"))
        }
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

/// `app/integrations/stripe/callback/route.ts`: the state is taken once
/// (Stripe revokes a connection whose code is used twice); only the member
/// who pressed Connect, still a super admin, finishes it. Answers on the
/// Stripe row with `stripe=connected|error|expired`.
async fn stripe_callback(
    state: &ApiState,
    session: SessionRow,
    query: &HashMap<String, String>,
    headers: &HeaderMap,
) -> Response {
    let ws = session.workspace_id;
    let return_to = return_url(&session);
    let Ok(session) = connect_session::claim_authorization(&state.db, session.id, ws).await else {
        return redirect(&with_outcome(&return_to, &[("stripe", "expired")]));
    };
    let refuse = |reason: &str| stripe_refuse(state, &session, &return_to, reason.to_string());
    let code = query.get("code").filter(|c| !c.is_empty());
    let (Some(code), None) = (code, query.get("error")) else {
        let reason = query.get("error").map_or("missingCode", String::as_str);
        return refuse(reason).await;
    };
    let user_id = match state.sessions.as_deref() {
        Some(auth) => crate::rpc::find_rpc_session(auth, headers)
            .await
            .ok()
            .flatten()
            .map(|s| s.user_id),
        None => None,
    };
    let Some(user_id) = user_id else {
        return refuse("signInRequired").await;
    };
    // A callback URL someone approved with their own Stripe account and
    // passed on never connects it here.
    if session.actor_user_id != Some(user_id) {
        return refuse("startedByAnotherUser").await;
    }
    if !is_workspace_super_admin(state, ws, user_id).await {
        return refuse("superAdminRequired").await;
    }
    let Some(encryption) = state.encryption.as_deref() else {
        return refuse("connectionFailed").await;
    };
    let ctx = payment_provider::Ctx {
        db: &state.db,
        encryption,
        app_url: &state.app_url,
    };
    let credential = session
        .next_action
        .as_ref()
        .and_then(|a| a["platformCredentialId"].as_str())
        .map(str::to_string);
    let connected = match tenant::resolve_credential_owner_id_for_workspace(&state.db, ws, None)
        .await
    {
        Ok(owner) => {
            payment_provider::complete_stripe_connect(ctx, ws, owner, credential.as_deref(), code)
                .await
        }
        Err(error) => Err(error),
    };
    let view = match connected {
        Ok(view) => view,
        Err(error) => {
            if error.code().is_none() {
                tracing::error!(%error, workspace_id = ws, "[stripe-connect] the connection could not be completed");
            }
            return refuse(error.code().unwrap_or("connectionFailed")).await;
        }
    };
    let result = json!({ "provider": "stripe", "accountId": view["accountId"] });
    let completed = tx::write(&state.db, {
        let session = session.clone();
        move |conn| {
            connect_session::complete_authorized(conn, &session, result, None, repo::now_ms())
        }
    })
    .await;
    if let Err(error) = completed {
        tracing::warn!(%error, session_id = session.id, "[stripe-connect] could not complete the connect session");
    }
    state
        .audit
        .record(chatbotx_business::audit::AuditRecordInput {
            user_id: Some(user_id.to_string()),
            workspace_id: Some(ws.to_string()),
            ip_address: Some(crate::auth::guest_client_ip(headers)),
            user_agent: headers
                .get(header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string),
            ..chatbotx_business::audit::AuditRecordInput::new(
                "connect",
                "connected Stripe for the web shop".to_string(),
            )
        })
        .await;
    // Test mode takes test payments before the account is activated.
    let outcome: &[(&str, &str)] = if view["livemode"] == true && view["chargesEnabled"] == false {
        &[("stripe", "connected"), ("chargesEnabled", "false")]
    } else {
        &[("stripe", "connected")]
    };
    redirect(&with_outcome(&return_to, outcome))
}

/// `withOutcome`: `return_to` with the outcome params set.
fn with_outcome(return_to: &str, outcome: &[(&str, &str)]) -> String {
    outcome.iter().fold(return_to.to_string(), |url, (k, v)| {
        append_param(&url, k, v)
    })
}

/// A refused Stripe callback: the session fails; `stripe=error&reason=…`.
async fn stripe_refuse(
    state: &ApiState,
    session: &SessionRow,
    return_to: &str,
    reason: String,
) -> Response {
    connect_session::fail(&state.db, session, "provider_error", None).await;
    redirect(&with_outcome(
        return_to,
        &[("stripe", "error"), ("reason", &reason)],
    ))
}

/// The Meta ads flows: only the member who pressed Connect finishes the
/// session (messaging ads: still a super admin, and the integration is the
/// workspace's); then back to the page that started it.
async fn ads_callback(
    state: &ApiState,
    session: &SessionRow,
    query: &HashMap<String, String>,
    headers: &HeaderMap,
) -> Response {
    use chatbotx_business::ads::connect;
    let return_to = return_url(session);
    if query.contains_key("error") {
        connect_session::fail(&state.db, session, "provider_denied", Some(PENDING)).await;
        return redirect(&return_to);
    }
    let user_id = match state.sessions.as_deref() {
        Some(auth) => crate::rpc::find_rpc_session(auth, headers)
            .await
            .ok()
            .flatten()
            .map(|s| s.user_id),
        None => None,
    };
    let ws = session.workspace_id;
    let (is_member, super_admin) = match user_id {
        Some(user) => {
            let member = tx::read(&state.db, move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT 1 FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
                        [ws, user],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some())
            })
            .await
            .unwrap_or(false);
            (
                member,
                member && is_workspace_super_admin(state, ws, user).await,
            )
        }
        None => (false, false),
    };
    let Some(encryption) = state.encryption.as_deref() else {
        connect_session::fail(&state.db, session, "internal_error", Some(PENDING)).await;
        return not_found();
    };
    let code = query.get("code").map(String::as_str).unwrap_or_default();
    let caller = connect::Caller {
        user_id,
        is_member,
        super_admin,
    };
    match connect::complete(
        &state.db,
        encryption,
        &state.app_url,
        &HttpUpstream,
        session,
        caller,
        code,
    )
    .await
    {
        Ok(true) => redirect(&return_to),
        Ok(false) => not_found(),
        Err(error) => {
            tracing::error!(%error, session_id = session.id, "Meta ads connect callback failed");
            redirect(&append_connect_error(&return_to, "provider_error"))
        }
    }
}

/// `assertWorkspaceSuperAdmin` for the signed-in member.
async fn is_workspace_super_admin(state: &ApiState, workspace_id: i64, user_id: i64) -> bool {
    let permissions = tx::read(&state.db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT "permissions" FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
                [workspace_id, user_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    })
    .await;
    matches!(permissions, Ok(Some(p)) if serde_json::from_str::<serde_json::Value>(&p)
        .is_ok_and(|p| p["superAdmin"] == true))
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
    fn stripe_outcome_stays_relative() {
        assert_eq!(
            with_outcome(
                "/space/1/settings/integrations/stripe",
                &[("stripe", "error"), ("reason", "a&b=//evil")]
            ),
            "/space/1/settings/integrations/stripe?stripe=error&reason=a%26b%3D%2F%2Fevil"
        );
        assert_eq!(
            with_outcome("//evil.example", &[("stripe", "connected")]),
            "/?stripe=connected"
        );
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
