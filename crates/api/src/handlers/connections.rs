//! `features/connections/api/public.ts`: `connections.*` and
//! `connectionProviders.list` (P3.6-D2: the self-serve `api` and `webchat`
//! channels, the AI key integrations). Other credential connects, OAuth
//! connects and `connectSessions.*` are not ported yet: a create for such a
//! provider answers "Operation not ported yet".

use chatbotx_business::BusinessError;
use chatbotx_business::connect_session::SessionRow;
use chatbotx_business::connection::api_key;
use chatbotx_business::connection::credentials::{self, HttpUpstream};
use chatbotx_business::connection::oauth::{self, Actor, StartSession};
use chatbotx_business::connection::providers::{
    self, CHANNEL_TYPES, CONNECTION_KINDS, CONNECTION_STATUSES, INTEGRATION_TYPES,
};
use chatbotx_business::connection::{self, ConnectionRow, ListConnections};
use chatbotx_business::integration_api::{self, CreateApiChannel};
use chatbotx_business::integration_webchat;
use chatbotx_business::net::ssrf_guard::HttpDoh;
use serde_json::{Value, json};

use super::input::{Input, Num, Str, key};
use super::webchats::parse_webchat_create;
use super::{
    ApiState, Call, ErrorMap, Principal, Reply, business_error, date, id, json, opt_date, opt_id,
};
use crate::orpc::OrpcError;

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "connections.list",
    "connections.get",
    "connections.create",
    "connections.update",
    "connections.disconnect",
    "connections.reconnect",
    "connections.refresh",
    "connections.verify",
    "connectionProviders.list",
];

const IDEMPOTENCY: ErrorMap = &[
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

/// `possibleErrorsOnCreatingConnection`.
const CREATING: ErrorMap = &[
    ("connectionProviderDedicatedOnly", 403),
    ("businessError", 400),
    ("notFound", 404),
    ("channelLimitReached", 400),
    ("connectionAlreadyConnected", 409),
    ("connectionWrongStrategy", 400),
    ("connectionCredentialsRejected", 400),
    ("connectionNotOAuth", 400),
    ("connectionNotConfigured", 400),
    ("connectionProviderUnavailable", 502),
    ("channelHidden", 403),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];
/// `possibleErrorsOnReconnectingConnection`.
const RECONNECTING: ErrorMap = &[
    ("connectionProviderDedicatedOnly", 403),
    ("notFound", 404),
    ("businessError", 400),
    ("connectionNotOAuth", 400),
    ("connectionNotConfigured", 400),
    ("connectionProviderUnavailable", 502),
];
/// `possibleErrorsOnUpdatingConnection` / `...OnDisconnectingConnection`.
const UPDATING: ErrorMap = &[
    ("connectionProviderDedicatedOnly", 403),
    ("notFound", 404),
    ("businessError", 400),
];
/// `possibleErrorsOnRefreshingConnection`.
const REFRESHING: ErrorMap = &[
    ("connectionProviderDedicatedOnly", 403),
    ("notFound", 404),
    ("connectionInactive", 409),
    ("connectionNotConfigured", 400),
    ("connectionNotRefreshable", 400),
];
/// `possibleErrorsOnVerifyingConnection`.
const VERIFYING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("connectionInactive", 409),
    ("connectionNotConfigured", 400),
];

fn with_idempotency(errors: ErrorMap, error: BusinessError) -> OrpcError {
    let mut out = business_error(error, errors);
    if !out.defined
        && IDEMPOTENCY
            .iter()
            .any(|(c, s)| *c == out.code && *s == out.status)
    {
        out.defined = true;
    }
    out
}

