//! WhatsApp channel connect and reconnect (Q.P7-V4):
//! `integrationWhatsappAPIs.connectWhatsappNumberAPI` (`api/connect.ts` +
//! `actions/connect-number.ts`, the signup-session path only: the picker's one
//! request per number) and `reconnectWhatsappAction` /
//! `persistReconnectAuthAndResubscribe` (`actions/reconnect.action.ts`), with
//! the Graph calls of `integrations/whatsapp/src/api/{auth,waba-owner,
//! waba-targets,granted-scopes,phone-number,waba,webhook,waba-setup}.ts`.
//!
//! Every Graph call goes to `ApiState.graph_url` (tests point it at a local
//! server). The tenant custom-domain origin is cloud (E16): the provider
//! origin is always the broker (`resolveProviderOriginForCredential`).

use chatbotx_business::audit::AuditRecordInput;
use chatbotx_business::platform_credential::{self, DecryptedCredential};
use chatbotx_business::{BusinessError, tenant, tx};
use chatbotx_db::repo::models::{IntegrationWhatsapp, WhatsappSignupSession};
use chatbotx_db::repo::{
    self, Json, integration_whatsapp, whatsapp_business_account as waba_repo,
    whatsapp_signup_session as signup,
};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use chatbotx_kv::LockOpts;
use reqwest::Method;
use serde_json::{Value, json};

use super::ApiState;
use super::messenger_templates::{Fail, broker_origin, graph_call};
use super::whatsapp_channels::{self, DEFAULT_API_VERSION, WhatsappAuth};
use crate::rpc::channel_connect::{failure, outcome};

/// `WHATSAPP_OAUTH_CALLBACK_PATH`.
pub const OAUTH_CALLBACK_PATH: &str = "/integrations/whatsapp/callback";
/// `WHATSAPP_CAPI_SCOPE`.
pub const CAPI_SCOPE: &str = "whatsapp_business_manage_events";
const MANAGEMENT_SCOPE: &str = "whatsapp_business_management";
/// `WABA_NODE_TYPE`.
const WABA_NODE_TYPE: &str = "whatsapp_business_account";
/// `WHATSAPP_ACCOUNT_MISUSE_SUBCODE` / `_USER_TITLE`.
const MISUSE_SUBCODE: i64 = 2_388_339;
const MISUSE_USER_TITLE: &str = "Invalid WhatsApp account usage";
/// `WHATSAPP_BASE_SUBSCRIBED_FIELDS` + `automatic_events`, then the routing fields.
const BASE_FIELDS: &[&str] = &[
    "messages",
    "history",
    "smb_app_state_sync",
    "smb_message_echoes",
    "account_update",
];
const ROUTING_FIELDS: &[&str] = &["messaging_handovers", "standby"];
/// `WABA_PROVISION_LOCK_TIMEOUT_SECONDS`.
const WABA_PROVISION_LOCK_SECS: u64 = 60;
const MAX_PAGES: usize = 50;

// ---- Graph -----------------------------------------------------------------

/// A failed Graph call: the formatted message and Meta's `error` object.
#[derive(Debug, Clone)]
pub struct Graph {
    pub message: String,
    pub error: Value,
}

impl std::fmt::Display for Graph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

async fn call(
    method: Method,
    bearer: Option<&str>,
    url: &str,
    query: &[(&str, &str)],
    body: Option<&Value>,
) -> Result<Value, Graph> {
    graph_call(method, bearer, url, query, body)
        .await
        .map_err(|failure| {
            let (fail, error) = *failure;
            let message = match fail {
                Fail::Sdk(message, _) => message,
                Fail::Business(error) => error.to_string(),
            };
            Graph { message, error }
        })
}

/// The WhatsApp app settings of a platform credential (`WhatsappCredential`).
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub client_id: String,
    pub client_secret: String,
    pub version: String,
    pub verify_token: String,
    pub system_user_id: String,
    pub system_user_token: String,
    pub business_id: String,
    pub business_name: String,
}

impl Settings {
    pub fn of(credential: &DecryptedCredential) -> Settings {
        let text = |key: &str| credential.text(key).unwrap_or_default().to_owned();
        Settings {
            client_id: text("clientId"),
            client_secret: text("clientSecret"),
            version: Some(text("version"))
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| DEFAULT_API_VERSION.to_owned()),
            verify_token: text("verifyToken"),
            system_user_id: text("systemUserId"),
            system_user_token: text("systemUserToken"),
            business_id: text("businessId"),
            business_name: text("businessName"),
        }
    }

    /// `appAccessToken`.
    pub fn app_token(&self) -> String {
        format!("{}|{}", self.client_id, self.client_secret)
    }
}

/// `exchangeAccessToken` with the callback `redirect_uri`.
pub async fn exchange_access_token(
    base: &str,
    settings: &Settings,
    code: &str,
    redirect_uri: &str,
) -> Result<String, Graph> {
    let answer = call(
        Method::GET,
        None,
        &format!("{base}/{}/oauth/access_token", settings.version),
        &[
            ("client_id", &settings.client_id),
            ("client_secret", &settings.client_secret),
            ("code", code),
            ("redirect_uri", redirect_uri),
        ],
        None,
    )
    .await?;
    Ok(answer["access_token"]
        .as_str()
        .unwrap_or_default()
        .to_owned())
}

/// `debugTokenOrThrow`: the token's data, `None` when it is not valid.
pub async fn debug_token(base: &str, token: &str, app_token: &str) -> Result<Option<Value>, Graph> {
    let answer = call(
        Method::GET,
        None,
        &format!("{base}/debug_token"),
        &[("input_token", token), ("access_token", app_token)],
        None,
    )
    .await?;
    let data = answer["data"].clone();
    Ok((data["is_valid"] == true).then_some(data))
}

