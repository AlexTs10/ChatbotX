//! The Google Ads settings actions (`features/integration-google-ads/
//! actions/*`) and the `googleAdsAPI` procedures the settings page and the
//! flow / trigger editors read. The connect itself is the generic OAuth
//! engine (`connection::oauth`, adapter `googleAds`).

use chatbotx_business::BusinessError;
use chatbotx_business::connect_session;
use chatbotx_business::connection::credentials::HttpUpstream;
use chatbotx_business::connection::oauth::{self, StartSession};
use chatbotx_business::google_ads_setup::{self as setup, Setup};
use chatbotx_business::tx;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid};
use crate::orpc::OrpcError;

pub(super) const ACTIONS: &[&str] = &[
    "startGoogleAdsConnectAction",
    "startGoogleAdsReconnectAction",
    "pickGoogleAdsAccountAction",
    "cancelGoogleAdsConnectAction",
    "disconnectGoogleAdsAction",
    "syncGoogleAdsConversionActionsAction",
    "retryGoogleAdsEventAction",
    "updateGoogleAdsConsentAction",
    "validateGoogleAdsRequestAction",
    "googleAdsSettingsPage",
];

const PROVIDER: &str = "googleAds";
/// `errors.superAdminRequired` (en).
const SUPER_ADMIN_REQUIRED: &str = "You need to be a workspace super admin to do this.";

/// `googleAdsException(key, status)` with the `googleAds.errors.*` (en) text.
fn google_ads_error(key: &str, status: u16) -> BusinessError {
    let message = match key {
        "developerTokenMissing" => {
            "Google Ads isn't available yet: the platform's Google Ads credentials (OAuth client ID and secret) are not configured."
        }
        "notConnected" => "Google Ads is not connected for this workspace.",
        "sessionNotFound" => {
            "This Google Ads connection attempt was not found or has expired. Start again."
        }
        "pickFailed" => {
            "Couldn't connect that Google Ads account. It may already be connected elsewhere or the attempt expired."
        }
        _ => "Couldn't start the Google sign-in. Try again in a moment.",
    };
    BusinessError::exception(message, &format!("googleAds.{key}"), status)
}

/// `assertWorkspaceSuperAdmin` (support sessions do not exist in Rust, so
/// `assertCanManageGoogleAds` is the same check).
fn assert_super_admin(ctx: &Ctx<'_>) -> Result<(), BusinessError> {
    if ctx.has("superAdmin") {
        Ok(())
    } else {
        Err(BusinessError::exception(
            SUPER_ADMIN_REQUIRED,
            "systemError",
            400,
        ))
    }
}

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, _input: &Value) -> Outcome {
    Some(match path {
        "googleAdsAPI.getIntegration" => {
            chatbotx_business::google_ads::get_connection(&ctx.api.db, ctx.workspace_id)
                .await
                .map_err(fail)
        }
        "googleAdsAPI.getInFlightConnectSession" => in_flight(ctx).await.map_err(fail),
        "googleAdsAPI.listEvents" => list_events(ctx, _input).await,
        "connectSessionsAPI.getConnectSessionAPI" => {
            match In(_input).str("id", false, 0, usize::MAX) {
                Ok(id) => {
                    super::public(ctx, "connectSessions.get", &[("id", id)], None, None).await
                }
                Err(e) => Err(e),
            }
        }
        _ => return None,
    })
}

/// `findLatestInFlightByProvider`: the newest unexpired active session.
async fn in_flight(ctx: &Ctx<'_>) -> Result<Value, BusinessError> {
    assert_super_admin(ctx)?;
    Ok(json!({ "session": latest_in_flight(ctx).await? }))
}