/// `toConnectionResource`.
pub fn connection_json(row: &ConnectionRow) -> Value {
    let meta = providers::provider_meta(&row.provider);
    json!({
        "id": id(row.id),
        "kind": row.kind,
        "provider": row.provider,
        "channel": row.channel,
        "status": row.status,
        "statusReason": row.status_reason,
        "sourceId": row.source_id,
        "displayName": row.display_name,
        "inboxId": opt_id(row.inbox_id),
        "integrationId": opt_id(row.integration_id),
        "strategy": meta.map_or("self_serve", |m| m.strategy),
        "capabilities": {
            "refreshable": meta.is_some_and(|m| m.refreshable),
            "verifiable": meta.is_some_and(|m| m.verifiable),
            "multiAccount": meta.is_some_and(|m| m.multi_account),
        },
        "authExpiresAt": opt_date(row.auth_expires_at),
        "lastError": row.last_error,
        "connectedAt": opt_date(row.connected_at),
        "disconnectedAt": opt_date(row.disconnected_at),
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
    })
}

/// `toConnectSessionResource` (never `encryptedAuth`, the claims or the
/// nonce hash).
pub fn session_json(row: &SessionRow) -> Value {
    json!({
        "id": id(row.id),
        "provider": row.provider,
        "purpose": row.purpose,
        "status": row.status,
        "step": row.step,
        "nextAction": row.next_action.clone().unwrap_or(Value::Null),
        "targets": row.targets,
        "connectionIds": row.result_connection_ids,
        "errorCode": row.error_code,
        "expiresAt": date(row.expires_at),
    })
}

/// `toSessionReturn` (`sanitizeOptionalReturnUrl`): an absolute
/// `redirectUrl` on the app origin becomes the session's relative return
/// URL and its origin host; any other is ignored.
fn session_return(app_url: &str, redirect: Option<String>) -> (Option<String>, Option<String>) {
    let (Some(redirect), Ok(app)) = (redirect, url::Url::parse(app_url)) else {
        return (None, None);
    };
    match url::Url::parse(&redirect) {
        Ok(url) if url.origin() == app.origin() => {
            let mut path = url.path().to_string();
            if let Some(query) = url.query() {
                path.push('?');
                path.push_str(query);
            }
            if let Some(fragment) = url.fragment() {
                path.push('#');
                path.push_str(fragment);
            }
            (
                Some(path),
                url.host_str().map(|h| match url.port() {
                    Some(port) => format!("{h}:{port}"),
                    None => h.to_string(),
                }),
            )
        }
        _ => (None, None),
    }
}

/// `startConnect` / `startReconnect` (OAuth part): the owner's platform
/// credential (else `connectionNotConfigured`), then a `pending` session.
async fn start_oauth(
    state: &ApiState,
    principal: &Principal,
    workspace_id: i64,
    provider: &str,
    target_connection_id: Option<i64>,
    redirect: Option<String>,
) -> Result<SessionRow, BusinessError> {
    let owner = oauth::workspace_owner(&state.db, workspace_id).await?;
    let Some(encryption) = state.encryption.as_deref() else {
        return Err(oauth::not_configured(provider));
    };
    let credential =
        oauth::resolve_credential(&state.db, encryption, &state.app_url, provider, owner)
            .await?
            .ok_or_else(|| oauth::not_configured(provider))?;
    let (return_url, origin_host) = session_return(&state.app_url, redirect);
    oauth::start_session(
        &state.db,
        &credential,
        StartSession {
            workspace_id,
            provider: provider.to_string(),
            purpose: if target_connection_id.is_some() {
                "reconnect"
            } else {
                "connect"
            },
            target_connection_id,
            platform_owner_id: owner,
            origin_host,
            return_url,
            actor: super::member()
                .map(|m| Actor::User(m.user_id))
                .or(principal.token_id.map(Actor::Token)),
        },
    )
    .await
}

/// `connectionIdField` (any string; a value that is not a bigint matches no
/// row).
fn path_id(input: &mut Input) -> i64 {
    let text = input.string("id", Str::default()).unwrap_or_default();
    if super::input::is_bigint_id(&text) {
        text.parse().unwrap_or(-1)
    } else {
        -1
    }
}

