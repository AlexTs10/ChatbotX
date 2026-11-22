//! The select-page connect procedures (Q.D12-channels):
//! `integrationMessengerAPIs.connectMessengerPageAPI` (`{ sessionId, pageId }`),
//! `integrationInstagramAPIs.connectInstagramAccountAPI` and
//! `connectInstagramFacebookAccountAPI` (`{ sessionId, igId }`), and the
//! select page data (`resolveSelectSession`). The workspace and the token
//! come only from the `ConnectSession` row (`connectSessionCandidate`);
//! every failure is a typed result in a 200 answer.

use chatbotx_business::connect_session::{self, SessionRow};
use chatbotx_business::connection::credentials::HttpUpstream;
use chatbotx_business::connection::oauth;
use chatbotx_business::{BusinessError, tx};
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use super::RpcContext;
use super::workspace::RpcOut;
use crate::handlers::ApiState;
use crate::orpc::OrpcError;

/// `(procedure, provider, target input key, satellite table)`.
const PROCEDURES: &[(&str, &str, &str, &str)] = &[
    (
        "integrationMessengerAPIs.connectMessengerPageAPI",
        "messenger",
        "pageId",
        "IntegrationMessenger",
    ),
    (
        "integrationInstagramAPIs.connectInstagramAccountAPI",
        "instagram",
        "igId",
        "IntegrationInstagram",
    ),
    (
        "integrationInstagramAPIs.connectInstagramFacebookAccountAPI",
        "instagramFacebook",
        "igId",
        "IntegrationInstagram",
    ),
];

pub async fn call(
    api: &ApiState,
    path: &str,
    context: &RpcContext,
    input: &Value,
) -> Option<Result<RpcOut, OrpcError>> {
    if path == "integrationWhatsappAPIs.connectWhatsappNumberAPI" {
        return Some(whatsapp(api, context, input).await);
    }
    let (_, provider, key, table) = PROCEDURES.iter().find(|p| p.0 == path)?;
    let text = |k: &str| {
        input[k]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let (Some(session_id), Some(target)) = (text("sessionId"), text(key)) else {
        return Some(Err(bad_input()));
    };
    let user_id = context.session.user_id;
    Some(Ok((
        connect(api, user_id, &session_id, &target, provider, table).await,
        Vec::new(),
    )))
}

/// `connectWhatsappNumberAPI` (`connectWhatsappViaSessionRequest`): the
/// picker's one request per number; `transferPhoneNumber: true` is refused.
async fn whatsapp(
    api: &ApiState,
    context: &RpcContext,
    input: &Value,
) -> Result<RpcOut, OrpcError> {
    let text = |k: &str| {
        input[k]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let flag = |k: &str| input.get(k).is_none_or(|v| v.is_null() || v.is_boolean());
    let (Some(signup_session_id), Some(phone_number_id), Some(connect_existing)) = (
        text("signupSessionId"),
        text("phoneNumberId"),
        input["connectExisting"].as_bool(),
    ) else {
        return Err(bad_input());
    };
    if !flag("transferPhoneNumber")
        || !flag("marketingMessageLite")
        || input["transferPhoneNumber"] == true
    {
        return Err(bad_input());
    }
    let request = crate::handlers::whatsapp_connect::SessionConnect {
        signup_session_id,
        phone_number_id,
        connect_existing,
    };
    let user_id = context.session.user_id;
    Ok((
        crate::handlers::whatsapp_connect::connect_number(api, user_id, &request).await,
        Vec::new(),
    ))
}

fn bad_input() -> OrpcError {
    let mut error = OrpcError::new("BAD_REQUEST", Some(400), Some("Input validation failed"));
    error.defined = false;
    error
}

pub(crate) fn session_error(code: &str) -> Value {
    json!({ "kind": "sessionError", "code": code })
}

pub(crate) fn outcome(source: &str, name: &str, fields: Value) -> Value {
    let mut out = json!({ "sourceId": source, "name": name, "coexistEligible": false });
    if let (Value::Object(target), Value::Object(extra)) = (&mut out, fields) {
        target.extend(extra);
    }
    json!({ "kind": "outcome", "outcome": out })
}

/// `toConnectActionFailure`: a session-level exception is a `sessionError`,
/// anything else a failed item.
pub(crate) fn failure(error: &BusinessError, source: &str, name: &str) -> Value {
    let code = match error {
        BusinessError::Exception { code, .. } => code.as_str(),
        _ => "",
    };
    let session = match code {
        "connectSessionExpired" | "signupSessionExpired" => Some("sessionExpired"),
        "notWorkspaceMember" => Some("notMember"),
        "trialExpired" | "macLimitReached" | "credentialMissing" => Some(code),
        _ => None,
    };
    if let Some(code) = session {
        return session_error(code);
    }
    tracing::error!(error = %error, "Failed to connect a channel from the select page");
    let (status, reason) = match code {
        "channelDuplicated" | "connectionAlreadyConnected" => ("duplicated", "alreadyConnected"),
        "channelLimitReached" => ("limitReached", "channelLimit"),
        "workspaceLimitReached" => ("limitReached", "workspaceLimit"),
        _ => ("failed", "unknown"),
    };
    outcome(source, name, json!({ "status": status, "reason": reason }))
}

fn expired() -> BusinessError {
    BusinessError::exception(
        "Your connect session expired. Please start again.",
        "connectSessionExpired",
        410,
    )
}

/// `resolveConnectSessionCore`: the user's own `connect` session of this
/// provider, awaiting selection, in a workspace the user is a member of.
pub async fn resolve_session(
    api: &ApiState,
    user_id: i64,
    session_id: i64,
    provider: &str,
) -> Result<SessionRow, BusinessError> {
    let workspace: Option<i64> = tx::read(&api.db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT "workspaceId" FROM "ConnectSession" WHERE "id" = ?1"#,
                params![session_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(chatbotx_db::repo::Error::from)?)
    })
    .await?;
    let Some(workspace) = workspace else {
        return Err(expired());
    };
    let session = connect_session::find_for_workspace(&api.db, session_id, workspace)
        .await?
        .filter(|s| s.actor_user_id == Some(user_id) && s.provider == provider)
        .ok_or_else(expired)?;
    if session.status == "failed" && session.error_code.as_deref() == Some("provider_denied") {
        return Err(BusinessError::exception(
            "The connection was cancelled.",
            "connectSessionCancelled",
            400,
        ));
    }
    if session.status != "awaiting_selection" || session.purpose != "connect" {
        return Err(expired());
    }
    let member: Option<i64> = tx::read(&api.db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT m."id" FROM "WorkspaceMember" m JOIN "Workspace" w ON w."id" = m."workspaceId"
                   WHERE m."workspaceId" = ?1 AND m."userId" = ?2"#,
                params![workspace, user_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(chatbotx_db::repo::Error::from)?)
    })
    .await?;
    if member.is_none() {
        return Err(BusinessError::exception(
            "You are not a member of this workspace.",
            "notWorkspaceMember",
            403,
        ));
    }
    Ok(session)
}

