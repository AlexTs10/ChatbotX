//! Settings writes of Messenger pages and Instagram accounts (Q.I1b slice 6):
//! `integration-messenger/lib/update-messenger-settings.ts` and
//! `integration-instagram/lib/update-instagram-settings.ts`, with their public
//! input schemas (`schema/public.ts`). Routes live in `channel_integrations`.
//!
//! ponytail: the Graph calls run before the row update instead of inside the
//! DB transaction; a failed call leaves the saved settings unchanged, as the
//! original's rollback does.

use std::collections::{HashMap, HashSet};

use chatbotx_business::messenger_menu::{branding_url, branding_url_for, call_to_actions};
use chatbotx_business::{BusinessError, tx};
use chatbotx_core::flow_config::util::{ButtonPayload, encode_button_payload};
use chatbotx_db::repo::models::{IntegrationInstagram, IntegrationMessenger};
use chatbotx_db::repo::{Json, new_id};
use chatbotx_db::rusqlite::{params, params_from_iter};
use chatbotx_kv::LockOpts;
use reqwest::Method;
use serde_json::{Map, Value, json};

use super::ApiState;
use super::channel_integrations::{find_instagram, find_messenger};
use super::input::{Input, Seg, Str, child, key, type_name};
use super::messenger_menus::menu_item;
use super::messenger_templates::{GRAPH_URL, graph_raw, public_message};
use super::polls::url_at;
use super::tags::{array, digits_at};

const MESSENGER_VERSION: &str = "v23.0";
const INSTAGRAM_LOGIN_VERSION: &str = "v22.0";
const INSTAGRAM_LOGIN_URL: &str = "https://graph.instagram.com";
const SETTINGS_LOCK_SECONDS: u64 = 60;
const WELCOME_PROFILE_READ_TTL_SECONDS: u64 = 5 * 60;
const UPLOAD_MODES: &[&str] = &["url", "file"];

/// `UpdateMessengerRequest` / `UpdateInstagramRequest` (ids kept as sent).
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub welcome_flow_id: Option<String>,
    pub persistent_menus: Vec<Value>,
    pub personas: Vec<Value>,
    pub conversation_starters: Vec<Value>,
}

/// The fields a request sent (`None` = keep the saved value).
#[derive(Debug, Default)]
pub struct Changes {
    pub welcome_flow_id: Option<Option<String>>,
    pub persistent_menus: Option<Vec<Value>>,
    pub personas: Option<Vec<Value>>,
    pub conversation_starters: Option<Vec<Value>>,
    pub mark_read_on_outbound: Option<bool>,
}

impl Changes {
    fn merged(self, mut saved: Settings) -> (Settings, Option<bool>) {
        if let Some(v) = self.welcome_flow_id {
            saved.welcome_flow_id = v;
        }
        if let Some(v) = self.persistent_menus {
            saved.persistent_menus = v;
        }
        if let Some(v) = self.personas {
            saved.personas = v;
        }
        if let Some(v) = self.conversation_starters {
            saved.conversation_starters = v;
        }
        (saved, self.mark_read_on_outbound)
    }
}

fn list(value: &Value) -> Vec<Value> {
    value.as_array().cloned().unwrap_or_default()
}

/// `zodBigintAsString` flow ids inside a stored list.
fn with_string_flow_ids(items: Vec<Value>) -> Vec<Value> {
    items
        .into_iter()
        .map(|mut item| {
            if let Some(id) = item.get("flowId").and_then(Value::as_i64) {
                item["flowId"] = json!(id.to_string());
            }
            item
        })
        .collect()
}

fn saved_messenger(row: &IntegrationMessenger) -> Settings {
    Settings {
        welcome_flow_id: row.welcome_flow_id.map(|id| id.to_string()),
        persistent_menus: with_string_flow_ids(list(&row.persistent_menus.0)),
        personas: list(&row.personas.0),
        conversation_starters: with_string_flow_ids(list(&row.conversation_starters.0)),
    }
}