/// `grantedScopesForWaba`: the CAPI scope counts on presence alone (its
/// targets name the Business Presence account), a scope without targets
/// counts for every WABA.
pub fn granted_scopes_for_waba(granular: &Value, waba_id: &str) -> Vec<String> {
    granular
        .as_array()
        .into_iter()
        .flatten()
        .filter(|scope| {
            let targets = scope["target_ids"].as_array();
            scope["scope"] == CAPI_SCOPE
                || targets.is_none_or(|t| t.is_empty())
                || targets.is_some_and(|t| t.iter().any(|id| id == waba_id))
        })
        .filter_map(|scope| scope["scope"].as_str().map(str::to_owned))
        .collect()
}

/// `getWhatsappGrantedScopes`.
pub async fn granted_scopes(
    base: &str,
    token: &str,
    app_token: &str,
    waba_id: &str,
) -> Result<Vec<String>, Graph> {
    let data = debug_token(base, token, app_token).await?;
    Ok(granted_scopes_for_waba(
        &data.unwrap_or_default()["granular_scopes"],
        waba_id,
    ))
}

/// `listPhoneNumbers` (every page).
pub async fn list_phone_numbers(
    base: &str,
    version: &str,
    waba_id: &str,
    token: &str,
) -> Result<Vec<Value>, Graph> {
    let url = format!("{base}/{version}/{waba_id}/phone_numbers");
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let query: Vec<(&str, &str)> = after.iter().map(|a| ("after", a.as_str())).collect();
        let page = call(Method::GET, Some(token), &url, &query, None).await?;
        out.extend(page["data"].as_array().cloned().unwrap_or_default());
        after = match (
            page.pointer("/paging/next"),
            page.pointer("/paging/cursors/after")
                .and_then(Value::as_str),
        ) {
            (Some(next), Some(cursor)) if !next.is_null() => Some(cursor.to_owned()),
            _ => break,
        };
    }
    Ok(out)
}

/// `findWaba` with `fields`.
pub async fn find_waba(
    base: &str,
    version: &str,
    waba_id: &str,
    token: &str,
    fields: &str,
) -> Result<Value, Graph> {
    call(
        Method::GET,
        Some(token),
        &format!("{base}/{version}/{waba_id}"),
        &[("fields", fields)],
        None,
    )
    .await
}

/// `subscribeWebhook` (no override callback: `WHATSAPP_OVERRIDE_CALLBACK_URI`
/// is not read).
pub async fn subscribe_webhook(
    base: &str,
    version: &str,
    waba_id: &str,
    token: &str,
    include_automatic_events: bool,
) -> Result<(), Graph> {
    let mut fields: Vec<&str> = BASE_FIELDS.to_vec();
    if include_automatic_events {
        fields.push("automatic_events");
    }
    fields.extend(ROUTING_FIELDS);
    let answer = call(
        Method::POST,
        Some(token),
        &format!("{base}/{version}/{waba_id}/subscribed_apps"),
        &[],
        Some(&json!({ "subscribed_fields": fields })),
    )
    .await?;
    if answer["success"] != true {
        return Err(Graph {
            message: "Failed to subscribe webhook".into(),
            error: Value::Null,
        });
    }
    Ok(())
}

/// One granted target (`TargetCandidate`).
#[derive(Debug, Clone)]
struct Target {
    id: String,
    is_waba: bool,
    score: usize,
}

/// `classifyTarget`: `metadata.type`, else the owner business, decides.
async fn classify(base: &str, version: &str, token: &str, id: &str) -> Target {
    let probe = call(
        Method::GET,
        Some(token),
        &format!("{base}/{version}/{id}"),
        &[("metadata", "1"), ("fields", "id,owner_business_info")],
        None,
    )
    .await;
    let is_waba = match probe {
        Ok(node) => match node.pointer("/metadata/type").and_then(Value::as_str) {
            Some(kind) => kind == WABA_NODE_TYPE,
            None => node
                .pointer("/owner_business_info/id")
                .and_then(Value::as_str)
                .is_some_and(|b| !b.is_empty()),
        },
        Err(error) => {
            tracing::warn!(%error, target_id = id, "WhatsApp granted target could not be classified, treating it as not a WABA");
            false
        }
    };
    Target {
        id: id.to_owned(),
        is_waba,
        score: 0,
    }
}

/// `probeCapability`: `Some(true)` usable, `Some(false)` the WhatsApp-account
/// refusal, `None` undecided.
async fn probe(base: &str, version: &str, settings: &Settings, id: &str) -> Option<bool> {
    let result = call(
        Method::POST,
        Some(&settings.system_user_token),
        &format!("{base}/{version}/{id}/assigned_users"),
        &[("user", &settings.system_user_id), ("tasks", "MANAGE")],
        None,
    )
    .await;
    match result {
        Ok(_) => Some(true),
        Err(error) => {
            let sub = error.error["error_subcode"].as_i64().or_else(|| {
                error.error["error_subcode"]
                    .as_str()
                    .and_then(|s| s.parse().ok())
            });
            if sub == Some(MISUSE_SUBCODE) || error.error["error_user_title"] == MISUSE_USER_TITLE {
                tracing::warn!(
                    target_id = id,
                    "WhatsApp granted target rejects WABA-level API use, excluding it"
                );
                Some(false)
            } else {
                tracing::warn!(%error, target_id = id, "WhatsApp capability probe failed for an unrelated reason, keeping the target");
                None
            }
        }
    }
}