/// `findLatestInFlightByProvider` (workspace and `googleAds` scoped).
async fn latest_in_flight(ctx: &Ctx<'_>) -> Result<Value, BusinessError> {
    let ws = ctx.workspace_id;
    let session: Option<(i64, String)> = tx::read(&ctx.api.db, move |conn| {
        conn.query_row(
            r#"SELECT "id", "status" FROM "ConnectSession"
               WHERE "workspaceId" = ?1 AND "provider" = 'googleAds'
                 AND "status" IN ('pending', 'authorized', 'awaiting_selection') AND "expiresAt" > ?2
               ORDER BY "createdAt" DESC, "id" DESC LIMIT 1"#,
            params![ws, chatbotx_db::repo::now_ms()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await?;
    Ok(json!(session.map(
        |(id, status)| json!({ "id": id.to_string(), "status": status })
    )))
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let input = arg(args, 1);
    match name {
        "startGoogleAdsConnectAction" => start(ctx, false).await.map_err(action_fail),
        "startGoogleAdsReconnectAction" => start(ctx, true).await.map_err(action_fail),
        "pickGoogleAdsAccountAction" => {
            let session_id = In(input).id("sessionId")?;
            let customer = In(input).str("customerId", false, 10, 10)?;
            if !customer.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
            pick(ctx, session_id, customer).await.map_err(action_fail)
        }
        "cancelGoogleAdsConnectAction" => {
            let session_id = In(input).id("sessionId")?;
            cancel(ctx, session_id).await.map_err(action_fail)
        }
        "disconnectGoogleAdsAction" => disconnect(ctx).await.map_err(action_fail),
        "retryGoogleAdsEventAction" => {
            let event_id = In(input).id("eventId")?;
            retry(ctx, event_id).await.map_err(action_fail)
        }
        "updateGoogleAdsConsentAction" => {
            assert_super_admin(ctx).map_err(action_fail)?;
            chatbotx_business::google_ads::update_consent(&ctx.api.db, ctx.workspace_id, input)
                .await
                .map_err(action_fail)
        }
        "validateGoogleAdsRequestAction" => validate(ctx, input).await,
        "googleAdsSettingsPage" => settings_page(ctx, arg(args, 0)).await.map_err(action_fail),
        _ => sync(ctx).await.map_err(action_fail),
    }
}

fn settings_path(ws: i64) -> String {
    format!("/space/{ws}/settings/integrations/google-ads")
}

/// `startGoogleAdsConnectAction` / `startGoogleAdsReconnectAction`: a fresh
/// `pending` session (older pending ones cancelled) whose return URL names
/// it (`?session={id}`). The original `redirect(consentUrl)`; here `{ url }`.
async fn start(ctx: &Ctx<'_>, reconnect: bool) -> Result<Value, BusinessError> {
    assert_super_admin(ctx)?;
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let encryption = ctx
        .api
        .encryption
        .as_deref()
        .ok_or_else(|| google_ads_error("developerTokenMissing", 409))?;
    let credential = setup::credential(db, encryption, &ctx.api.app_url, ws).await?;
    let Some(credential) = credential.filter(|c| setup::is_configured(Some(c))) else {
        return Err(google_ads_error("developerTokenMissing", 409));
    };
    let target = if reconnect {
        let found = setup::get_setup(db, ws)
            .await?
            .and_then(|s| s.connection_id);
        Some(found.ok_or_else(|| google_ads_error("notConnected", 404))?)
    } else {
        None
    };
    connect_session::cancel_pending_by_provider(db, ws, PROVIDER).await?;
    let owner =
        chatbotx_business::tenant::resolve_credential_owner_id_for_workspace(db, ws, None).await?;
    let origin_host = url::Url::parse(&ctx.api.app_url).ok().and_then(|u| {
        u.host_str().map(|h| match u.port() {
            Some(port) => format!("{h}:{port}"),
            None => h.to_string(),
        })
    });
    let session = oauth::start_session(
        db,
        &credential,
        StartSession {
            workspace_id: ws,
            provider: PROVIDER.to_string(),
            purpose: if reconnect { "reconnect" } else { "connect" },
            target_connection_id: target,
            platform_owner_id: Some(owner),
            origin_host,
            return_url: Some(settings_path(ws)),
            actor: Some(oauth::Actor::User(ctx.user_id)),
        },
    )
    .await?;
    let url = session
        .next_action
        .as_ref()
        .filter(|a| a["type"] == "open_url")
        .and_then(|a| a["url"].as_str())
        .ok_or_else(|| google_ads_error("authorizationUnavailable", 502))?
        .to_string();
    let base = session
        .return_url
        .clone()
        .unwrap_or_else(|| settings_path(ws));
    let separator = if base.contains('?') { '&' } else { '?' };
    connect_session::update_return_url(
        db,
        session.id,
        format!("{base}{separator}session={}", session.id),
    )
    .await?;
    Ok(json!({ "url": url }))
}

/// `requireGoogleAdsSession`: a foreign / unknown / other-provider id all
/// answer the same 404.
async fn require_session(
    ctx: &Ctx<'_>,
    session_id: i64,
) -> Result<connect_session::SessionRow, BusinessError> {
    connect_session::find_for_workspace(&ctx.api.db, session_id, ctx.workspace_id)
        .await?
        .filter(|s| s.provider == PROVIDER)
        .ok_or_else(|| google_ads_error("sessionNotFound", 404))
}

fn setup_view(setup: Option<&Setup>) -> Value {
    json!({
        "readiness": setup.map(|s| s.readiness),
        "setupError": setup.and_then(|s| s.integration.setup_error.clone()),
    })
}

/// `pickGoogleAdsAccountAction`: connects the chosen customer, closes the
/// selection (the other candidates' tokens are dropped) and syncs the setup.
async fn pick(ctx: &Ctx<'_>, session_id: i64, customer: String) -> Result<Value, BusinessError> {
    assert_super_admin(ctx)?;
    require_session(ctx, session_id).await?;
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let encryption = ctx
        .api
        .encryption
        .as_deref()
        .ok_or_else(|| google_ads_error("pickFailed", 409))?;
    let result = oauth::connect_targets(
        db,
        encryption,
        &HttpUpstream,
        session_id,
        ws,
        std::slice::from_ref(&customer),
        Some(ctx.user_id),
    )
    .await?;
    let connected = result.outcomes.iter().any(|o| o["status"] == "connected");
    // `isRecoverableDuplicate`: a retry after the connection committed.
    let recovered = !connected
        && result
            .outcomes
            .iter()
            .any(|o| o["targetId"] == customer.as_str() && o["status"] == "duplicated")
        && setup::get_setup(db, ws).await?.is_some_and(|s| {
            s.integration.customer_id == customer && s.readiness != "needs_reauth"
        });
    if !(connected || recovered) {
        return Err(google_ads_error("pickFailed", 409));
    }
    connect_session::complete_selection(db, session_id, ws).await?;
    let credential = setup::credential(db, encryption, &ctx.api.app_url, ws).await?;
    let refreshed = setup::refresh_setup(db, &HttpUpstream, credential.as_ref(), ws).await?;
    Ok(setup_view(refreshed.as_ref()))
}

/// `cancelGoogleAdsConnectAction`.
async fn cancel(ctx: &Ctx<'_>, session_id: i64) -> Result<Value, BusinessError> {
    assert_super_admin(ctx)?;
    require_session(ctx, session_id).await?;
    connect_session::cancel(&ctx.api.db, session_id, ctx.workspace_id).await?;
    Ok(Value::Null)
}

/// `disconnectGoogleAdsAction`: revokes the grant (best effort) and
/// disconnects the connection; the satellite row and history stay.
async fn disconnect(ctx: &Ctx<'_>) -> Result<Value, BusinessError> {
    assert_super_admin(ctx)?;
    let db = &ctx.api.db;
    let current = setup::get_setup(db, ctx.workspace_id).await?;
    let Some((current, connection_id)) = current.and_then(|s| s.connection_id.map(|id| (s, id)))
    else {
        return Err(google_ads_error("notConnected", 404));
    };
    if let Err(error) = setup::revoke(&HttpUpstream, &current.integration).await {
        tracing::error!(connection_id, %error, "connection disconnect: provider-side disconnect failed");
    }
    chatbotx_business::connection::disconnect(db, connection_id, ctx.workspace_id).await?;
    Ok(Value::Null)
}

/// `syncGoogleAdsConversionActionsAction` (super admin; support sessions too).
async fn sync(ctx: &Ctx<'_>) -> Result<Value, BusinessError> {
    assert_super_admin(ctx)?;
    let db = &ctx.api.db;
    let credential = match ctx.api.encryption.as_deref() {
        Some(encryption) => {
            setup::credential(db, encryption, &ctx.api.app_url, ctx.workspace_id).await?
        }
        None => None,
    };
    let refreshed = setup::refresh_setup(db, &HttpUpstream, credential.as_ref(), ctx.workspace_id)
        .await?
        .ok_or_else(|| google_ads_error("notConnected", 404))?;
    Ok(setup_view(Some(&refreshed)))
}

/// `googleAdsAPI.listEvents` (private): super admin, the filters of
/// `listGoogleAdsEventsRequest`, `{ data, total, page, perPage }` (click ids
/// masked by the resource).
async fn list_events(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    assert_super_admin(ctx).map_err(fail)?;
    let pick = |key: &str, allowed: &[&str]| -> Result<Option<String>, OrpcError> {
        match &input[key] {
            Value::Null => Ok(None),
            Value::String(v) if allowed.is_empty() || allowed.contains(&v.as_str()) => {
                Ok(Some(v.clone()))
            }
            _ => Err(invalid()),
        }
    };
    let status = pick("status", chatbotx_business::google_ads::EVENT_STATUSES)?;
    let channel = pick(
        "channel",
        chatbotx_core::utils::google_click::GOOGLE_ADS_CHANNEL_VALUES,
    )?;
    let action = pick("conversionActionId", &[])?;
    if action
        .as_deref()
        .is_some_and(|a| a.is_empty() || !a.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(invalid());
    }
    let date = |key: &str| -> Result<Option<i64>, OrpcError> {
        match &input[key] {
            Value::Null => Ok(None),
            Value::String(v) => chatbotx_core::js::parse_date(v)
                .map(Some)
                .ok_or_else(invalid),
            Value::Number(n) => n.as_i64().map(Some).ok_or_else(invalid),
            _ => Err(invalid()),
        }
    };
    let int = |key: &str, default: i64| -> Result<i64, OrpcError> {
        let n = match &input[key] {
            Value::Null => return Ok(default),
            Value::Number(n) => n.as_f64(),
            Value::String(v) => v.trim().parse::<f64>().ok(),
            _ => None,
        };
        match n {
            Some(n) if n.fract() == 0.0 && n >= 1.0 => Ok(n as i64),
            _ => Err(invalid()),
        }
    };
    let page = int("page", 1)?;
    let (data, total, per_page) = chatbotx_business::google_ads::list_events_total(
        &ctx.api.db,
        chatbotx_db::repo::google_ads_conversion_event::ListInput {
            workspace_id: ctx.workspace_id,
            status,
            channel,
            conversion_action_id: action,
            since: date("since")?,
            until: date("until")?,
            page,
            per_page: int("perPage", 20)?,
        },
    )
    .await
    .map_err(fail)?;
    Ok(json!({ "data": data, "total": total, "page": page, "perPage": per_page }))
}

/// `retryGoogleAdsEventAction`: a non-`failed` event is `notRetryable` (409).
async fn retry(ctx: &Ctx<'_>, event_id: i64) -> Result<Value, BusinessError> {
    assert_super_admin(ctx)?;
    let jobs = ctx.api.jobs.as_deref().ok_or_else(|| {
        BusinessError::exception("Job queue is not configured", "systemError", 500)
    })?;
    let retried = chatbotx_business::google_ads_conversion::retry(
        &ctx.api.db,
        jobs,
        ctx.workspace_id,
        event_id,
    )
    .await?;
    if !retried {
        return Err(BusinessError::exception(
            "This conversion can't be retried: only failed conversions can be retried.",
            "googleAds.notRetryable",
            409,
        ));
    }
    Ok(Value::Null)
}

/// `validateGoogleAdsRequestAction`: super admin, the request schema, then
/// the `validateOnly` check; the click id never comes back.
async fn validate(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let action_id = In(input).str("conversionActionId", false, 1, 20)?;
    if !action_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let click_type = In(input).str("clickIdType", false, 1, 16)?;
    if !matches!(click_type.as_str(), "gclid" | "gbraid") {
        return Err(invalid());
    }
    // `CLICK_ID_PATTERN`: /^[A-Za-z0-9_-]{10,512}$/.
    let click_id = In(input).str("clickId", false, 10, 512)?;
    if !click_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(invalid());
    }
    assert_super_admin(ctx).map_err(action_fail)?;
    chatbotx_business::google_ads_conversion::validate_ingest(
        &ctx.api.db,
        &HttpUpstream,
        &ctx.api.app_url,
        ctx.workspace_id,
        &action_id,
        &click_type,
        &click_id,
    )
    .await
    .map_err(action_fail)
}

/// `loadGoogleAdsSettingsPage`: setup and consent views, `isConfigured`, and
/// the session to show (`loadSettingsSession`: the `?session=` id in any
/// status, else the newest unexpired `authorized` / `awaiting_selection`).
/// Like the original page, any member may load it (no secret is read); the
/// actions keep the super-admin check.
async fn settings_page(ctx: &Ctx<'_>, session_param: &Value) -> Result<Value, BusinessError> {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let mut page = chatbotx_business::google_ads::settings_page(db, ws).await?;
    let is_configured = match ctx.api.encryption.as_deref() {
        Some(encryption) => {
            let credential = setup::credential(db, encryption, &ctx.api.app_url, ws).await?;
            setup::is_configured(credential.as_ref())
        }
        None => false,
    };
    let requested = session_param
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= 20 && v.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|v| v.parse::<i64>().ok());
    let by_id = match requested {
        Some(id) => connect_session::find_for_workspace(db, id, ws)
            .await?
            .filter(|row| row.provider == PROVIDER)
            .map(|row| json!({ "id": row.id.to_string(), "status": row.status })),
        None => None,
    };
    let session = match by_id {
        Some(session) => Some(session),
        None => {
            let current = latest_in_flight(ctx).await?;
            matches!(
                current["status"].as_str(),
                Some("authorized" | "awaiting_selection")
            )
            .then_some(current)
        }
    };
    page["isConfigured"] = json!(is_configured);
    page["session"] = session.unwrap_or(Value::Null);
    Ok(page)
}