fn saved_instagram(row: &IntegrationInstagram) -> Settings {
    Settings {
        welcome_flow_id: row.welcome_flow_id.map(|id| id.to_string()),
        persistent_menus: with_string_flow_ids(list(&row.persistent_menus.0)),
        personas: Vec::new(),
        conversation_starters: with_string_flow_ids(list(&row.conversation_starters.0)),
    }
}

// --- input ---------------------------------------------------------------------------

/// `messengerConversationStarterSchema` / `instagramConversationStarterSchema`.
fn starter(input: &mut Input, path: &[Seg], value: &Value) -> Option<Value> {
    let Value::Object(item) = value else {
        let message = format!(
            "Invalid input: expected object, received {}",
            type_name(Some(value))
        );
        input.issue(path, "invalid_type", &message);
        return None;
    };
    let question = input.string_at(
        &child(path, "question"),
        item.get("question"),
        Str::default(),
        true,
    );
    let flow = digits_at(input, &child(path, "flowId"), item.get("flowId"), true);
    (question.is_some() && flow.is_some())
        .then(|| json!({ "question": question, "flowId": item.get("flowId") }))
}

/// `messengerPersonaSchema.shape.profilePicture`.
fn profile_picture(input: &mut Input, path: &[Seg], value: &Value) -> Option<Value> {
    let Value::Object(item) = value else {
        let message = format!(
            "Invalid input: expected object, received {}",
            type_name(Some(value))
        );
        input.issue(path, "invalid_type", &message);
        return None;
    };
    let id = digits_at(input, &child(path, "id"), item.get("id"), true);
    let url = url_at(input, &child(path, "url"), item.get("url"), true);
    let mode = input.enum_at(&child(path, "mode"), item.get("mode"), UPLOAD_MODES, true);
    (id.is_some() && url.is_some() && mode.is_some())
        .then(|| json!({ "id": item.get("id"), "url": url, "mode": mode }))
}

/// `messengerPersonaPublicInput` + `toStoredMessengerPersona`.
fn persona(input: &mut Input, path: &[Seg], value: &Value) -> Option<Value> {
    let Value::Object(item) = value else {
        let message = format!(
            "Invalid input: expected object, received {}",
            type_name(Some(value))
        );
        input.issue(path, "invalid_type", &message);
        return None;
    };
    let id = input.string_at(&child(path, "id"), item.get("id"), Str::default(), false);
    let min1 = Str {
        min: Some(1),
        ..Str::default()
    };
    let name = input.string_at(&child(path, "name"), item.get("name"), min1, true);
    let is_default = input.bool_at(&child(path, "isDefault"), item.get("isDefault"), false);
    let picture_url = url_at(
        input,
        &child(path, "profilePictureUrl"),
        item.get("profilePictureUrl"),
        false,
    );
    let picture = match item.get("profilePicture") {
        None => None,
        Some(value) => profile_picture(input, &child(path, "profilePicture"), value),
    };
    let bad = |key: &str, ok: bool| item.contains_key(key) && !ok;
    if name.is_none()
        || bad("id", id.is_some())
        || bad("isDefault", is_default.is_some())
        || bad("profilePictureUrl", picture_url.is_some())
        || bad("profilePicture", picture.is_some())
    {
        return None;
    }
    // `.refine(...)`: runs once the object itself parsed.
    let picture = match (picture_url, picture) {
        (Some(url), _) => json!({ "id": new_id().to_string(), "url": url, "mode": "url" }),
        (None, Some(picture)) => picture,
        (None, None) => {
            input.issue(
                &child(path, "profilePictureUrl"),
                "custom",
                "Send profilePictureUrl",
            );
            return None;
        }
    };
    Some(json!({
        "id": id.unwrap_or_default(),
        "name": name,
        "isDefault": is_default.unwrap_or(false),
        "profilePicture": picture,
    }))
}

/// `welcomeFlowId: zodBigintAsString().nullable()` (`.optional()` on a patch).
fn welcome_flow(input: &mut Input, patch: bool) -> Option<Option<String>> {
    match input.get("welcomeFlowId").cloned() {
        None if patch => None,
        Some(Value::Null) => Some(None),
        value => {
            let text = value.as_ref().and_then(Value::as_str).map(str::to_owned);
            digits_at(input, &key("welcomeFlowId"), value.as_ref(), true).map(|_| text)
        }
    }
}