/// `loadScopedConnection`.
async fn load_scoped(
    state: &ApiState,
    principal: &Principal,
    connection_id: i64,
    workspace_id: i64,
) -> Result<ConnectionRow, BusinessError> {
    let row = connection::get_for_workspace(&state.db, connection_id, workspace_id)
        .await?
        .ok_or_else(|| BusinessError::not_found("Connection not found"))?;
    providers::assert_token_scope_for_provider(principal.scopes.as_deref(), &row.provider)?;
    Ok(row)
}

/// A `ZodError` thrown by an adapter's config parse: `422
/// invalidRequestData` whose `data` is the bare issue array.
fn config_error(input: Input) -> Result<(), OrpcError> {
    input.finish().map_err(|mut error| {
        error.data = error
            .data
            .and_then(|data| data.get("issues").cloned())
            .or_else(|| Some(json!([])));
        error
    })
}

/// Runs one of [`ROUTES`].
pub async fn handle(
    state: &ApiState,
    route_id: &str,
    call: Call<'_>,
    principal: &Principal,
) -> Option<Reply> {
    let ws = call.workspace_id;
    let reply = match route_id {
        "connections.list" => list(state, call, principal).await,
        "connectionProviders.list" => list_providers(state, call, principal).await,
        "connections.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let connection_id = path_id(&mut input);
            if let Err(error) = input.finish() {
                return Some(Err(error));
            }
            load_scoped(state, principal, connection_id, ws)
                .await
                .map(|row| json(200, connection_json(&row)))
                .map_err(|e| business_error(e, super::FINDING))
        }
        "connections.create" => return create(state, call, principal).await,
        "connections.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let connection_id = path_id(&mut input);
            let display_name = input.string("displayName", Str::trimmed(1, 200));
            if let Err(error) = input.finish() {
                return Some(Err(error));
            }
            async {
                let row = load_scoped(state, principal, connection_id, ws).await?;
                providers::assert_generic_provider_allowed(&row.provider)?;
                connection::update_display_name(
                    &state.db,
                    row.id,
                    ws,
                    &display_name.unwrap_or_default(),
                )
                .await
            }
            .await
            .map(|row| json(200, connection_json(&row)))
            .map_err(|e| with_idempotency(UPDATING, e))
        }
        "connections.disconnect" | "connections.refresh" | "connections.verify" => {
            let mut input = if route_id == "connections.disconnect" {
                Input::from_query(call.params, call.query)
            } else {
                Input::from_body(call.params, call.body)
            };
            let connection_id = path_id(&mut input);
            if let Err(error) = input.finish() {
                return Some(Err(error));
            }
            let errors = match route_id {
                "connections.disconnect" => UPDATING,
                "connections.refresh" => REFRESHING,
                _ => VERIFYING,
            };
            async {
                let row = load_scoped(state, principal, connection_id, ws).await?;
                if route_id != "connections.verify" {
                    providers::assert_generic_provider_allowed(&row.provider)?;
                }
                let ai_key = credentials::ai_key_provider(&row.provider);
                let email = api_key::api_key_provider(&row.provider);
                let active = connection::is_active(&row.status);
                match (route_id, ai_key, email) {
                    ("connections.disconnect", Some(_), _)
                    | ("connections.disconnect", _, Some(_)) => {
                        credentials::disconnect_integration(&state.db, row).await
                    }
                    ("connections.disconnect", _, _) if row.provider == "telegram" => {
                        credentials::disconnect_telegram(&state.db, &HttpUpstream, row).await
                    }
                    ("connections.disconnect", _, _) => {
                        connection::disconnect(&state.db, row.id, ws).await
                    }
                    ("connections.refresh", _, _) => {
                        connection::refresh(&state.db, row.id, ws).await
                    }
                    (_, Some(provider), _) if active => {
                        credentials::verify_ai_key_connection(
                            &state.db,
                            &HttpUpstream,
                            provider,
                            row,
                        )
                        .await
                    }
                    (_, _, Some(provider)) if active => {
                        api_key::verify(&state.db, &HttpUpstream, provider, row).await
                    }
                    _ if row.provider == "telegram" && active => {
                        credentials::verify_telegram(&state.db, &HttpUpstream, row).await
                    }
                    _ => connection::verify(&state.db, row.id, ws).await,
                }
            }
            .await
            .map(|row| json(200, connection_json(&row)))
            .map_err(|e| with_idempotency(errors, e))
        }
        "connections.reconnect" => {
            let mut input = Input::from_body(call.params, call.body);
            let connection_id = path_id(&mut input);
            let redirect = input.nullish_url("redirectUrl", usize::MAX).flatten();
            if let Err(error) = input.finish() {
                return Some(Err(error));
            }
            async {
                let row = load_scoped(state, principal, connection_id, ws).await?;
                providers::assert_generic_provider_allowed(&row.provider)?;
                if oauth::adapter(&row.provider).is_none() {
                    connection::reconnect(&state.db, &row).await?;
                }
                start_oauth(state, principal, ws, &row.provider, Some(row.id), redirect).await
            }
            .await
            .map(|session| {
                json(
                    201,
                    json!({ "connection": null, "session": session_json(&session), "secret": null }),
                )
            })
            .map_err(|e| with_idempotency(RECONNECTING, e))
        }
        _ => return None,
    };
    Some(reply)
}

