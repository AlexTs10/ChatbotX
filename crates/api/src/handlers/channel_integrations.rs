//! `features/channel-integrations/api/public.ts` (`createChannelReadRoutes`,
//! `createHandoverResumeFlowRoute`) and the Messenger / Instagram channel
//! routes of `features/integration-{messenger,instagram}/api/public.ts`
//! (scope `channels`; Q.I1b slice 6): list / get, tag sync, handover resume
//! flow, settings (writes in `channel_settings`); the TikTok / Zalo routes
//! of `integration-{tiktok,zalo}/api/public.ts` (Q.I2) in
//! [`handle_tiktok_zalo`].
//!
//! ponytail: no `assertReferencable` template locks (P3.6-D1 rule) and no
//! `#messengers` cache tags (no read cache in Rust).

use chatbotx_business::channel_integration::{self, TIKTOK_BUSINESS_API_URL};
use chatbotx_business::connection::credentials::HttpUpstream;
use chatbotx_business::{BusinessError, tx};
use chatbotx_db::repo::models::{IntegrationInstagram, IntegrationMessenger, IntegrationWhatsapp};
use chatbotx_db::repo::{find_many, find_one, integration_messenger, now_ms};
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};

use reqwest::Method;

use super::input::{Input, key};
use super::messenger_templates::{
    Fail, GRAPH_URL, graph_call, graph_raw, public_message, reply_error,
};
use super::tags::digits_at;
use super::{
    ApiState, Call, FINDING, LISTING, MUTATING, Reply, business_error, date, json, no_content,
    opt_date,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "messengerChannels.list",
    "messengerChannels.get",
    "messengerChannels.updateTagSync",
    "messengerChannels.updateHandoverResumeFlow",
    "messengerChannels.getSettings",
    "messengerChannels.updateSettings",
    "messengerChannels.patchSettings",
    "instagramChannels.list",
    "instagramChannels.get",
    "instagramChannels.getSettings",
    "instagramChannels.updateSettings",
    "instagramChannels.patchSettings",
    "messengerChannels.disconnect",
    "instagramChannels.disconnect",
    "tiktokChannels.list",
    "tiktokChannels.get",
    "tiktokChannels.getCommentToMessage",
    "tiktokChannels.updateCommentToMessage",
    "zaloChannels.list",
    "zaloChannels.get",
    "zaloChannels.updateTagSync",
    "channelIntegrations.list",
];

/// `channelIntegrationChannels`, in the order `channelIntegrationService.list`
/// concatenates them (`channelFetchers` keys).
const CHANNELS: &[&str] = &["whatsapp", "messenger", "instagram", "zalo", "tiktok"];

/// `channelIntegrations.list`: every connected channel, or only `channel`.
async fn list_all(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let value = input.get("channel").cloned();
    let only = input.enum_at(&key("channel"), value.as_ref(), CHANNELS, false);
    input.finish()?;
    let (db, ws) = (&state.db, call.workspace_id);
    let wanted = |only: Option<&str>, channel: &str| only.is_none_or(|o| o == channel);
    let filter = only.clone();
    let mut out: Vec<Value> = tx::read(db, move |conn| {
        let wanted = |channel: &str| wanted(filter.as_deref(), channel);
        let mut out = Vec::new();
        let scope = r#"WHERE "workspaceId" = ?1"#;
        if wanted("whatsapp") {
            let rows = find_many::<IntegrationWhatsapp>(conn, scope, &[&ws])?;
            out.extend(rows.iter().map(super::whatsapp_channels::summary));
        }
        if wanted("messenger") {
            let rows = find_many::<IntegrationMessenger>(conn, scope, &[&ws])?;
            out.extend(rows.iter().map(messenger_summary));
        }
        if wanted("instagram") {
            let rows = find_many::<IntegrationInstagram>(conn, scope, &[&ws])?;
            out.extend(rows.iter().map(instagram_summary));
        }
        Ok(out)
    })
    .await
    .map_err(|e| business_error(e, LISTING))?;
    for channel in ["zalo", "tiktok"] {
        if wanted(only.as_deref(), channel) {
            let rows = channel_integration::list(db, channel, ws, None)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            out.extend(rows.unwrap_or_default());
        }
    }
    Ok(json(200, Value::Array(out)))
}