fn optional_array(
    input: &mut Input,
    name: &str,
    patch: bool,
    element: impl FnMut(&mut Input, &[Seg], &Value) -> Option<Value>,
) -> Option<Vec<Value>> {
    if patch && !input.has(name) {
        return None;
    }
    array(input, name, 0, usize::MAX, element)
}

/// The settings body of a PUT (`patch` = false) or PATCH, for Messenger
/// (`personas`) or Instagram.
pub fn changes(input: &mut Input, patch: bool, with_personas: bool) -> Changes {
    let welcome_flow_id = welcome_flow(input, patch);
    let persistent_menus = optional_array(input, "persistentMenus", patch, menu_item);
    let personas = if with_personas {
        let personas = optional_array(input, "personas", patch, persona);
        if personas.as_ref().is_some_and(|p| default_count(p) > 1) {
            input.issue(
                &key("personas"),
                "custom",
                "Only one persona can be the default",
            );
        }
        personas
    } else {
        None
    };
    let conversation_starters = optional_array(input, "conversationStarters", patch, starter);
    let mark_read_on_outbound = input.opt_bool("markReadOnOutbound");
    Changes {
        welcome_flow_id,
        persistent_menus,
        personas,
        conversation_starters,
        mark_read_on_outbound,
    }
}

fn default_count(personas: &[Value]) -> usize {
    personas
        .iter()
        .filter(|p| p.get("isDefault").and_then(Value::as_bool) == Some(true))
        .count()
}

// --- shared write steps -----------------------------------------------------------------

/// `collectFlowIds`.
fn flow_ids(settings: &Settings) -> Vec<i64> {
    let menus = settings
        .persistent_menus
        .iter()
        .filter(|m| m.get("type").and_then(Value::as_str) == Some("flow"));
    let ids: HashSet<i64> = settings
        .welcome_flow_id
        .iter()
        .cloned()
        .chain(
            menus
                .chain(&settings.conversation_starters)
                .filter_map(|item| {
                    item.get("flowId")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                }),
        )
        .map(|id| id.parse().unwrap_or(-1))
        .collect();
    ids.into_iter().collect()
}

/// `flowService.assertAllExist`.
async fn assert_flows_exist(state: &ApiState, ws: i64, ids: Vec<i64>) -> Result<(), BusinessError> {
    if ids.is_empty() {
        return Ok(());
    }
    tx::read(&state.db, move |conn| {
        let marks = vec!["?"; ids.len()].join(",");
        let sql =
            format!(r#"SELECT COUNT(*) FROM "Flow" WHERE "workspaceId" = ? AND "id" IN ({marks})"#);
        let mut args: Vec<i64> = vec![ws];
        args.extend(&ids);
        let found: i64 = conn.query_row(&sql, params_from_iter(args), |r| r.get(0))?;
        if usize::try_from(found).unwrap_or(0) != ids.len() {
            return Err(BusinessError::not_found("Flow does not exists."));
        }
        Ok(())
    })
    .await
}

/// `inboxService.updateMarkReadOnOutbound`.
async fn update_mark_read(
    state: &ApiState,
    ws: i64,
    inbox: i64,
    enabled: Option<bool>,
) -> Result<(), BusinessError> {
    let Some(enabled) = enabled else {
        return Ok(());
    };
    tx::write(&state.db, move |conn| {
        conn.execute(
            r#"UPDATE "Inbox" SET "markReadOnOutbound" = ?1 WHERE "id" = ?2 AND "workspaceId" = ?3"#,
            params![enabled, inbox, ws],
        )?;
        Ok(())
    })
    .await
}

/// `withMessengerSettingsLock` / `withInstagramSettingsLock`.
async fn settings_lock(
    state: &ApiState,
    key: String,
) -> Result<Option<chatbotx_kv::LockGuard>, BusinessError> {
    let Some(kv) = &state.kv else { return Ok(None) };
    let opts = LockOpts {
        ttl_secs: SETTINGS_LOCK_SECONDS,
        retry_secs: None,
    };
    kv.locks
        .acquire(&key, opts)
        .await
        .map(Some)
        .map_err(|e| BusinessError::exception(e.to_string(), "systemError", 500))
}

fn flow_payload(flow_id: &str, flow_version_id: Option<String>) -> String {
    encode_button_payload(&ButtonPayload {
        flow_id: flow_id.to_owned(),
        button_id: flow_version_id.as_ref().map(|_| String::new()),
        flow_version_id,
        ..ButtonPayload::default()
    })
}

fn text_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
}