async fn list(state: &ApiState, call: Call<'_>, principal: &Principal) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let kind = enum_key(&mut input, "kind", CONNECTION_KINDS);
    let provider = enum_key(&mut input, "provider", INTEGRATION_TYPES);
    let channel = enum_key(&mut input, "channel", CHANNEL_TYPES);
    let status = enum_key(&mut input, "status", CONNECTION_STATUSES);
    let page = input.opt_coerce_int(
        "page",
        Num {
            min: Some(1.0),
            ..Num::default()
        },
    );
    let per_page = input.opt_coerce_int("perPage", Num::int(1.0, 50.0));
    input.finish()?;
    let kind = providers::resolve_list_kind(principal.scopes.as_deref(), kind.as_deref())
        .map_err(|e| business_error(e, super::LISTING))?;
    let per_page = per_page.unwrap_or(50);
    let (rows, count) = connection::list(
        &state.db,
        ListConnections {
            workspace_id: call.workspace_id,
            kind,
            provider,
            channel,
            status,
            page: page.unwrap_or(1),
            per_page,
        },
    )
    .await
    .map_err(|e| business_error(e, super::LISTING))?;
    let page_count = ((count + per_page - 1) / per_page).max(1);
    Ok(json(
        200,
        json!({
            "data": rows.iter().map(connection_json).collect::<Vec<_>>(),
            "pageCount": page_count,
        }),
    ))
}

fn enum_key(input: &mut Input, name: &str, options: &[&str]) -> Option<String> {
    let value = input.obj.get(name).cloned();
    input.enum_at(&key(name), value.as_ref(), options, false)
}