/// `resolveOwningWabaId`: the WABA owning the numbers behind the token.
pub async fn resolve_owning_waba_id(
    base: &str,
    token: &str,
    settings: &Settings,
    phone_number_ids: &[String],
) -> Result<Option<String>, Graph> {
    let version = settings.version.as_str();
    let data = debug_token(base, token, &settings.app_token())
        .await
        .unwrap_or_else(|error| {
            tracing::error!(%error, "Failed to debug token");
            None
        })
        .unwrap_or_default();
    let target_ids: Vec<String> = data["granular_scopes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|s| s["scope"] == MANAGEMENT_SCOPE)
        .and_then(|s| s["target_ids"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|id| id.as_str().map(str::to_owned))
        .collect();
    if target_ids.len() <= 1 {
        return Ok(target_ids.into_iter().next());
    }
    let mut targets = Vec::new();
    for id in &target_ids {
        targets.push(classify(base, version, token, id).await);
    }
    let wabas: Vec<&Target> = targets.iter().filter(|t| t.is_waba).collect();
    match wabas.len() {
        0 => return Ok(unresolved("noTargetIsAWaba")),
        1 => return Ok(Some(wabas[0].id.clone())),
        _ => {}
    }
    for target in targets.iter_mut().filter(|t| t.is_waba) {
        let owned: Vec<String> = match list_phone_numbers(base, version, &target.id, token).await {
            Ok(rows) => rows
                .iter()
                .filter_map(|r| r["id"].as_str().map(str::to_owned))
                .collect(),
            Err(error) => {
                tracing::warn!(%error, waba_id = target.id, "WhatsApp granted target did not list phone numbers, skipping it");
                Vec::new()
            }
        };
        target.score = if phone_number_ids.is_empty() {
            usize::from(!owned.is_empty())
        } else {
            phone_number_ids
                .iter()
                .filter(|id| owned.contains(id))
                .count()
        };
    }
    let owners: Vec<&Target> = targets
        .iter()
        .filter(|t| t.is_waba && t.score > 0)
        .collect();
    match owners.len() {
        0 => return Ok(unresolved("noTargetOwnsTheNumbers")),
        1 => return Ok(Some(owners[0].id.clone())),
        _ => {}
    }
    // `pickUsableWaba`: best score first, Meta order within; the first target
    // not positively rejected wins.
    let can_probe = !settings.system_user_token.is_empty() && !settings.system_user_id.is_empty();
    let mut scores: Vec<usize> = owners.iter().map(|t| t.score).collect();
    scores.sort_unstable_by(|a, b| b.cmp(a));
    scores.dedup();
    for score in scores {
        for target in owners.iter().filter(|t| t.score == score) {
            let capability = if can_probe {
                probe(base, version, settings, &target.id).await
            } else {
                None
            };
            if capability != Some(false) {
                return Ok(Some(target.id.clone()));
            }
        }
    }
    Ok(unresolved("everyOwningTargetRejected"))
}

fn unresolved(reason: &str) -> Option<String> {
    tracing::warn!(
        reason,
        "No granted target resolved to an owning WhatsApp Business Account"
    );
    None
}

/// `addSystemUser` + `shareCreditLine` (when the app has a business).
async fn setup_oauth_resources(
    base: &str,
    version: &str,
    waba_id: &str,
    settings: &Settings,
) -> Result<(), Graph> {
    call(
        Method::POST,
        Some(&settings.system_user_token),
        &format!("{base}/{version}/{waba_id}/assigned_users"),
        &[("user", &settings.system_user_id), ("tasks", "MANAGE")],
        None,
    )
    .await?;
    if settings.business_id.is_empty() {
        return Ok(());
    }
    let lines = call(
        Method::GET,
        Some(&settings.system_user_token),
        &format!(
            "{base}/{DEFAULT_API_VERSION}/{}/extendedcredits",
            settings.business_id
        ),
        &[("fields", "id,legal_entity_name")],
        None,
    )
    .await?;
    let line = lines["data"].as_array().and_then(|rows| {
        rows.iter().find(|line| {
            line["legal_entity_name"]
                .as_str()
                .is_some_and(|name| name.contains(&settings.business_name))
        })
    });
    let Some(line_id) = line.and_then(|l| l["id"].as_str()) else {
        return Err(Graph {
            message: "You need to set up a line of credit".into(),
            error: Value::Null,
        });
    };
    if let Err(error) = call(
        Method::POST,
        Some(&settings.system_user_token),
        &format!("{base}/{version}/{line_id}/whatsapp_credit_sharing_and_attach"),
        &[("waba_id", waba_id), ("waba_currency", "USD")],
        None,
    )
    .await
    {
        tracing::info!(%error, "Failed to share credit line");
    }
    Ok(())
}

// ---- auth value ------------------------------------------------------------

/// `buildAuthValue` (OAuth, not manual): the persisted `WhatsappAuthValue`.
pub fn auth_value(
    settings: &Settings,
    origin: &str,
    token: &str,
    waba_id: &str,
    business_id: &str,
    phone_number: &Value,
) -> Value {
    let origin = origin.trim_end_matches('/');
    json!({
        "clientId": settings.client_id,
        "clientSecret": settings.client_secret,
        "verifyToken": settings.verify_token,
        "redirectUrl": format!("{origin}{OAUTH_CALLBACK_PATH}"),
        "authType": "oauth2",
        "tokens": { "accessToken": token },
        "metadata": {
            "wabaId": waba_id,
            "phoneNumber": phone_number,
            "businessId": business_id,
            "webhookUrl": format!("{origin}/integrations/whatsapp/webhook"),
        },
    })
}

/// The WhatsApp credential of the owner (`platformCredentialService.resolveForOwner`).
async fn settings_for_owner(
    api: &ApiState,
    owner_id: i64,
) -> Result<Option<Settings>, BusinessError> {
    let Some(encryption) = api.encryption.clone() else {
        return Ok(None);
    };
    tx::read(&api.db, move |conn| {
        platform_credential::resolve_for_owner(conn, &encryption, owner_id, "whatsapp", false)
    })
    .await
    .map(|c| c.as_ref().map(Settings::of))
}

/// `whatsappBusinessAccountService.upsertCurrentCredential` (best effort):
/// the encrypted WABA credential at the row's current revision.
async fn upsert_waba_credential(
    api: &ApiState,
    workspace_id: i64,
    waba_id: &str,
    business_id: &str,
    token: &str,
    version: &str,
    scopes: Vec<String>,
) -> Result<Option<chatbotx_db::repo::models::WhatsappBusinessAccount>, String> {
    let encryption = api
        .encryption
        .clone()
        .ok_or("encryption keys are not set")?;
    let aad = format!("whatsapp-waba:{workspace_id}:{waba_id}");
    let sealed = encryption
        .encrypt_object(
            &json!({ "accessToken": token, "apiVersion": version }),
            Some(&aad),
        )
        .map_err(|e| e.to_string())?;
    let sealed = serde_json::to_value(sealed).map_err(|e| e.to_string())?;
    let (waba_id, business_id) = (waba_id.to_owned(), business_id.to_owned());
    tx::write(&api.db, move |conn| {
        let revision =
            waba_repo::find_by_waba(conn, workspace_id, &waba_id)?.map_or(0, |r| r.revision);
        Ok(waba_repo::upsert_credential(
            conn,
            &waba_repo::UpsertCredentialInput {
                workspace_id,
                waba_id,
                business_id,
                credential: Json(sealed),
                granted_scopes: scopes,
                scope_checked_at: repo::now_ms(),
                expected_revision: revision,
            },
        )?)
    })
    .await
    .map_err(|e| e.to_string())
}

fn exception(message: &str) -> BusinessError {
    BusinessError::exception(message, "systemError", 400)
}

fn system(message: String) -> BusinessError {
    BusinessError::exception(message, "systemError", 500)
}

// ---- reconnect -------------------------------------------------------------

/// `t("whatsapp.reconnect.errors.*")` / `whatsapp.connect.errors.*` (en).
const NOT_FOUND: &str = "WhatsApp integration not found.";
const APP_SETTINGS_NOT_FOUND: &str = "WhatsApp app settings were not found.";
const WABA_RESOLVE_FAILED: &str =
    "Could not resolve the WhatsApp Business Account from the authorization.";
const WABA_MISMATCH: &str =
    "Reconnect returned a different WhatsApp Business Account. Please reconnect the same account.";
const PHONE_NUMBER_MISMATCH: &str = "Reconnect returned the expected WhatsApp Business Account, but not the phone number connected to this workspace.";
/// `errors.superAdminRequired` (en).
pub const SUPER_ADMIN_REQUIRED: &str = "You need to be a workspace super admin to do this.";

/// `reconnectWhatsapp` (the caller ran `assertWorkspaceSuperAdmin`):
/// `{ ok, hasCapiScope, scope, resubscribed }`.
pub async fn reconnect(
    api: &ApiState,
    workspace_id: i64,
    integration_id: i64,
    code: &str,
) -> Result<Value, BusinessError> {
    let existing = tx::read(&api.db, move |conn| {
        Ok(integration_whatsapp::find_by_id_for_workspace(
            conn,
            integration_id,
            workspace_id,
        )?)
    })
    .await?
    .ok_or_else(|| exception(NOT_FOUND))?;
    let owner: i64 = tx::read(&api.db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT "ownerId" FROM "Workspace" WHERE "id" = ?1"#,
                [workspace_id],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()
            .map_err(repo::Error::from)?
            .flatten())
    })
    .await?
    .unwrap_or_default();
    let settings = settings_for_owner(api, owner)
        .await?
        .ok_or_else(|| exception(APP_SETTINGS_NOT_FOUND))?;
    let base = api.graph_url.as_str();
    let origin = broker_origin(&api.app_url);
    let graph = |e: Graph| exception(&e.message);

    // `exchangeCodeForOwnedWabaToken`.
    let token = exchange_access_token(
        base,
        &settings,
        code,
        &format!("{origin}{OAUTH_CALLBACK_PATH}"),
    )
    .await
    .map_err(graph)?;
    let waba_id =
        resolve_owning_waba_id(base, &token, &settings, &[existing.phone_number_id.clone()])
            .await
            .map_err(graph)?
            .ok_or_else(|| exception(WABA_RESOLVE_FAILED))?;
    if waba_id != existing.waba_id {
        return Err(exception(WABA_MISMATCH));
    }
    let version = settings.version.clone();
    let (waba, numbers) = tokio::join!(
        find_waba(base, &version, &waba_id, &token, "owner_business_info"),
        list_phone_numbers(base, &version, &waba_id, &token),
    );
    let (waba, numbers) = (waba.map_err(graph)?, numbers.map_err(graph)?);
    let phone_number = numbers
        .into_iter()
        .find(|n| n["id"] == existing.phone_number_id.as_str())
        .ok_or_else(|| exception(PHONE_NUMBER_MISMATCH))?;
    let business_id = waba
        .pointer("/owner_business_info/id")
        .and_then(Value::as_str)
        .map_or_else(|| existing.business_id.clone(), str::to_owned);
    let auth = auth_value(
        &settings,
        &origin,
        &token,
        &waba_id,
        &business_id,
        &phone_number,
    );
    let scopes = granted_scopes(base, &token, &settings.app_token(), &waba_id)
        .await
        .map_err(graph)?;
    let resubscribed = persist_reconnect_auth_and_resubscribe(
        api,
        &existing,
        auth,
        scopes.clone(),
        &business_id,
        &token,
        &version,
    )
    .await?;
    Ok(json!({
        "ok": true,
        "hasCapiScope": scopes.iter().any(|s| s == CAPI_SCOPE),
        "scope": CAPI_SCOPE,
        "resubscribed": resubscribed,
    }))
}