/// A failed Graph call: logged, never fatal (`.catch(logger.warn)`).
fn warn(
    result: Result<Value, super::messenger_templates::Fail>,
    ws: i64,
    id: i64,
    action: &str,
) -> Option<Value> {
    match result {
        Ok(value) => Some(value),
        Err(fail) => {
            tracing::warn!(
                workspace_id = ws,
                integration_id = id,
                action,
                reason = "messengerProfileSyncFailed",
                error = %public_message(&fail, "request failed"),
                "Messenger profile sync failed after settings update"
            );
            None
        }
    }
}

// --- Messenger ----------------------------------------------------------------------------

/// `updateMessenger` (`patch` = false) / `patchMessengerSettings`.
pub async fn write_messenger(
    state: &ApiState,
    ws: i64,
    id: i64,
    changes: Changes,
    patch: bool,
) -> Result<(), BusinessError> {
    let _lock = settings_lock(state, format!("messenger-settings:{id}")).await?;
    let saved = if patch {
        saved_messenger(&find_messenger(&state.db, ws, id).await?)
    } else {
        Settings::default()
    };
    let (settings, mark_read) = changes.merged(saved);
    if default_count(&settings.personas) > 1 {
        return Err(BusinessError::validation(
            "personas",
            "Only one persona can be the default",
        ));
    }
    assert_flows_exist(state, ws, flow_ids(&settings)).await?;
    save_messenger(state, ws, id, settings, mark_read)
        .await
        .map_err(|e| match e {
            // Explicit, actionable messages stay; the rest is the generic one.
            e @ BusinessError::Exception { status: 400, .. } if e.code() == Some("systemError") => {
                e
            }
            e => {
                tracing::debug!(error = %e, "Failed to update Facebook page");
                BusinessError::exception("Failed to update Facebook page", "systemError", 400)
            }
        })
}

struct MessengerAuth {
    token: String,
    page_id: String,
    version: String,
}

fn messenger_auth(row: &IntegrationMessenger) -> MessengerAuth {
    let auth = &row.auth.0;
    MessengerAuth {
        token: text_at(auth, "/tokens/accessToken")
            .unwrap_or_default()
            .to_owned(),
        page_id: text_at(auth, "/metadata/pageId")
            .unwrap_or_default()
            .to_owned(),
        version: text_at(auth, "/version")
            .unwrap_or(MESSENGER_VERSION)
            .to_owned(),
    }
}