/// `resolveCapiConnection(...) === "connected"`; Messenger and Instagram
/// need the CAPI scope unless a dedicated token is stored.
fn capi_connected(disconnected: bool, dataset: &Option<String>, token: bool, scope: bool) -> bool {
    !disconnected && dataset.is_some() && (token || scope)
}

/// `summarize` (`ChannelIntegrationSummary`) with the channel defaults.
fn summary(channel: &str, fields: Value) -> Value {
    let mut out = json!({
        "id": Value::Null,
        "channel": channel,
        "inboxId": Value::Null,
        "name": Value::Null,
        "externalId": Value::Null,
        "wabaId": Value::Null,
        "displayPhoneNumber": Value::Null,
        "username": Value::Null,
        "coexistEnabled": false,
        "isCoexist": Value::Null,
        "hasCapiScope": false,
        "capiConnected": false,
        "datasetId": Value::Null,
        "capiTestEventCode": Value::Null,
        "capiDisconnected": false,
        "syncTagEnabledAt": Value::Null,
        "handoverResumeFlowId": Value::Null,
        "tokenRefreshError": Value::Null,
        // `adsEligibleChannelTypes`: whatsapp, messenger, instagram.
        "adsEligible": true,
    });
    if let (Value::Object(out), Value::Object(fields)) = (&mut out, fields) {
        out.extend(fields);
    }
    out
}

fn messenger_summary(row: &IntegrationMessenger) -> Value {
    let disconnected = row.capi_disconnected_at.is_some();
    summary(
        "messenger",
        json!({
            "id": row.id.to_string(),
            "inboxId": row.inbox_id.to_string(),
            "name": row.name,
            "externalId": row.page_id,
            "coexistEnabled": row.coexist_enabled,
            "hasCapiScope": row.has_capi_scope,
            "capiConnected": capi_connected(disconnected, &row.dataset_id, row.capi_access_token.is_some(), row.has_capi_scope),
            "datasetId": row.dataset_id,
            "capiTestEventCode": row.capi_test_event_code,
            "capiDisconnected": disconnected,
            "syncTagEnabledAt": row.sync_tag_enabled_at.map(date),
            "handoverResumeFlowId": row.handover_resume_flow_id.map(|id| id.to_string()),
            "tokenRefreshError": row.token_refresh_error,
        }),
    )
}

fn instagram_summary(row: &IntegrationInstagram) -> Value {
    let disconnected = row.capi_disconnected_at.is_some();
    summary(
        "instagram",
        json!({
            "id": row.id.to_string(),
            "inboxId": row.inbox_id.to_string(),
            "name": row.name,
            "externalId": row.ig_id,
            // Click-to-message ads need a Facebook-login account.
            "adsEligible": row.r#type == "facebook",
            "username": row.username,
            "coexistEnabled": row.coexist_enabled,
            "hasCapiScope": row.has_capi_scope,
            "capiConnected": capi_connected(disconnected, &row.dataset_id, row.capi_access_token.is_some(), row.has_capi_scope),
            "datasetId": row.dataset_id,
            "capiTestEventCode": row.capi_test_event_code,
            "capiDisconnected": disconnected,
            "tokenRefreshError": row.token_refresh_error,
        }),
    )
}

/// `zodBigintAsString` ids inside stored settings lists.
fn flow_ids_as_strings(list: &Value) -> Value {
    let mut list = list.clone();
    for item in list.as_array_mut().into_iter().flatten() {
        if let Some(id) = item.get("flowId").and_then(Value::as_i64) {
            item["flowId"] = json!(id.to_string());
        }
    }
    list
}

/// `messengerSettingsPublicResource`.
pub fn messenger_settings(row: &IntegrationMessenger) -> Value {
    json!({
        "welcomeFlowId": row.welcome_flow_id.map(|id| id.to_string()),
        "persistentMenus": flow_ids_as_strings(&row.persistent_menus.0),
        "personas": row.personas.0,
        "conversationStarters": flow_ids_as_strings(&row.conversation_starters.0),
    })
}

/// `instagramSettingsPublicResource`.
pub fn instagram_settings(row: &IntegrationInstagram) -> Value {
    json!({
        "welcomeFlowId": row.welcome_flow_id.map(|id| id.to_string()),
        "conversationStarters": flow_ids_as_strings(&row.conversation_starters.0),
        "persistentMenus": flow_ids_as_strings(&row.persistent_menus.0),
    })
}

