//! The channel side of thread control: `@chatbotx.io/channel-registry/thread-control`
//! (`runOnChannel`, `getChannelThreadOwner`) with the WhatsApp
//! (`integrations/whatsapp/src/{handlers/conversation,api/thread-control}.ts`) and
//! Messenger (`integrations/messenger/src/{handlers/conversation,apis/thread-control,
//! lib/thread-control-config}.ts`) handlers. Only those two channels have
//! `updateThreadControl`; only Messenger has `getThreadOwner`.

use std::time::Duration;

use chatbotx_core::utils::graph_error::format_graph_error;
use chatbotx_db::Db;
use chatbotx_db::repo::models::ContactInbox;
use serde_json::{Map, Value, json};

use super::{ThreadControlError, roles};
use crate::errors::BusinessError;
use crate::tx;

/// `THREAD_CONTROL_METADATA_MAX_LENGTH`.
pub const METADATA_MAX_LENGTH: usize = 2000;
/// `THREAD_CONTROL_NOT_ESCALATION_CODE` (WhatsApp `take` by a non-escalation partner).
pub const NOT_ESCALATION_CODE: i64 = 2_494_191;
/// `BUSINESS_AI_APP_ID` (Meta's Business-AI agent app).
pub const BUSINESS_AI_APP_ID: &str = "622851382610562";
/// `BUSINESS_AI_APP_ID_ENV`.
const BUSINESS_AI_APP_ID_ENV: &str = "FACEBOOK_BUSINESS_AI_APP_ID";
const DEFAULT_API_VERSION: &str = "v23.0";

/// A `ChannelError` of a thread-control call.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelFailure {
    /// `formatGraphErrorMessage(...)` or the handler's own sentence.
    pub message: String,
    pub code: Option<Value>,
    /// `ThreadControlTakeRefusedError` (a refused `take`).
    pub take_refused: bool,
    /// `ChannelError.isPermanent`.
    pub permanent: bool,
}

/// `ThreadControlChannelResult`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChannelResult {
    pub owner_role: Option<&'static str>,
    pub owner_app_id: Option<String>,
}

/// `ThreadOwnerResult` of `getThreadOwner`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadOwner {
    pub owner_app_id: Option<String>,
    pub expires_at: Option<i64>,
    pub own_app_id: Option<String>,
    pub ai_agent_app_id: Option<String>,
}

/// `readBusinessAiAppId`: the env override, else the fixed id.
pub fn business_ai_app_id() -> String {
    std::env::var(BUSINESS_AI_APP_ID_ENV)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| BUSINESS_AI_APP_ID.to_owned())
}

/// The channel integration of a contact inbox (`resolveIntegrationContextFromContactInbox`).
enum Integration {
    Whatsapp(Value),
    Messenger(Value),
}

/// `true` when the channel has `updateThreadControl`.
pub fn supports_update(channel: &str) -> bool {
    matches!(channel, "whatsapp" | "messenger")
}

async fn integration_auth(
    db: &Db,
    workspace_id: i64,
    contact_inbox: &ContactInbox,
) -> Result<Integration, ThreadControlError> {
    let channel = contact_inbox.channel.clone();
    let table = match channel.as_str() {
        "whatsapp" => "IntegrationWhatsapp",
        "messenger" => "IntegrationMessenger",
        _ => return Err(ThreadControlError::Unsupported(channel)),
    };
    let inbox_id = contact_inbox.inbox_id;
    let is_whatsapp = table == "IntegrationWhatsapp";
    let auth: Option<Value> = tx::read(db, move |conn| {
        use chatbotx_db::repo::{integration_messenger, integration_whatsapp};
        Ok(if is_whatsapp {
            integration_whatsapp::find_by_inbox_id_for_workspace(conn, workspace_id, inbox_id)?
                .map(|row| row.auth.0)
        } else {
            integration_messenger::find_by_inbox_id(conn, inbox_id)?
                .filter(|row| row.workspace_id == workspace_id)
                .map(|row| row.auth.0)
        })
    })
    .await?;
    let Some(auth) = auth else {
        return Err(ThreadControlError::Business(BusinessError::not_found(
            "Integration not found",
        )));
    };
    Ok(if channel == "whatsapp" {
        Integration::Whatsapp(auth)
    } else {
        Integration::Messenger(auth)
    })
}

fn text<'a>(v: &'a Value, pointer: &str) -> &'a str {
    v.pointer(pointer).and_then(Value::as_str).unwrap_or("")
}

fn version(auth: &Value) -> &str {
    Some(text(auth, "/version"))
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_API_VERSION)
}

fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default()
    })
}

/// Meta codes that a retry cannot fix (auth / permission / payload categories of both
/// error mappers).
///
/// ponytail: only the permanence of a failure is derived (the archive release decides
/// retry vs. log on it); the full `ChannelErrorCategory` mapping lives in the worker.
fn is_permanent_code(code: Option<i64>, kind: Option<&str>) -> bool {
    const TRANSIENT: &[i64] = &[
        1, 2, 4, 17, 32, 613, 80_001, 80_006, 130_429, 131_000, 131_016, 131_048, 131_056, 133_004,
    ];
    const PERMANENT: &[i64] = &[
        0,
        3,
        10,
        102,
        190,
        341,
        368,
        458,
        459,
        460,
        463,
        464,
        467,
        492,
        131_005,
        131_031,
        131_042,
        131_057,
        131_064,
        133_000,
        133_005,
        133_006,
        133_010,
        133_015,
        NOT_ESCALATION_CODE,
    ];
    match code {
        Some(c) if TRANSIENT.contains(&c) => false,
        Some(c) if PERMANENT.contains(&c) || (200..=299).contains(&c) => true,
        _ => kind == Some("OAuthException"),
    }
}

/// One Graph call; a non-2xx answer becomes a [`ChannelFailure`] (`parseOriginError` +
/// `mapApiFields`), a transport error a transient one.
async fn graph(request: reqwest::RequestBuilder, fallback: &str) -> Result<Value, ChannelFailure> {
    let response = request.send().await.map_err(|e| ChannelFailure {
        message: e.to_string(),
        code: None,
        take_refused: false,
        permanent: false,
    })?;
    let ok = response.status().is_success();
    let body: Value = response
        .text()
        .await
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    if ok {
        return Ok(body);
    }
    let error = body.get("error");
    let code = error.and_then(|e| e.get("code")).cloned();
    let kind = error.and_then(|e| e.get("type")).and_then(Value::as_str);
    Err(ChannelFailure {
        message: format_graph_error(error).unwrap_or_else(|| fallback.to_owned()),
        permanent: is_permanent_code(code.as_ref().and_then(Value::as_i64), kind),
        code,
        take_refused: false,
    })
}

fn permanent(message: &str) -> ThreadControlError {
    ThreadControlError::Channel(ChannelFailure {
        message: message.to_owned(),
        code: Some(Value::from("threadControlActionUnsupported")),
        take_refused: false,
        permanent: true,
    })
}

fn clip(metadata: Option<&str>) -> Option<String> {
    metadata
        .filter(|m| !m.is_empty())
        .map(|m| m.chars().take(METADATA_MAX_LENGTH).collect())
}

/// `runOnChannel`: take / release / pass on the contact's channel. `Unsupported` for a
/// channel without `updateThreadControl`.
pub async fn update_on_channel(
    db: &Db,
    base_url: &str,
    workspace_id: i64,
    contact_inbox: &ContactInbox,
    action: &str,
    target_role: Option<&str>,
    metadata: Option<&str>,
) -> Result<ChannelResult, ThreadControlError> {
    if !supports_update(&contact_inbox.channel) {
        return Err(ThreadControlError::Unsupported(
            contact_inbox.channel.clone(),
        ));
    }
    let metadata = if action == "release" { None } else { metadata };
    match integration_auth(db, workspace_id, contact_inbox).await? {
        Integration::Whatsapp(auth) => {
            whatsapp_update(
                base_url,
                &auth,
                contact_inbox,
                action,
                target_role,
                metadata,
            )
            .await
        }
        Integration::Messenger(auth) => {
            messenger_update(
                base_url,
                &auth,
                contact_inbox,
                action,
                target_role,
                metadata,
            )
            .await
        }
    }
}

/// `buildThreadControlBody`.
pub fn whatsapp_body(
    source_id: &str,
    source_user_id: Option<&str>,
    action: &str,
    target_role: Option<&str>,
    metadata: Option<&str>,
) -> Value {
    let mut body = Map::new();
    body.insert("messaging_product".into(), json!("whatsapp"));
    let bsuid = crate::whatsapp::should_address_by_source_user_id(source_id, source_user_id);
    match source_user_id {
        Some(user) if bsuid && !user.is_empty() => {
            body.insert("recipient".into(), json!(user));
        }
        _ => {
            body.insert("to".into(), json!(source_id));
        }
    }
    body.insert("action".into(), json!(action));
    if let Some(note) = clip(metadata) {
        body.insert("metadata".into(), json!(note));
    }
    if action == "pass"
        && let Some(role) = target_role.filter(|r| *r != "escalation")
    {
        body.insert("control_pass".into(), json!({ "target_role": role }));
    }
    Value::Object(body)
}