/// `persistReconnectAuthAndResubscribe`: the auth write and the inbox's
/// `connect.completed` share one transaction (`commitReconnect`); then the
/// WABA credential (best effort) and the `automatic_events` resubscribe.
pub async fn persist_reconnect_auth_and_resubscribe(
    api: &ApiState,
    existing: &IntegrationWhatsapp,
    auth: Value,
    scopes: Vec<String>,
    business_id: &str,
    token: &str,
    version: &str,
) -> Result<bool, BusinessError> {
    let (id, workspace_id, inbox_id) = (existing.id, existing.workspace_id, existing.inbox_id);
    let has_capi = scopes.iter().any(|s| s == CAPI_SCOPE);
    let saved = Json(auth.clone());
    tx::write(&api.db, move |conn| {
        commit_reconnect(conn, id, workspace_id, inbox_id, &saved, has_capi)
    })
    .await?;
    let waba_id = existing.waba_id.clone();
    if let Err(error) = upsert_waba_credential(
        api,
        workspace_id,
        &waba_id,
        business_id,
        token,
        version,
        scopes,
    )
    .await
    {
        tracing::warn!(%error, workspace_id, waba_id, "Unable to persist WhatsApp Business Account credential after reconnect");
    }
    match subscribe_webhook(&api.graph_url, version, &waba_id, token, true).await {
        Ok(()) => Ok(true),
        Err(error) => {
            tracing::warn!(%error, integration_id = id, workspace_id, "Unable to resubscribe WhatsApp webhook after reconnect");
            Ok(false)
        }
    }
}