async fn save_messenger(
    state: &ApiState,
    ws: i64,
    id: i64,
    settings: Settings,
    mark_read: Option<bool>,
) -> Result<(), BusinessError> {
    let row = find_messenger(&state.db, ws, id).await?;
    let auth = messenger_auth(&row);
    let personas = sync_personas(&auth, &row, &settings.personas, ws).await;
    let default = personas
        .iter()
        .find(|p| p.get("isDefault").and_then(Value::as_bool) == Some(true));
    let persona_id = match default {
        Some(p) => match p.get("facebookPersonaId").and_then(Value::as_str) {
            Some(fb) if !fb.is_empty() => Some(fb.to_owned()),
            // `isRegisteredPersona` failed: surface it, keep the old settings.
            _ => {
                return Err(BusinessError::exception(
                    "Couldn't register the default persona with Facebook. Please try saving again.",
                    "systemError",
                    400,
                ));
            }
        },
        None => None,
    };

    let welcome: Option<i64> = settings
        .welcome_flow_id
        .as_deref()
        .map(|v| v.parse().unwrap_or(-1));
    let menus = Json(Value::Array(settings.persistent_menus.clone()));
    let starters = Json(Value::Array(settings.conversation_starters.clone()));
    let stored_personas = Json(Value::Array(personas));
    tx::write(&state.db, move |conn| {
        conn.execute(
            r#"UPDATE "IntegrationMessenger" SET "welcomeFlowId" = ?1, "persistentMenus" = ?2,
                 "personas" = ?3, "conversationStarters" = ?4, "personaId" = ?5
               WHERE "id" = ?6 AND "workspaceId" = ?7"#,
            params![
                welcome,
                menus,
                stored_personas,
                starters,
                persona_id,
                id,
                ws
            ],
        )?;
        Ok(())
    })
    .await?;
    update_mark_read(state, ws, row.inbox_id, mark_read).await?;

    let profile_url = format!("{GRAPH_URL}/{}/me/messenger_profile", auth.version);
    let token = Some(auth.token.as_str());
    let mut fields = Vec::new();
    if settings.persistent_menus.is_empty() {
        fields.push("PERSISTENT_MENU");
    }
    if settings.conversation_starters.is_empty() {
        fields.push("ICE_BREAKERS");
    }
    if !fields.is_empty() {
        let body = json!({ "fields": fields });
        let result = graph_raw(Method::DELETE, token, &profile_url, &[], Some(&body)).await;
        warn(result, ws, id, "deleteProfileFields");
    }
    if let Some(domain) = https_origin(&state.app_url) {
        let result = ensure_whitelisted_domain(token, &profile_url, &domain).await;
        warn(result, ws, id, "ensureMessengerWhitelistedDomain");
    }

    // `getMessengerProfileParams`: `get_started` is always sent.
    let mut profile = Map::new();
    let get_started = match &settings.welcome_flow_id {
        Some(flow) => flow_payload(flow, None),
        None => "GET_STARTED".to_owned(),
    };
    profile.insert("get_started".into(), json!({ "payload": get_started }));
    if !settings.persistent_menus.is_empty() {
        let menus = Value::Array(settings.persistent_menus.clone());
        let ctas = call_to_actions(&menus, &branding_url(&state.app_url));
        profile.insert(
            "persistent_menu".into(),
            json!([{ "locale": "default", "composer_input_disabled": false, "call_to_actions": ctas }]),
        );
    }
    if !settings.conversation_starters.is_empty() {
        let ice: Vec<Value> = settings
            .conversation_starters
            .iter()
            .map(|s| {
                let flow = s.get("flowId").and_then(Value::as_str).unwrap_or_default();
                json!({ "question": s.get("question"), "payload": flow_payload(flow, None) })
            })
            .collect();
        profile.insert("ice_breakers".into(), Value::Array(ice));
    }
    let body = Value::Object(profile);
    let result = graph_raw(Method::POST, token, &profile_url, &[], Some(&body)).await;
    warn(result, ws, id, "updateProfile");
    if claim_welcome_profile_read(state, &auth.page_id).await {
        // `logMessengerWelcomeProfile`: diagnostic, single attempt.
        let query = [("fields", "get_started,ice_breakers,greeting")];
        match graph_raw(Method::GET, token, &profile_url, &query, None).await {
            Ok(profile) => {
                tracing::info!(page_id = %auth.page_id, reason = "profileUpdated", profile = %profile["data"][0], "Messenger welcome profile")
            }
            Err(_) => {
                tracing::debug!(page_id = %auth.page_id, "Skipped Messenger welcome profile read-back")
            }
        }
    }
    Ok(())
}

/// `normalizeMessengerWhitelistedDomain`: the https origin, else none.
fn https_origin(app_url: &str) -> Option<String> {
    let url = url::Url::parse(app_url).ok()?;
    (url.scheme() == "https").then(|| url.origin().ascii_serialization())
}