/// `connectSessionCandidate`.
async fn connect(
    api: &ApiState,
    user_id: i64,
    session_id: &str,
    target_id: &str,
    provider: &str,
    table: &'static str,
) -> Value {
    let Ok(id) = session_id.parse::<i64>() else {
        return session_error("sessionExpired");
    };
    let session = match resolve_session(api, user_id, id, provider).await {
        Ok(session) => session,
        Err(error) => return failure(&error, target_id, target_id),
    };
    if session.platform_owner_id.is_none() {
        return session_error("credentialMissing");
    }
    let Some(target) = session
        .targets
        .as_array()
        .and_then(|t| t.iter().find(|t| t["id"] == target_id))
    else {
        return outcome(
            target_id,
            target_id,
            json!({ "status": "failed", "reason": "notSelectable" }),
        );
    };
    let name = target["name"].as_str().unwrap_or(target_id).to_string();
    if target["selectable"] != true {
        return if target["alreadyConnected"].is_string() {
            outcome(
                target_id,
                &name,
                json!({ "status": "duplicated", "reason": "alreadyConnected" }),
            )
        } else {
            outcome(
                target_id,
                &name,
                json!({ "status": "failed", "reason": "notSelectable" }),
            )
        };
    }
    let Some(encryption) = api.encryption.as_deref() else {
        return session_error("credentialMissing");
    };
    let result = match oauth::connect_targets(
        &api.db,
        encryption,
        &HttpUpstream,
        session.id,
        session.workspace_id,
        &[target_id.to_string()],
        Some(user_id),
    )
    .await
    {
        Ok(result) => result,
        Err(error) => return failure(&error, target_id, &name),
    };
    let first = result.outcomes.first().cloned().unwrap_or(Value::Null);
    let Some(connection) = result
        .connections
        .first()
        .filter(|_| first["status"] == "connected")
    else {
        tracing::error!(session_id = session.id, target_id, outcome = %first, "connectTargets returned no connected outcome for a selectable target");
        let mut fields = json!({
            "status": first["status"].as_str().unwrap_or("failed"),
            "reason": first["reason"].as_str().unwrap_or("unknown"),
        });
        if let Some(detail) = first["detail"].as_str() {
            fields["detail"] = json!(detail);
        }
        return outcome(target_id, &name, fields);
    };
    let inbox = connection.inbox_id;
    let row: Option<i64> = match inbox {
        Some(inbox) => tx::read(&api.db, move |conn| {
            Ok(conn
                .query_row(
                    &format!(r#"SELECT "id" FROM "{table}" WHERE "inboxId" = ?1"#),
                    params![inbox],
                    |r| r.get(0),
                )
                .optional()
                .map_err(chatbotx_db::repo::Error::from)?)
        })
        .await
        .ok()
        .flatten(),
        None => None,
    };
    // ponytail: the branding menu seed and the Messenger tag-sync scan of the
    // original follow-ups have no Rust service yet; only the warnings remain.
    let warning = if connection.status == "degraded" {
        Some("webhookSubscribeFailed")
    } else if row.is_none() {
        Some("followUpFailed")
    } else {
        None
    };
    let mut fields = json!({
        "status": "connected",
        "integrationId": row.map(|id| id.to_string()),
        "coexistEligible": row.is_some(),
    });
    if let Some(warning) = warning {
        fields["warning"] = json!(warning);
    }
    outcome(target_id, &name, fields)
}

/// `ACTIONABLE_FAILURES`: a stored session failure as the picker's error.
fn actionable(code: &str, no_candidates: &str) -> Option<String> {
    Some(
        match code {
            "no_candidates" => no_candidates,
            "exchange_failed" => "exchangeFailed",
            "provider_error" | "provider_unavailable" => "providerError",
            "internal_error" => "connectFailed",
            _ => return None,
        }
        .to_owned(),
    )
}

/// `resolveSelectSession` of `/channels/<channel>/select`: `{ sessionId,
/// workspaceId, targets }`, or `{ redirect }` (`/channels/create?error=…`;
/// a cancelled dialog goes back to the picker).
pub async fn select_page(api: &ApiState, user_id: i64, input: &Value) -> Value {
    let text = |k: &str| input[k].as_str().filter(|s| !s.is_empty());
    let provider = text("provider").unwrap_or_default();
    let no_candidates = match provider {
        "messenger" => "noPages",
        _ => "noInstagramAccounts",
    };
    let picker = |error: &str, ws: Option<i64>| json!({ "redirect": crate::channel_connect::create_channel_path(Some(error), ws) });
    let Some(id) = text("session").and_then(|s| s.parse::<i64>().ok()) else {
        return json!({ "redirect": "/channels/create" });
    };
    match resolve_session(api, user_id, id, provider).await {
        Ok(session) => json!({
            "sessionId": session.id.to_string(),
            "workspaceId": session.workspace_id.to_string(),
            "targets": session.targets,
        }),
        Err(BusinessError::Exception { code, .. }) if code == "connectSessionCancelled" => {
            json!({ "redirect": "/channels/create" })
        }
        Err(BusinessError::Exception { code, .. }) if code == "connectSessionExpired" => {
            // The session of this user (if any) names its workspace and failure.
            let row = tx::read(&api.db, move |conn| {
                Ok(conn
                    .query_row(
                        r#"SELECT "workspaceId", "status", "errorCode", "actorUserId" FROM "ConnectSession" WHERE "id" = ?1"#,
                        params![id],
                        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<i64>>(3)?)),
                    )
                    .optional()
                    .map_err(chatbotx_db::repo::Error::from)?)
            })
            .await
            .ok()
            .flatten()
            .filter(|row| row.3 == Some(user_id));
            let workspace = row.as_ref().map(|r| r.0);
            let stored = row
                .as_ref()
                .filter(|r| r.1 == "failed")
                .and_then(|r| r.2.as_deref())
                .and_then(|c| actionable(c, no_candidates));
            let code = stored
                .or_else(|| text("connectError").and_then(|c| actionable(c, no_candidates)))
                .unwrap_or_else(|| "sessionExpired".into());
            picker(&code, workspace)
        }
        Err(BusinessError::Exception { code, .. }) if code == "notWorkspaceMember" => {
            picker("notMember", None)
        }
        Err(error) => {
            tracing::warn!(error = %error, session_id = id, "resolveConnectSession rejected the select page");
            picker("sessionExpired", None)
        }
    }
}