/// `commitReconnect`: `replaceAuth` + `reconnectInbox` (the inbox's
/// `Connection`, when it has one, is `connected` again).
fn commit_reconnect(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    inbox_id: i64,
    auth: &Json,
    has_capi_scope: bool,
) -> chatbotx_business::Result<()> {
    let now = repo::now_ms();
    integration_whatsapp::replace_auth(conn, id, workspace_id, auth, has_capi_scope, now)?;
    let connection: Option<(i64, i64)> = conn
        .query_row(
            r#"SELECT "id", "workspaceId" FROM "Connection" WHERE "inboxId" = ?1"#,
            [inbox_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(repo::Error::from)?;
    let Some((connection_id, connection_workspace)) = connection else {
        return Ok(());
    };
    if connection_workspace != workspace_id {
        return Err(BusinessError::not_found("Connection not found"));
    }
    conn.execute(
        r#"UPDATE "Connection" SET "status" = 'connected', "statusReason" = NULL, "lastError" = NULL,
               "authExpiresAt" = NULL, "connectedAt" = ?2, "disconnectedAt" = NULL, "updatedAt" = ?2
           WHERE "id" = ?1"#,
        params![connection_id, now],
    )
    .map_err(repo::Error::from)?;
    conn.execute(
        r#"UPDATE "Inbox" SET "status" = 'connected', "disconnectedAt" = NULL,
               "disconnectReason" = NULL, "updatedAt" = ?3
           WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        params![inbox_id, workspace_id, now],
    )
    .map_err(repo::Error::from)?;
    Ok(())
}

// ---- connect from a signup session ----------------------------------------

/// The picker's connect body (`connectWhatsappViaSessionRequest`).
pub struct SessionConnect {
    pub signup_session_id: String,
    pub phone_number_id: String,
    pub connect_existing: bool,
}

fn not_selectable(id: &str, detail: Option<&str>) -> Value {
    let mut fields = json!({ "status": "failed", "reason": "notSelectable" });
    if let Some(detail) = detail {
        fields["detail"] = json!(detail);
    }
    outcome(id, id, fields)
}

fn duplicated(id: &str) -> Value {
    outcome(
        id,
        id,
        json!({ "status": "duplicated", "reason": "alreadyConnected" }),
    )
}

fn signup_expired() -> BusinessError {
    BusinessError::exception(
        "Your WhatsApp signup session has expired. Please start the connection again.",
        "signupSessionExpired",
        410,
    )
}

/// `connectWhatsappNumber` on the session path: every failure is a typed
/// result. `isCoexistOnboardingIntent` = `connectExisting` (never manual).
pub async fn connect_number(api: &ApiState, user_id: i64, input: &SessionConnect) -> Value {
    let id = input.phone_number_id.as_str();
    match connect_number_inner(api, user_id, input).await {
        Ok(value) => value,
        Err(error) => failure(&error, id, id),
    }
}

/// `findActiveSignupSessionForUser` + its decrypted access token.
async fn active_session(
    api: &ApiState,
    session_id: i64,
    user_id: i64,
) -> Result<Option<(WhatsappSignupSession, String)>, BusinessError> {
    let found = tx::read(&api.db, move |conn| {
        Ok(signup::find_active_signup_session_for_user(
            conn,
            session_id,
            user_id,
            repo::now_ms(),
        )?)
    })
    .await?;
    let Some(session) = found else {
        return Ok(None);
    };
    let encryption = api.encryption.as_deref().ok_or_else(|| {
        BusinessError::exception(
            "Encryption keys are not configured.",
            "credentialMissing",
            500,
        )
    })?;
    let sealed = serde_json::from_value(session.encrypted_access_token.0.clone())
        .map_err(|e| system(e.to_string()))?;
    let token = encryption
        .decrypt_text(&sealed, None)
        .map_err(|e| system(e.to_string()))?;
    Ok(Some((session, token)))
}

async fn connect_number_inner(
    api: &ApiState,
    user_id: i64,
    input: &SessionConnect,
) -> Result<Value, BusinessError> {
    let phone_id = input.phone_number_id.as_str();
    let session_id: i64 = input
        .signup_session_id
        .parse()
        .map_err(|_| signup_expired())?;
    // `resolveConnectOwner`: the user's own active session whose owner is the
    // owner this user resolves to; its workspace (when bound) is checked.
    let (session, token) = active_session(api, session_id, user_id)
        .await?
        .ok_or_else(signup_expired)?;
    let expected =
        tenant::resolve_platform_owner_id(&api.db, api.edition, "", user_id, session.workspace_id)
            .await?;
    if session.owner_id != expected {
        return Err(signup_expired());
    }
    if let Some(workspace_id) = session.workspace_id {
        assert_member(api, workspace_id, user_id).await?;
    }
    let settings = settings_for_owner(api, session.owner_id)
        .await?
        .ok_or_else(|| {
            BusinessError::exception(
                "WhatsApp app credentials are not configured for this workspace.",
                "credentialMissing",
                400,
            )
        })?;
    let origin = broker_origin(&api.app_url);
    let base = api.graph_url.as_str();

    // `prepareSessionConnectInput` + `partitionDirectPhoneNumber`.
    let candidates: Vec<String> =
        serde_json::from_value(session.candidate_phone_number_ids.0.clone()).unwrap_or_default();
    if !candidates.iter().any(|c| c == phone_id) {
        return Ok(not_selectable(phone_id, None));
    }
    let wanted = vec![phone_id.to_owned()];
    let connected = tx::read(&api.db, move |conn| {
        Ok(integration_whatsapp::find_connected_phone_number_ids(
            conn, &wanted,
        )?)
    })
    .await?;
    if connected.iter().any(|c| c == phone_id) {
        return Ok(duplicated(phone_id));
    }
    let numbers = list_phone_numbers(base, &session.api_version, &session.waba_id, &token)
        .await
        .map_err(|e| exception(&e.message))?;
    let Some(phone_number) = numbers.into_iter().find(|n| n["id"] == phone_id) else {
        return Ok(not_selectable(phone_id, None));
    };

    let lock_key = format!(
        "whatsapp:waba-provision:{}:{}",
        session.workspace_id.unwrap_or(session.owner_id),
        session.waba_id
    );
    let _guard = chatbotx_business::channel_token_refresh::refresh_locks()
        .acquire(
            &lock_key,
            LockOpts {
                ttl_secs: WABA_PROVISION_LOCK_SECS,
                retry_secs: Some(WABA_PROVISION_LOCK_SECS),
            },
        )
        .await
        .map_err(|e| system(e.to_string()))?;

    // The first connect of the session may have bound a workspace meanwhile.
    let current = active_session(api, session_id, user_id)
        .await?
        .map(|(s, _)| s);
    let target_workspace = current
        .as_ref()
        .and_then(|s| s.workspace_id)
        .or(session.workspace_id);
    let existing_waba = match target_workspace {
        Some(ws) => {
            let waba_id = session.waba_id.clone();
            tx::read(&api.db, move |conn| {
                Ok(waba_repo::find_by_waba(conn, ws, &waba_id)?)
            })
            .await?
        }
        None => None,
    };
    let provisioned = existing_waba
        .as_ref()
        .and_then(|w| w.provisioned_at)
        .is_none();
    if provisioned {
        setup_oauth_resources(base, &settings.version, &session.waba_id, &settings)
            .await
            .map_err(|e| exception(&e.message))?;
        subscribe_webhook(base, &settings.version, &session.waba_id, &token, false)
            .await
            .map_err(|e| exception(&e.message))?;
    }

    // `resolveCoexistState`.
    let (is_coexist, platform_type) = if input.connect_existing {
        match call(
            Method::GET,
            Some(&token),
            &format!("{base}/{}/{phone_id}", settings.version),
            &[("fields", "is_on_biz_app,platform_type")],
            None,
        )
        .await
        {
            Ok(answer) => {
                let platform = answer["platform_type"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                (
                    answer["is_on_biz_app"] == true && platform == "CLOUD_API",
                    platform,
                )
            }
            Err(error) => {
                tracing::warn!(%error, phone_number_id = phone_id, "[wa-connect] coexist eligibility check failed");
                (false, String::new())
            }
        }
    } else {
        (false, String::new())
    };

    // `persistWhatsappNumber` → `connectPhoneNumber`.
    let auth = auth_value(
        &settings,
        &origin,
        &token,
        &session.waba_id,
        &session.business_id,
        &phone_number,
    );
    let display = chatbotx_business::whatsapp::normalize_display_phone_number(
        phone_number["display_phone_number"]
            .as_str()
            .unwrap_or_default(),
    );
    let verified = phone_number["verified_name"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_owned();
    let name = if verified.is_empty() {
        display.clone()
    } else {
        verified
    };
    let persist = Persist {
        user_id,
        owner_id: session.owner_id,
        session_id,
        phone_number_id: phone_id.to_owned(),
        name: name.clone(),
        display_phone_number: display,
        waba_id: session.waba_id.clone(),
        business_id: session.business_id.clone(),
        auth: auth.clone(),
        is_coexist,
        platform_type,
    };
    let persisted = tx::write(&api.db, move |conn| persist_phone_number(conn, &persist)).await?;
    let workspace_id = persisted.workspace_id;
    if persisted.created_workspace {
        if let Some(jobs) = api.jobs.as_deref() {
            chatbotx_business::template::install_intent::dispatch_staged_installs(
                &api.db,
                jobs,
                workspace_id,
            )
            .await;
        }
        audit(
            api,
            user_id,
            workspace_id,
            "create",
            format!("created the workspace (#{workspace_id})"),
        )
        .await;
    }
    if persisted.was_created {
        audit(
            api,
            user_id,
            workspace_id,
            "connect",
            format!(
                "connected a new WhatsApp channel (#{})",
                persisted.integration_id
            ),
        )
        .await;
    }

    // `persistConnectedWaba` (best effort).
    let scopes = match granted_scopes(base, &token, &settings.app_token(), &session.waba_id).await {
        Ok(scopes) => Some(scopes),
        Err(error) => {
            tracing::warn!(%error, workspace_id, "Unable to persist WhatsApp Business Account credential after connect");
            None
        }
    };
    if let Some(scopes) = scopes.clone() {
        match upsert_waba_credential(
            api,
            workspace_id,
            &session.waba_id,
            &session.business_id,
            &token,
            &settings.version,
            scopes,
        )
        .await
        {
            Ok(Some(row)) if provisioned => {
                let waba_id = session.waba_id.clone();
                if let Err(error) = tx::write(&api.db, move |conn| {
                    Ok(waba_repo::mark_provisioned(
                        conn,
                        workspace_id,
                        &waba_id,
                        row.revision,
                        None,
                    )?)
                })
                .await
                {
                    tracing::warn!(%error, workspace_id, "Unable to persist WhatsApp Business Account credential after connect");
                }
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(%error, workspace_id, "Unable to persist WhatsApp Business Account credential after connect");
            }
        }
    }

    // `runWhatsappPostConnect`: a failure is the `followUpFailed` warning.
    // ponytail: `applyBranding` (the workspace logo from the business
    // profile) is not ported; the logo stays as it is.
    let integration_id = persisted.integration_id;
    let follow_up = async {
        let has_capi = scopes
            .as_ref()
            .ok_or_else(|| "scope check failed".to_owned())?
            .iter()
            .any(|s| s == CAPI_SCOPE);
        tx::write(&api.db, move |conn| {
            Ok(integration_whatsapp::update_capi_scope_cache(
                conn,
                integration_id,
                workspace_id,
                has_capi,
                Some(repo::now_ms()),
                None,
            )?)
        })
        .await
        .map_err(|e| e.to_string())?;
        if is_coexist {
            return Ok::<_, String>((false, Value::Null));
        }
        let row = tx::read(&api.db, move |conn| {
            Ok(integration_whatsapp::find_by_id_for_workspace(
                conn,
                integration_id,
                workspace_id,
            )?)
        })
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "integration row missing".to_owned())?;
        let registration =
            whatsapp_channels::register_phone_number(base, &WhatsappAuth::of(&row), phone_id)
                .await
                .map_err(|e| e.message)?;
        let pending = registration.0 == "pending_verification";
        let error = whatsapp_channels::record_registration(
            &api.db,
            workspace_id,
            integration_id,
            registration,
        )
        .await
        .map_err(|e| e.to_string())?;
        Ok((pending, error.unwrap_or(Value::Null)))
    };
    let (warning, extra) = match follow_up.await {
        Ok((requires_verification, registration_error)) => (
            None,
            json!({
                "requiresPhoneVerification": requires_verification,
                "registrationError": registration_error,
                "displayPhoneNumber": phone_number["display_phone_number"],
                "verifiedName": phone_number["verified_name"],
            }),
        ),
        Err(error) => {
            tracing::error!(%error, integration_id, "WhatsApp connect follow-up failed after the phone number was connected");
            (Some("followUpFailed"), Value::Null)
        }
    };
    let mut item = json!({
        "sourceId": phone_id,
        "name": name,
        "status": "connected",
        "integrationId": integration_id.to_string(),
        "coexistEligible": is_coexist,
        "extra": extra,
    });
    if let Some(warning) = warning {
        item["warning"] = json!(warning);
    }
    Ok(json!({
        "type": "connected",
        "workspaceId": workspace_id.to_string(),
        "isManual": false,
        "redirectUrl": format!("/space/{workspace_id}"),
        "outcome": item,
    }))
}

/// `assertWorkspaceConnectAccess` (the owner-access denial is cloud-only).
async fn assert_member(
    api: &ApiState,
    workspace_id: i64,
    user_id: i64,
) -> Result<(), BusinessError> {
    let member = tx::read(&api.db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT 1 FROM "WorkspaceMember" m JOIN "Workspace" w ON w."id" = m."workspaceId"
                   WHERE m."workspaceId" = ?1 AND m."userId" = ?2"#,
                params![workspace_id, user_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(repo::Error::from)?
            .is_some())
    })
    .await?;
    if member {
        Ok(())
    } else {
        Err(BusinessError::exception(
            "You are not a member of this workspace.",
            "notWorkspaceMember",
            403,
        ))
    }
}

async fn audit(api: &ApiState, user_id: i64, workspace_id: i64, action: &str, detail: String) {
    api.audit
        .record(AuditRecordInput {
            user_id: Some(user_id.to_string()),
            workspace_id: Some(workspace_id.to_string()),
            ..AuditRecordInput::new(action, detail)
        })
        .await;
}

/// The `connectPhoneNumber` input.
struct Persist {
    user_id: i64,
    owner_id: i64,
    session_id: i64,
    phone_number_id: String,
    name: String,
    display_phone_number: String,
    waba_id: String,
    business_id: String,
    auth: Value,
    is_coexist: bool,
    platform_type: String,
}

struct Persisted {
    workspace_id: i64,
    created_workspace: bool,
    integration_id: i64,
    was_created: bool,
}

fn channel_duplicated() -> BusinessError {
    BusinessError::exception(
        "This channel is already connected to another workspace.",
        "channelDuplicated",
        409,
    )
}

/// `connectPhoneNumber`'s transaction: claim the number from the session
/// (a refused claim ends the batch), create and bind the workspace on the
/// session's first number, refuse a number connected elsewhere, then the
/// inbox, the satellite row and the `Connection` (an inactive one revived).
fn persist_phone_number(
    conn: &Connection,
    input: &Persist,
) -> chatbotx_business::Result<Persisted> {
    let now = repo::now_ms();
    let claimed = signup::claim_signup_session_phone_number(
        conn,
        &signup::ClaimInput {
            id: input.session_id,
            user_id: input.user_id,
            owner_id: input.owner_id,
            phone_number_id: input.phone_number_id.clone(),
            now,
        },
    )?
    .ok_or_else(signup_expired)?;
    let (workspace_id, created_workspace) = match claimed.workspace_id {
        Some(id) => (id, false),
        None => {
            let id = crate::spa::insert_workspace(conn, input.user_id, &input.name, None)?;
            signup::bind_signup_session_workspace(conn, input.session_id, id)?;
            (id, true)
        }
    };
    let elsewhere: Option<i64> = conn
        .query_row(
            r#"SELECT "id" FROM "Inbox" WHERE "channel" = 'whatsapp' AND "sourceId" = ?1
                   AND "workspaceId" <> ?2 AND "status" = 'connected' LIMIT 1"#,
            params![input.phone_number_id, workspace_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(repo::Error::from)?;
    if elsewhere.is_some() {
        return Err(channel_duplicated());
    }
    let existing: Option<(i64, String)> = conn
        .query_row(
            r#"SELECT "id", "status" FROM "Connection"
               WHERE "workspaceId" = ?1 AND "provider" = 'whatsapp' AND "sourceId" = ?2"#,
            params![workspace_id, input.phone_number_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(repo::Error::from)?;
    if existing
        .as_ref()
        .is_some_and(|(_, status)| chatbotx_business::connection::is_active(status))
    {
        return Err(BusinessError::exception(
            "This account is already connected.",
            "connectionAlreadyConnected",
            409,
        ));
    }
    let unique = |error: chatbotx_db::rusqlite::Error| -> BusinessError {
        if error.to_string().contains("UNIQUE") {
            channel_duplicated()
        } else {
            repo::Error::from(error).into()
        }
    };
    let inbox_id = repo::new_id();
    conn.execute(
        r#"INSERT INTO "Inbox" ("id", "createdAt", "updatedAt", "name", "channel", "sourceId",
               "workspaceId", "status")
           VALUES (?1, ?2, ?2, ?3, 'whatsapp', ?4, ?5, 'connected')"#,
        params![
            inbox_id,
            now,
            input.name,
            input.phone_number_id,
            workspace_id
        ],
    )
    .map_err(unique)?;
    let integration_id = repo::new_id();
    conn.execute(
        r#"INSERT INTO "IntegrationWhatsapp" ("id", "createdAt", "updatedAt", "auth", "phoneNumberId",
               "wabaId", "businessId", "name", "displayPhoneNumber", "isCoexist", "platformType",
               "workspaceId", "inboxId")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
        params![
            integration_id,
            now,
            input.auth.to_string(),
            input.phone_number_id,
            input.waba_id,
            input.business_id,
            input.name,
            input.display_phone_number,
            input.is_coexist,
            input.platform_type,
            workspace_id,
            inbox_id
        ],
    )
    .map_err(unique)?;
    match &existing {
        Some((id, _)) => conn.execute(
            r#"UPDATE "Connection" SET "inboxId" = ?3, "displayName" = ?4, "status" = 'connected',
                   "statusReason" = NULL, "lastError" = NULL, "connectedAt" = ?5,
                   "disconnectedAt" = NULL, "updatedAt" = ?5
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id, inbox_id, input.name, now],
        ),
        None => conn.execute(
            r#"INSERT INTO "Connection" ("id", "createdAt", "updatedAt", "workspaceId", "provider",
                   "kind", "channel", "inboxId", "sourceId", "displayName", "status", "createdBy",
                   "connectedAt")
               VALUES (?1, ?2, ?2, ?3, 'whatsapp', 'channel', 'whatsapp', ?4, ?5, ?6, 'connected', ?7, ?2)"#,
            params![repo::new_id(), now, workspace_id, inbox_id, input.phone_number_id, input.name, input.user_id],
        ),
    }
    .map_err(unique)?;
    Ok(Persisted {
        workspace_id,
        created_workspace,
        integration_id,
        was_created: existing.is_none(),
    })
}