/// `ensureMessengerWhitelistedDomain`.
async fn ensure_whitelisted_domain(
    token: Option<&str>,
    profile_url: &str,
    domain: &str,
) -> Result<Value, super::messenger_templates::Fail> {
    let query = [("fields", "whitelisted_domains")];
    let current = graph_raw(Method::GET, token, profile_url, &query, None).await?;
    let mut domains = list(&current["whitelisted_domains"]);
    if domains.iter().any(|d| d.as_str() == Some(domain)) {
        return Ok(Value::Null);
    }
    domains.push(json!(domain));
    let body = json!({ "whitelisted_domains": domains });
    graph_raw(Method::POST, token, profile_url, &[], Some(&body)).await
}

/// `claimWelcomeProfileRead`: `SET NX EX` per page; fails closed.
async fn claim_welcome_profile_read(state: &ApiState, page_id: &str) -> bool {
    let Some(kv) = state.kv.as_ref().filter(|_| !page_id.is_empty()) else {
        return false;
    };
    let key = format!("messenger:welcome-profile-read:{page_id}");
    match kv
        .durable
        .set_number_if_not_exists(&key, 1.0, WELCOME_PROFILE_READ_TTL_SECONDS)
        .await
    {
        Ok(claimed) => claimed,
        Err(e) => {
            tracing::warn!(error = %e, page_id, "Skipped Messenger welcome profile read-back: throttle unavailable");
            false
        }
    }
}

/// `syncMessengerPersonas` + the `syncPersonas` action: local ids, carried
/// Facebook ids for unchanged personas, creates for the rest, deletes for
/// Facebook personas no longer used. Single failures are logged and skipped.
async fn sync_personas(
    auth: &MessengerAuth,
    row: &IntegrationMessenger,
    personas: &[Value],
    ws: i64,
) -> Vec<Value> {
    let old: HashMap<&str, &Value> = row
        .personas
        .0
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p.get("id")?.as_str()?, p)))
        .collect();
    let mut out = Vec::with_capacity(personas.len());
    for persona in personas {
        let mut persona = persona.clone();
        let id = match persona.get("id").and_then(Value::as_str) {
            Some(id) if !id.is_empty() => id.to_owned(),
            _ => new_id().to_string(),
        };
        let url = persona.pointer("/profilePicture/url").cloned();
        let carried = old
            .get(id.as_str())
            .filter(|o| {
                o.get("name") == persona.get("name")
                    && o.pointer("/profilePicture/url") == url.as_ref()
            })
            .and_then(|o| o.get("facebookPersonaId").and_then(Value::as_str))
            .filter(|fb| !fb.is_empty())
            .map(str::to_owned);
        let facebook_id = match carried {
            Some(fb) => Some(fb),
            None => {
                // `createPersona`: unversioned, token in the query.
                let body = json!({ "name": persona.get("name"), "profile_picture_url": url });
                let url = format!("{GRAPH_URL}/me/personas");
                let query = [("access_token", auth.token.as_str())];
                match graph_raw(Method::POST, None, &url, &query, Some(&body)).await {
                    Ok(created) => created.get("id").and_then(Value::as_str).map(str::to_owned),
                    Err(fail) => {
                        tracing::error!(workspace_id = ws, persona = %id, error = %public_message(&fail, "request failed"), "Failed to register Messenger persona");
                        None
                    }
                }
            }
        };
        persona["id"] = json!(id);
        match facebook_id {
            Some(fb) => persona["facebookPersonaId"] = json!(fb),
            None => {
                if let Some(map) = persona.as_object_mut() {
                    map.remove("facebookPersonaId");
                }
            }
        }
        out.push(persona);
    }

    let keep: HashSet<&str> = out
        .iter()
        .filter_map(|p| p.get("facebookPersonaId").and_then(Value::as_str))
        .collect();
    let token = Some(auth.token.as_str());
    match graph_raw(
        Method::GET,
        token,
        &format!("{GRAPH_URL}/me/personas"),
        &[],
        None,
    )
    .await
    {
        Ok(existing) => {
            for persona in list(&existing["data"]) {
                let Some(fb) = persona.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if keep.contains(fb) {
                    continue;
                }
                let url = format!("{GRAPH_URL}/{fb}");
                if let Err(fail) = graph_raw(Method::DELETE, token, &url, &[], None).await {
                    tracing::error!(error = %public_message(&fail, "request failed"), "Failed to reconcile deleted Messenger personas");
                }
            }
        }
        Err(fail) => {
            tracing::error!(error = %public_message(&fail, "request failed"), "Failed to reconcile deleted Messenger personas");
        }
    }
    out
}