/// `connections.create`; `None` = a provider whose connect is not ported.
async fn create(state: &ApiState, call: Call<'_>, principal: &Principal) -> Option<Reply> {
    let mut input = Input::from_body(call.params, call.body);
    let provider = enum_key_required(&mut input, "provider", INTEGRATION_TYPES);
    let config = match input.obj.get("config") {
        None | Some(Value::Null) => None,
        Some(Value::Object(map)) => Some(map.clone()),
        Some(_) => {
            input.issue(
                &key("config"),
                "invalid_type",
                "Invalid input: expected record, received string",
            );
            None
        }
    };
    let redirect = input.nullish_url("redirectUrl", usize::MAX).flatten();
    if let Err(error) = input.finish() {
        return Some(Err(error));
    }
    let provider = provider.unwrap_or_default();
    let config = config.unwrap_or_default();
    if let Err(error) =
        providers::assert_token_scope_for_provider(principal.scopes.as_deref(), &provider)
            .and_then(|()| providers::assert_generic_provider_allowed(&provider))
    {
        return Some(Err(with_idempotency(CREATING, error)));
    }
    let Some(meta) = providers::provider_meta(&provider) else {
        return Some(Err(with_idempotency(
            CREATING,
            BusinessError::exception(
                format!("Connection provider \"{provider}\" is not configured."),
                "connectionNotConfigured",
                400,
            ),
        )));
    };
    if provider == "facebookAds" {
        return Some(Err(with_idempotency(
            CREATING,
            BusinessError::exception(
                format!(
                    "Connection provider \"{provider}\" does not support an OAuth connect flow."
                ),
                "connectionNotOAuth",
                400,
            ),
        )));
    }
    let now = chatbotx_db::repo::now_ms();
    let created = match provider.as_str() {
        "api" => {
            let mut cfg = Input::from_object(config);
            let name = cfg.string(
                "name",
                Str {
                    min: Some(1),
                    ..Str::max(40)
                },
            );
            let callback_url = cfg.nullish_url("callbackUrl", usize::MAX);
            if let Err(error) = config_error(cfg) {
                return Some(Err(error));
            }
            integration_api::create_with_token(
                &state.db,
                CreateApiChannel {
                    workspace_id: call.workspace_id,
                    actor_user_id: None,
                    name: name.unwrap_or_default(),
                    callback_url: callback_url.flatten(),
                },
                &HttpDoh,
                now,
            )
            .await
            .map(|(row, token)| {
                let detail = format!(
                    "created a new API key (#{})",
                    row.inbox_id.unwrap_or_default()
                );
                (row, Some(token), detail)
            })
        }
        "webchat" => {
            let mut cfg = Input::from_object(config);
            let data = parse_webchat_create(&mut cfg, false);
            if let Err(error) = config_error(cfg) {
                return Some(Err(error));
            }
            let Some(data) = data else {
                return Some(Err(super::internal_error("webchat config parse")));
            };
            integration_webchat::create(
                &state.db,
                call.workspace_id,
                None,
                data,
                &state.app_url,
                now,
            )
            .await
            .map(|(webchat, row)| {
                let detail = format!("connected a new Webchat channel (#{})", webchat.id);
                (row, None, detail)
            })
        }
        other => {
            let _ = meta;
            if other == "telegram" {
                return Some(
                    credentials::connect_telegram(
                        &state.db,
                        &HttpUpstream,
                        call.workspace_id,
                        &config,
                        None,
                    )
                    .await
                    .map(|row| {
                        json(
                            201,
                            json!({ "connection": connection_json(&row), "session": null, "secret": null }),
                        )
                    })
                    .map_err(|e| with_idempotency(CREATING, e)),
                );
            }
            if oauth::adapter(other).is_some() {
                return Some(
                    start_oauth(state, principal, call.workspace_id, other, None, redirect)
                        .await
                        .map(|session| {
                            json(
                                201,
                                json!({ "connection": null, "session": session_json(&session), "secret": null }),
                            )
                        })
                        .map_err(|e| with_idempotency(CREATING, e)),
                );
            }
            if let Some(provider) = api_key::api_key_provider(other) {
                return Some(
                    api_key::connect(
                        &state.db,
                        &HttpUpstream,
                        call.workspace_id,
                        provider,
                        &config,
                        None,
                    )
                    .await
                    .map(|row| {
                        json(
                            201,
                            json!({ "connection": connection_json(&row), "session": null, "secret": null }),
                        )
                    })
                    .map_err(|e| with_idempotency(CREATING, e)),
                );
            }
            let provider = credentials::ai_key_provider(other)?;
            credentials::connect_ai_key(
                &state.db,
                &HttpUpstream,
                call.workspace_id,
                provider,
                &config,
                None,
            )
            .await
            .map(|row| (row, None, String::new()))
        }
    };
    // `integrationApiService.connect` / `createWithWorkspace`: the audit
    // record of the created channel.
    let created = match created {
        Ok((row, token, detail)) => {
            // A token-driven credential connect has no actor user: no audit
            // record (`connectAndPersist`).
            if !detail.is_empty() {
                let action = if token.is_some() { "create" } else { "connect" };
                state.record_audit(action, &detail).await;
            }
            Ok((row, token))
        }
        Err(error) => Err(error),
    };
    Some(
        created
            .map(|(row, token)| {
                json(
                    201,
                    json!({
                        "connection": connection_json(&row),
                        "session": null,
                        "secret": token.map(|token| json!({ "kind": "api_channel_token", "token": token })),
                    }),
                )
            })
            .map_err(|e| with_idempotency(CREATING, e)),
    )
}