async fn whatsapp_update(
    base_url: &str,
    auth: &Value,
    contact_inbox: &ContactInbox,
    action: &str,
    target_role: Option<&str>,
    metadata: Option<&str>,
) -> Result<ChannelResult, ThreadControlError> {
    let url = format!(
        "{base_url}/{}/{}/thread_control",
        version(auth),
        text(auth, "/metadata/phoneNumber/id")
    );
    let body = whatsapp_body(
        &contact_inbox.source_id,
        contact_inbox.source_user_id.as_deref(),
        action,
        target_role,
        metadata,
    );
    let request = http()
        .post(url)
        .bearer_auth(text(auth, "/tokens/accessToken"))
        .json(&body);
    match graph(request, "WhatsApp API call failed").await {
        Ok(answer) => {
            tracing::info!(
                action,
                target_role,
                has_metadata = metadata.is_some(),
                request_id = answer.get("request_id").and_then(Value::as_str),
                "Whatsapp thread_control applied"
            );
        }
        Err(mut failure) => {
            tracing::warn!(action, code = ?failure.code, "Whatsapp thread_control refused");
            failure.take_refused = action == "take"
                && failure.code.as_ref().and_then(Value::as_i64) == Some(NOT_ESCALATION_CODE);
            return Err(ThreadControlError::Channel(failure));
        }
    }
    // `OWNER_ROLE_AFTER_ACTION`.
    let owner_role = match action {
        "take" => Some("escalation"),
        "pass" => Some(roles::parse(target_role).unwrap_or("escalation")),
        _ => None,
    };
    Ok(ChannelResult {
        owner_role,
        owner_app_id: None,
    })
}

/// `resolveOwnAppId(auth)`: the OAuth client id, `None` when blank.
fn own_app_id(auth: &Value) -> Option<String> {
    Some(text(auth, "/clientId").trim().to_owned()).filter(|v| !v.is_empty())
}

fn messenger_psid(contact_inbox: &ContactInbox) -> Result<&str, ThreadControlError> {
    Some(contact_inbox.source_id.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ThreadControlError::Channel(ChannelFailure {
                message: "Missing recipient ID in conversation".into(),
                code: None,
                take_refused: false,
                permanent: true,
            })
        })
}

async fn messenger_update(
    base_url: &str,
    auth: &Value,
    contact_inbox: &ContactInbox,
    action: &str,
    target_role: Option<&str>,
    metadata: Option<&str>,
) -> Result<ChannelResult, ThreadControlError> {
    let psid = messenger_psid(contact_inbox)?;
    if action == "release" {
        return Err(permanent(
            "Messenger conversations cannot be released; take over or return to the bot instead",
        ));
    }
    if action == "pass" && target_role.is_some_and(|r| r != "ai_agent") {
        return Err(permanent(
            "Messenger can only pass a conversation back to the AI agent",
        ));
    }
    let return_target = (action == "pass").then(business_ai_app_id);
    let (edge, mut body) = match &return_target {
        Some(target) => (
            "pass_thread_control",
            json!({ "recipient": { "id": psid }, "target_app_id": target }),
        ),
        None => (
            "take_thread_control",
            json!({ "recipient": { "id": psid } }),
        ),
    };
    if let Some(note) = clip(metadata) {
        body["metadata"] = json!(note);
    }
    let url = format!("{base_url}/{}/me/{edge}", version(auth));
    let request = http()
        .post(url)
        .bearer_auth(text(auth, "/tokens/accessToken"))
        .json(&body);
    let outcome = match graph(request, "Messenger API call failed").await {
        Ok(answer) if answer.get("success") == Some(&Value::Bool(false)) => Err(ChannelFailure {
            message: "Messenger rejected the thread control request".into(),
            code: None,
            take_refused: false,
            permanent: true,
        }),
        Ok(_) => Ok(()),
        Err(failure) => Err(failure),
    };
    if let Err(mut failure) = outcome {
        // `toThreadControlChannelError`: a permission refusal of a take.
        let code = failure.code.as_ref().and_then(Value::as_i64);
        let permission = failure.message == "Messenger rejected the thread control request"
            || code.is_some_and(|c| matches!(c, 3 | 10 | 341 | 368 | 200..=299));
        if action == "take" && permission {
            failure.take_refused = true;
            failure.message = "Messenger did not let this app take the conversation. Make sure this app is the Page's primary receiver in the Page's Handover Protocol settings.".into();
        }
        return Err(ThreadControlError::Channel(failure));
    }
    Ok(ChannelResult {
        owner_role: (action == "pass").then_some("ai_agent"),
        owner_app_id: if action == "take" {
            own_app_id(auth)
        } else {
            return_target
        },
    })
}