fn id_param(input: &mut Input, name: &str) -> Option<i64> {
    let value = input.get(name).cloned();
    digits_at(input, &key(name), value.as_ref(), true)
}

/// `findOrFail` (`Record not found`) of the workspace's Messenger page.
pub async fn find_messenger(
    db: &chatbotx_db::Db,
    ws: i64,
    id: i64,
) -> Result<IntegrationMessenger, BusinessError> {
    tx::read(db, move |conn| {
        find_one::<IntegrationMessenger>(conn, r#""id" = ?1 AND "workspaceId" = ?2"#, &[&id, &ws])?
            .ok_or_else(|| BusinessError::not_found("Record not found"))
    })
    .await
}

/// `findOrFail` (`Record not found`) of the workspace's Instagram account.
pub async fn find_instagram(
    db: &chatbotx_db::Db,
    ws: i64,
    id: i64,
) -> Result<IntegrationInstagram, BusinessError> {
    tx::read(db, move |conn| {
        find_one::<IntegrationInstagram>(conn, r#""id" = ?1 AND "workspaceId" = ?2"#, &[&id, &ws])?
            .ok_or_else(|| BusinessError::not_found("Record not found"))
    })
    .await
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    if route_id == "channelIntegrations.list" {
        return list_all(state, call).await;
    }
    if route_id.starts_with("tiktokChannels.") || route_id.starts_with("zaloChannels.") {
        return handle_tiktok_zalo(state, route_id, call).await;
    }
    let db = &state.db;
    let ws = call.workspace_id;
    let not_found = || BusinessError::not_found("Channel integration not found");
    match route_id {
        "messengerChannels.list" => tx::read(db, move |conn| {
            Ok(find_many::<IntegrationMessenger>(
                conn,
                r#"WHERE "workspaceId" = ?1"#,
                &[&ws],
            )?)
        })
        .await
        .map(|rows| json(200, rows.iter().map(messenger_summary).collect()))
        .map_err(|e| business_error(e, LISTING)),
        "instagramChannels.list" => tx::read(db, move |conn| {
            Ok(find_many::<IntegrationInstagram>(
                conn,
                r#"WHERE "workspaceId" = ?1"#,
                &[&ws],
            )?)
        })
        .await
        .map(|rows| json(200, rows.iter().map(instagram_summary).collect()))
        .map_err(|e| business_error(e, LISTING)),
        "messengerChannels.get" | "instagramChannels.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_param(&mut input, "id").unwrap_or_default();
            input.finish()?;
            let out = if route_id == "messengerChannels.get" {
                find_messenger(db, ws, id)
                    .await
                    .map(|r| messenger_summary(&r))
            } else {
                find_instagram(db, ws, id)
                    .await
                    .map(|r| instagram_summary(&r))
            };
            out.map_err(|e| match e {
                BusinessError::Exception { .. } => not_found(),
                other => other,
            })
            .map(|out| json(200, out))
            .map_err(|e| business_error(e, FINDING))
        }
        "messengerChannels.getSettings" | "instagramChannels.getSettings" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_param(&mut input, "id").unwrap_or_default();
            input.finish()?;
            let out = if route_id == "messengerChannels.getSettings" {
                find_messenger(db, ws, id)
                    .await
                    .map(|r| messenger_settings(&r))
            } else {
                find_instagram(db, ws, id)
                    .await
                    .map(|r| instagram_settings(&r))
            };
            out.map(|out| json(200, out))
                .map_err(|e| business_error(e, FINDING))
        }
        "messengerChannels.updateTagSync" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_param(&mut input, "id").unwrap_or_default();
            let enabled = input.bool("enabled");
            input.finish()?;
            let enabled = enabled.unwrap_or_default();
            let at = tx::write(db, move |conn| {
                let at = enabled.then(now_ms);
                let changed = conn.execute(
                    r#"UPDATE "IntegrationMessenger" SET "syncTagEnabledAt" = ?1 WHERE "id" = ?2 AND "workspaceId" = ?3"#,
                    params![at, id, ws],
                )?;
                if changed == 0 {
                    return Err(BusinessError::not_found("Messenger channel not found"));
                }
                Ok(at)
            })
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, json!({ "syncTagEnabledAt": at.map(date) })))
        }
        "messengerChannels.updateHandoverResumeFlow" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_param(&mut input, "id").unwrap_or_default();
            let flow = match input.get("handoverResumeFlowId").cloned() {
                Some(Value::Null) => Some(None),
                value => digits_at(
                    &mut input,
                    &key("handoverResumeFlowId"),
                    value.as_ref(),
                    true,
                )
                .map(Some),
            };
            input.finish()?;
            let flow = flow.flatten();
            tx::write(db, move |conn| {
                // `flowService.findActiveById`.
                if let Some(flow) = flow {
                    let active: Option<i64> = conn
                        .query_row(
                            r#"SELECT "id" FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "active" = 1"#,
                            params![flow, ws],
                            |r| r.get(0),
                        )
                        .ok();
                    if active.is_none() {
                        return Err(BusinessError::not_found("Handover flow not found"));
                    }
                }
                integration_messenger::update_handover_resume_flow(conn, id, ws, flow)?
                    .ok_or_else(|| BusinessError::not_found("Messenger integration not found"))
            })
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        "messengerChannels.updateSettings"
        | "messengerChannels.patchSettings"
        | "instagramChannels.updateSettings"
        | "instagramChannels.patchSettings" => {
            let patch = route_id.ends_with("patchSettings");
            let messenger = route_id.starts_with("messenger");
            let mut input = Input::from_body(call.params, call.body);
            let id = id_param(&mut input, "id").unwrap_or_default();
            let changes = super::channel_settings::changes(&mut input, patch, messenger);
            input.finish()?;
            let written = if messenger {
                super::channel_settings::write_messenger(state, ws, id, changes, patch).await
            } else {
                super::channel_settings::write_instagram(state, ws, id, changes, patch).await
            };
            written.map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        "messengerChannels.disconnect" | "instagramChannels.disconnect" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_param(&mut input, "id").unwrap_or_default();
            input.finish()?;
            let done = if route_id == "messengerChannels.disconnect" {
                disconnect_messenger(state, ws, id).await
            } else {
                disconnect_instagram(state, ws, id).await
            };
            done?;
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}