// --- Instagram ----------------------------------------------------------------------------

/// `updateInstagram` (`patch` = false) / `patchInstagramSettings`.
pub async fn write_instagram(
    state: &ApiState,
    ws: i64,
    id: i64,
    changes: Changes,
    patch: bool,
) -> Result<(), BusinessError> {
    let _lock = settings_lock(state, format!("instagram-settings:{id}")).await?;
    let saved = if patch {
        saved_instagram(&find_instagram(&state.db, ws, id).await?)
    } else {
        Settings::default()
    };
    let (settings, mark_read) = changes.merged(saved);
    // Before the catch-all: a foreign flow id keeps its own not-found error.
    assert_flows_exist(state, ws, flow_ids(&settings)).await?;
    save_instagram(state, ws, id, settings, mark_read)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to update Instagram integration");
            BusinessError::exception("Failed to update Instagram integration", "systemError", 400)
        })
}

/// `parseInstagramButtons` / `buildIceBreakersParams`: payloads carry the
/// latest flow version (`findOrFail`).
fn latest_version(
    conn: &chatbotx_db::rusqlite::Connection,
    flow_id: &str,
) -> Result<String, BusinessError> {
    let flow: i64 = flow_id.parse().unwrap_or(-1);
    conn.query_row(
        r#"SELECT "id" FROM "FlowVersion" WHERE "flowId" = ?1 AND "isLatest" = 1"#,
        params![flow],
        |r| r.get::<_, i64>(0),
    )
    .map(|id| id.to_string())
    .map_err(|_| BusinessError::not_found("Record not found"))
}