/// `parseExpiration`: epoch seconds or a date text (unix ms).
fn parse_expiration(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(n) => n.as_f64().map(|s| (s * 1000.0) as i64),
        Value::String(s) if s.is_empty() => None,
        Value::String(s) if s.bytes().all(|b| b.is_ascii_digit()) => {
            s.parse::<i64>().ok().map(|s| s * 1000)
        }
        Value::String(s) => chatbotx_db::repo::parse_iso(s),
        _ => None,
    }
}

/// `getChannelThreadOwner`: `None` when the channel has no `getThreadOwner` (every
/// channel but Messenger): "cannot sync", never "no owner".
pub async fn get_thread_owner(
    db: &Db,
    base_url: &str,
    workspace_id: i64,
    contact_inbox: &ContactInbox,
) -> Result<Option<ThreadOwner>, ThreadControlError> {
    if contact_inbox.channel != "messenger" {
        return Ok(None);
    }
    let Integration::Messenger(auth) = integration_auth(db, workspace_id, contact_inbox).await?
    else {
        return Ok(None);
    };
    let psid = messenger_psid(contact_inbox)?;
    let url = format!("{base_url}/{}/me/thread_owner", version(&auth));
    let request = http()
        .get(url)
        .bearer_auth(text(&auth, "/tokens/accessToken"))
        .query(&[("recipient", psid)]);
    let answer = graph(request, "Messenger API call failed")
        .await
        .map_err(ThreadControlError::Channel)?;
    let Some(data) = answer.get("data").and_then(Value::as_array) else {
        return Err(ThreadControlError::Channel(ChannelFailure {
            message: "Malformed thread_owner response".into(),
            code: None,
            take_refused: false,
            permanent: false,
        }));
    };
    let owner = data.first().and_then(|d| d.get("thread_owner"));
    let app_id = match owner.and_then(|o| o.get("app_id")) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    let expires_at = app_id
        .as_ref()
        .and_then(|_| parse_expiration(owner.and_then(|o| o.get("expiration"))));
    Ok(Some(ThreadOwner {
        owner_app_id: app_id,
        expires_at,
        own_app_id: own_app_id(&auth),
        ai_agent_app_id: Some(business_ai_app_id()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `integrations/whatsapp/__tests__/thread-control.test.ts` body cases.
    #[test]
    fn whatsapp_body_matches_build_thread_control_body() {
        assert_eq!(
            whatsapp_body("15550001", None, "pass", Some("marketing"), Some("note")),
            json!({ "messaging_product": "whatsapp", "to": "15550001", "action": "pass",
                    "metadata": "note", "control_pass": { "target_role": "marketing" } })
        );
        assert_eq!(
            whatsapp_body("15550001", None, "pass", Some("escalation"), None),
            json!({ "messaging_product": "whatsapp", "to": "15550001", "action": "pass" })
        );
        assert_eq!(
            whatsapp_body(
                "15550001",
                None,
                "take",
                Some("marketing"),
                Some(&"n".repeat(2100))
            ),
            json!({ "messaging_product": "whatsapp", "to": "15550001", "action": "take",
                    "metadata": "n".repeat(2000) })
        );
        assert_eq!(
            whatsapp_body("", Some("US.123"), "release", None, None),
            json!({ "messaging_product": "whatsapp", "recipient": "US.123", "action": "release" })
        );
    }

    #[test]
    fn expiration_reads_seconds_and_dates() {
        assert_eq!(
            parse_expiration(Some(&json!(1_700_000_000))),
            Some(1_700_000_000_000)
        );
        assert_eq!(
            parse_expiration(Some(&json!("1700000000"))),
            Some(1_700_000_000_000)
        );
        assert_eq!(parse_expiration(Some(&json!(""))), None);
        assert_eq!(parse_expiration(None), None);
    }

    #[test]
    fn permanence_follows_the_categories() {
        assert!(is_permanent_code(Some(NOT_ESCALATION_CODE), None));
        assert!(is_permanent_code(Some(100), Some("OAuthException")));
        assert!(!is_permanent_code(Some(4), Some("OAuthException")));
        assert!(!is_permanent_code(None, None));
    }
}