fn enum_key_required(input: &mut Input, name: &str, options: &[&str]) -> Option<String> {
    let value = input.obj.get(name).cloned();
    input.enum_at(&key(name), value.as_ref(), options, true)
}

/// The 422 of a catalog that includes the channel-kind providers: the
/// `chatbotx` adapter's name is not a `channelTypes` value, so the original
/// `toChannelType` throws a `ZodError` (a bare issue array as `data`).
fn channel_catalog_error() -> OrpcError {
    let values: Vec<String> = CHANNEL_TYPES.iter().map(|c| format!("\"{c}\"")).collect();
    OrpcError {
        code: "invalidRequestData".into(),
        status: 422,
        message: "Invalid request data".into(),
        data: Some(json!([{
            "code": "invalid_value",
            "values": CHANNEL_TYPES,
            "path": [],
            "message": format!("Invalid option: expected one of {}", values.join("|")),
        }])),
        defined: false,
    }
}

/// `connectionProviders.list` (`listConnectionProviderResources`): the
/// integration catalog with `available` / `unavailableReason`.
async fn list_providers(state: &ApiState, call: Call<'_>, principal: &Principal) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let kind = enum_key(&mut input, "kind", CONNECTION_KINDS);
    input.finish()?;
    let kind = providers::resolve_list_kind(principal.scopes.as_deref(), kind.as_deref())
        .map_err(|e| business_error(e, super::LISTING))?;
    if kind.as_deref() != Some("integration") {
        return Err(channel_catalog_error());
    }
    let ws = call.workspace_id;
    let live = credentials::live_providers(&state.db, ws)
        .await
        .map_err(|e| business_error(e, super::LISTING))?;
    let owner_id: Option<i64> = state
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "ownerId" FROM "Workspace" WHERE "id" = ?1"#,
                    [ws],
                    |r| r.get(0),
                )
                .ok())
        })
        .await
        .map_err(|e| super::internal_error(&e.to_string()))?;
    let mut data = Vec::new();
    for provider in INTEGRATION_TYPES {
        let Some(meta) = providers::provider_meta(provider) else {
            continue;
        };
        if *provider == "facebookAds" || meta.kind != "integration" {
            continue;
        }
        // No integration provider sets `multiInstance` (the snapshot: all false).
        let multi_instance = false;
        let mut reason = None;
        if !(meta.multi_account || multi_instance) && live.iter().any(|p| p == provider) {
            reason = Some("alreadyConnected");
        } else if let Some(credential_type) = credentials::credential_type(provider) {
            let found = credentials::has_platform_credential(&state.db, owner_id, credential_type)
                .await
                .map_err(|e| business_error(e, super::LISTING))?;
            if !found {
                reason = Some("credentialMissing");
            }
        }
        let fields: Vec<Value> = credentials::config_fields(provider)
            .iter()
            .map(|field| {
                json!({
                    "name": field.name,
                    "type": field.kind,
                    "required": field.required,
                    "label": field.name,
                })
            })
            .collect();
        data.push(json!({
            "provider": provider,
            "kind": meta.kind,
            "channel": null,
            "strategy": meta.strategy,
            "multiAccount": meta.multi_account,
            "multiInstance": multi_instance,
            "configFields": fields,
            "available": reason.is_none(),
            "unavailableReason": reason,
        }));
    }
    Ok(json(200, json!({ "data": data, "pageCount": 1 })))
}