async fn save_instagram(
    state: &ApiState,
    ws: i64,
    id: i64,
    settings: Settings,
    mark_read: Option<bool>,
) -> Result<(), BusinessError> {
    let row = find_instagram(&state.db, ws, id).await?;
    let auth = &row.auth.0;
    let token = text_at(auth, "/tokens/accessToken")
        .unwrap_or_default()
        .to_owned();
    // Facebook-Login accounts use the Facebook Graph host and `auth.version`.
    let (host, version) = if row.r#type == "facebook" {
        (
            GRAPH_URL,
            text_at(auth, "/version").unwrap_or(MESSENGER_VERSION),
        )
    } else {
        (
            INSTAGRAM_LOGIN_URL,
            text_at(auth, "/metadata/version").unwrap_or(INSTAGRAM_LOGIN_VERSION),
        )
    };
    let profile_url = format!("{host}/{version}/me/messenger_profile");

    let branding = branding_url_for(&state.app_url, "instagram");
    let menus = settings.persistent_menus.clone();
    let starters = settings.conversation_starters.clone();
    let (ice_breakers, menu_ctas) = tx::read(&state.db, move |conn| {
        let ice = starters
            .iter()
            .map(|s| {
                let flow = s.get("flowId").and_then(Value::as_str).unwrap_or_default();
                let payload = flow_payload(flow, Some(latest_version(conn, flow)?));
                Ok(json!({ "question": s.get("question"), "payload": payload }))
            })
            .collect::<Result<Vec<_>, BusinessError>>()?;
        // `moveBrandingMenuLast`.
        let mut ordered: Vec<&Value> = menus.iter().collect();
        let is_branding = |m: &&Value| {
            m.get("type").and_then(Value::as_str) == Some("url") && m.get("url").and_then(Value::as_str) == Some(&branding)
        };
        if let Some(index) = ordered.iter().position(is_branding) {
            let item = ordered.remove(index);
            ordered.push(item);
        }
        let mut ctas = Vec::new();
        for menu in ordered {
            match menu.get("type").and_then(Value::as_str) {
                Some("flow") => {
                    let flow = menu.get("flowId").and_then(Value::as_str).unwrap_or_default();
                    let payload = flow_payload(flow, Some(latest_version(conn, flow)?));
                    ctas.push(json!({ "type": "postback", "title": menu.get("label"), "payload": payload }));
                }
                Some("url") => ctas.push(json!({ "type": "web_url", "title": menu.get("label"), "url": menu.get("url") })),
                _ => {}
            }
        }
        Ok((ice, ctas))
    })
    .await?;

    let bearer = Some(token.as_str());
    let mut fields = Vec::new();
    if settings.conversation_starters.is_empty() {
        fields.push("ICE_BREAKERS");
    }
    if settings.persistent_menus.is_empty() {
        fields.push("PERSISTENT_MENU");
    }
    let fail = |fail: super::messenger_templates::Fail| {
        BusinessError::exception(public_message(&fail, "request failed"), "systemError", 400)
    };
    if !fields.is_empty() {
        let body = json!({ "platform": "instagram", "fields": fields });
        graph_raw(Method::DELETE, bearer, &profile_url, &[], Some(&body))
            .await
            .map_err(fail)?;
    }
    let mut profile = Map::new();
    if !ice_breakers.is_empty() {
        profile.insert(
            "ice_breakers".into(),
            json!([{ "locale": "default", "call_to_actions": ice_breakers }]),
        );
    }
    if !menu_ctas.is_empty() {
        profile.insert(
            "persistent_menu".into(),
            json!([{ "locale": "default", "call_to_actions": menu_ctas }]),
        );
    }
    if !profile.is_empty() {
        profile.insert("platform".into(), json!("instagram"));
        let query = [("platform", "instagram"), ("access_token", token.as_str())];
        graph_raw(
            Method::POST,
            bearer,
            &profile_url,
            &query,
            Some(&Value::Object(profile)),
        )
        .await
        .map_err(fail)?;
    }

    let welcome: Option<i64> = settings
        .welcome_flow_id
        .as_deref()
        .map(|v| v.parse().unwrap_or(-1));
    let menus = Json(Value::Array(settings.persistent_menus));
    let starters = Json(Value::Array(settings.conversation_starters));
    tx::write(&state.db, move |conn| {
        conn.execute(
            r#"UPDATE "IntegrationInstagram" SET "welcomeFlowId" = ?1, "persistentMenus" = ?2, "conversationStarters" = ?3
               WHERE "id" = ?4 AND "workspaceId" = ?5"#,
            params![welcome, menus, starters, id, ws],
        )?;
        Ok(())
    })
    .await?;
    update_mark_read(state, ws, row.inbox_id, mark_read).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_ids_are_unique_and_skip_url_menus() {
        let settings = Settings {
            welcome_flow_id: Some("1".into()),
            persistent_menus: vec![
                json!({ "type": "flow", "flowId": "1" }),
                json!({ "type": "url", "url": "https://x.test" }),
            ],
            personas: Vec::new(),
            conversation_starters: vec![json!({ "question": "q", "flowId": "2" })],
        };
        let mut ids = flow_ids(&settings);
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn https_origin_only_for_https() {
        assert_eq!(
            https_origin("https://app.test/x"),
            Some("https://app.test".into())
        );
        assert_eq!(https_origin("http://app.test"), None);
    }

    #[test]
    fn patch_keeps_unsent_fields() {
        let saved = Settings {
            welcome_flow_id: Some("5".into()),
            conversation_starters: vec![json!({ "question": "q", "flowId": "5" })],
            ..Settings::default()
        };
        let changes = Changes {
            welcome_flow_id: Some(None),
            ..Changes::default()
        };
        let (merged, mark) = changes.merged(saved);
        assert_eq!(merged.welcome_flow_id, None);
        assert_eq!(merged.conversation_starters.len(), 1);
        assert_eq!(mark, None);
    }
}