// --- disconnect ---------------------------------------------------------------------------

const MESSENGER_VERSION: &str = "v23.0";
const INSTAGRAM_LOGIN_VERSION: &str = "v22.0";
const INSTAGRAM_LOGIN_URL: &str = "https://graph.instagram.com";
/// `REVOKED_TOKEN_SUBCODES` (Instagram error mappers).
const REVOKED_TOKEN_SUBCODES: &[i64] = &[458, 460, 463, 467];

fn auth_text<'a>(auth: &'a Value, pointer: &str) -> &'a str {
    auth.pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn or_default<'a>(value: &'a str, default: &'a str) -> &'a str {
    if value.is_empty() { default } else { value }
}

/// `existsForPage` of the other Meta channel (same page, same app).
fn sibling_exists(
    conn: &chatbotx_db::rusqlite::Connection,
    table: &str,
    page_id: &str,
    client_id: &str,
) -> Result<bool, BusinessError> {
    let sql = format!(
        r#"SELECT EXISTS(SELECT 1 FROM "{table}" WHERE "pageId" = ?1 AND (?2 = '' OR json_extract("auth", '$.clientId') = ?2))"#
    );
    Ok(conn.query_row(&sql, params![page_id, client_id], |r| r.get(0))?)
}

/// The local teardown of both channels: coexist runs, CAPI events, (tags,)
/// the row and the inbox.
async fn remove_channel(
    state: &ApiState,
    ws: i64,
    id: i64,
    inbox: i64,
    channel: &'static str,
) -> Result<(), BusinessError> {
    tx::write(&state.db, move |conn| {
        let table = if channel == "messenger" {
            // `coexistService.tearDownForIntegration` + tag cleanup.
            chatbotx_db::repo::coexist_sync_run::tear_down_active_runs_for_integration(conn, channel, id, "Integration disconnected")?;
            conn.execute(
                r#"DELETE FROM "TagChannel" WHERE "channelType" = 'messenger' AND "integrationId" = ?1"#,
                params![id],
            )?;
            "IntegrationMessenger"
        } else {
            "IntegrationInstagram"
        };
        chatbotx_db::repo::meta_capi_event::delete_by_integration(conn, ws, channel, id)?;
        conn.execute(&format!(r#"DELETE FROM "{table}" WHERE "id" = ?1"#), params![id])?;
        chatbotx_business::connection::disconnect_inbox(conn, inbox, ws, now_ms())
    })
    .await
}

async fn audit_disconnect(state: &ApiState, ws: i64, label: &str, id: i64) {
    state
        .audit
        .record(chatbotx_business::audit::AuditRecordInput {
            workspace_id: Some(ws.to_string()),
            ..chatbotx_business::audit::AuditRecordInput::new(
                "disconnect",
                format!("disconnected the {label} channel (#{id})"),
            )
        })
        .await;
}

/// `disconnectMessenger` + `disconnectMessengerConnection`: the Meta side is
/// best effort (logged), the local teardown always runs.
async fn disconnect_messenger(state: &ApiState, ws: i64, id: i64) -> Result<(), super::OrpcError> {
    let row = find_messenger(&state.db, ws, id)
        .await
        .map_err(|_| BusinessError::not_found("Messenger channel not found"))
        .map_err(|e| business_error(e, MUTATING))?;
    let auth = &row.auth.0;
    let page_id = auth_text(auth, "/metadata/pageId").to_owned();
    let client_id = auth_text(auth, "/clientId").to_owned();
    let token = auth_text(auth, "/tokens/accessToken").to_owned();
    let version = or_default(auth_text(auth, "/metadata/version"), MESSENGER_VERSION).to_owned();
    let profile_version = or_default(auth_text(auth, "/version"), MESSENGER_VERSION).to_owned();
    let app_token = format!("{client_id}|{}", auth_text(auth, "/clientSecret"));

    let (page, client) = (page_id.clone(), client_id.clone());
    let shared = tx::read(&state.db, move |conn| {
        sibling_exists(conn, "IntegrationInstagram", &page, &client)
    })
    .await;
    let log = |what: &str, fail: &Fail| {
        tracing::warn!(page_id = %page_id, workspace_id = ws, integration_id = id, error = %public_message(fail, "Provider-side teardown failed"), "{what}");
    };
    match shared {
        // Keep the subscription the Instagram account of this page needs.
        Ok(true) => {
            let url = format!("{GRAPH_URL}/{version}/me/subscribed_apps");
            let body = json!({ "subscribed_fields": "general_info" });
            if let Err(fail) = graph_raw(Method::POST, Some(&token), &url, &[], Some(&body)).await {
                log(
                    "Failed to preserve shared Messenger webhook subscription during disconnect",
                    &fail,
                );
            }
        }
        Ok(false) => {
            let url = format!("{GRAPH_URL}/{profile_version}/me/messenger_profile");
            let body = json!({ "fields": ["persistent_menu"] });
            if let Err(fail) = graph_raw(Method::DELETE, Some(&token), &url, &[], Some(&body)).await
            {
                log(
                    "Failed to clear Messenger persistent menu before disconnect",
                    &fail,
                );
            }
            let url = format!("{GRAPH_URL}/{version}/{page_id}/subscribed_apps");
            match graph_raw(Method::DELETE, Some(&app_token), &url, &[], None).await {
                Ok(out) if out["success"] == json!(true) => {}
                Ok(_) => tracing::warn!(page_id = %page_id, "Unsubscribe failed for page"),
                Err(fail) => log(
                    "Messenger remote Graph API teardown failed — proceeding with local database cleanup",
                    &fail,
                ),
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "Messenger remote Graph API teardown failed — proceeding with local database cleanup")
        }
    }
    remove_channel(state, ws, id, row.inbox_id, "messenger")
        .await
        .map_err(|e| business_error(e, MUTATING))?;
    audit_disconnect(state, ws, "Messenger", id).await;
    Ok(())
}

/// `disconnectInstagram`: a revoked token still disconnects; another Meta
/// error fails the request and keeps the channel.
async fn disconnect_instagram(state: &ApiState, ws: i64, id: i64) -> Result<(), super::OrpcError> {
    let row = find_instagram(&state.db, ws, id)
        .await
        .map_err(|_| BusinessError::not_found("Instagram channel not found"))
        .map_err(|e| business_error(e, MUTATING))?;
    let auth = &row.auth.0;
    let page_id = auth_text(auth, "/metadata/pageId").to_owned();
    let facebook = row.r#type == "facebook";
    let call = if facebook {
        let client_id = auth_text(auth, "/clientId").to_owned();
        let page = page_id.clone();
        let sibling = tx::read(&state.db, move |conn| {
            sibling_exists(conn, "IntegrationMessenger", &page, &client_id)
        })
        .await
        .map_err(|e| business_error(e, MUTATING))?;
        (!sibling).then(|| {
            let version = or_default(auth_text(auth, "/metadata/version"), MESSENGER_VERSION);
            let token = format!(
                "{}|{}",
                auth_text(auth, "/clientId"),
                auth_text(auth, "/clientSecret")
            );
            (
                format!("{GRAPH_URL}/{version}/{page_id}/subscribed_apps"),
                token,
            )
        })
    } else {
        let version = or_default(
            auth_text(auth, "/metadata/version"),
            INSTAGRAM_LOGIN_VERSION,
        );
        let token = auth_text(auth, "/tokens/accessToken").to_owned();
        Some((
            format!("{INSTAGRAM_LOGIN_URL}/{version}/{page_id}/subscribed_apps"),
            token,
        ))
    };
    if let Some((url, token)) = call {
        let result = match graph_call(Method::DELETE, Some(&token), &url, &[], None).await {
            Ok(out) if out["success"] == json!(true) => Ok(()),
            Ok(_) => Err(Box::new((
                Fail::Sdk(format!("Unsubscribe failed for page {page_id}"), 400),
                Value::Null,
            ))),
            Err(failure) => Err(failure),
        };
        if let Err(failure) = result {
            let (fail, error) = *failure;
            tracing::warn!(error = %public_message(&fail, "request failed"), "Instagram disconnect API call failed — proceeding with local cleanup");
            let revoked = error["code"].as_i64() == Some(190)
                && error["error_subcode"]
                    .as_i64()
                    .is_some_and(|c| REVOKED_TOKEN_SUBCODES.contains(&c));
            if !revoked {
                return Err(reply_error(fail, MUTATING));
            }
        }
    }
    let channel = "instagram";
    if !facebook {
        let teardown = tx::write(&state.db, move |conn| {
            Ok(
                chatbotx_db::repo::coexist_sync_run::tear_down_active_runs_for_integration(
                    conn,
                    channel,
                    id,
                    "Integration disconnected",
                )?,
            )
        });
        teardown.await.map_err(|e| business_error(e, MUTATING))?;
    }
    remove_channel(state, ws, id, row.inbox_id, channel)
        .await
        .map_err(|e| business_error(e, MUTATING))?;
    audit_disconnect(state, ws, "Instagram", id).await;
    Ok(())
}

/// The `tiktokChannels.*` / `zaloChannels.*` routes (Q.I2).
async fn handle_tiktok_zalo(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let (group, action) = route_id.split_once('.').unwrap_or_default();
    let channel = if group == "tiktokChannels" {
        "tiktok"
    } else {
        "zalo"
    };
    match action {
        "list" => {
            Input::from_query(call.params, call.query).finish()?;
            let rows = channel_integration::list(db, channel, ws, None)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            Ok(json(200, Value::Array(rows.unwrap_or_default())))
        }
        "get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.id("id");
            input.finish()?;
            let row = channel_integration::list(db, channel, ws, id)
                .await
                .and_then(|rows| {
                    rows.and_then(|mut rows| rows.pop())
                        .ok_or_else(|| BusinessError::not_found("Channel integration not found"))
                })
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, row))
        }
        "getCommentToMessage" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = input.id("id");
            input.finish()?;
            let status = channel_integration::refresh_comment_to_message(
                db,
                &HttpUpstream,
                TIKTOK_BUSINESS_API_URL,
                ws,
                id.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, json!({ "status": status })))
        }
        "updateCommentToMessage" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.id("id");
            let enabled = input.bool("enabled");
            input.finish()?;
            let status = channel_integration::set_comment_to_message(
                db,
                &HttpUpstream,
                &state.audit,
                TIKTOK_BUSINESS_API_URL,
                ws,
                id.unwrap_or_default(),
                enabled.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, json!({ "status": status })))
        }
        _ => {
            let mut input = Input::from_body(call.params, call.body);
            let id = input.id("id");
            let enabled = input.bool("enabled");
            input.finish()?;
            let at = channel_integration::update_zalo_tag_sync(
                db,
                ws,
                id.unwrap_or_default(),
                enabled.unwrap_or_default(),
            )
            .await
            .map_err(|e| business_error(e, MUTATING))?;
            Ok(json(200, json!({ "syncTagEnabledAt": opt_date(at) })))
        }
    }
}
