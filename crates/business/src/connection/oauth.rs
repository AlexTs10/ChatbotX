//! OAuth connects (Q.D2c): `packages/connections/src/connect-session-flow.ts`
//! (`startSession`, `completeAuthorization`, `listAndAttachCandidates`,
//! `completeReconnect`), `connect-targets.ts` (`connectTargets`), the
//! `connectAndPersist` / `subscribeWebhookBestEffort` part of `internal.ts`,
//! `apps/builder/.../resolve-connect-credential.ts` and the Messenger
//! `connection` adapter (`integrations/messenger/src/integration.ts`,
//! `apis/auth.ts`, `apis/page.ts`).
//!
//! The TikTok and Zalo adapters (single account) are in
//! [`super::oauth_channels`]. ponytail: another OAuth provider answers
//! `connectionNotConfigured` until its adapter is ported (add a match arm in
//! [`adapter`]).

use chatbotx_core::encryption::{EncryptedData, Encryption};
use chatbotx_db::Db;
use chatbotx_db::repo::partials::credential::platform_credential_aad;
use chatbotx_db::repo::partials::credential::user_credential_aad;
use chatbotx_db::repo::{self, new_id};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::{Value, json};

use super::credentials::{Upstream, UpstreamRequest};
use super::{COLUMNS, ConnectionRow, find_or_fail, providers, repo_err, row};
use super::{oauth_channels, oauth_google_ads, oauth_instagram};
use crate::connect_session::{self, NewSession, SessionRow};
use crate::{BusinessError, Result, tx};

const GRAPH: &str = "https://graph.facebook.com";
const FACEBOOK: &str = "https://www.facebook.com";
const MESSENGER_DEFAULT_VERSION: &str = "v23.0";
const MESSENGER_SCOPES: &[&str] = &[
    "email",
    "public_profile",
    "pages_manage_ads",
    "pages_manage_metadata",
    "pages_read_engagement",
    "pages_read_user_content",
    "pages_manage_posts",
    "pages_manage_engagement",
    "pages_messaging",
    "pages_show_list",
    "business_management",
    "pages_utility_messaging",
];
const ADMIN_PAGE_TASKS: &[&str] = &[
    "ADVERTISE",
    "ANALYZE",
    "CREATE_CONTENT",
    "MANAGE",
    "MODERATE",
];
const PAGE_SUBSCRIBE_FIELDS: &[&str] = &[
    "messages",
    "messaging_postbacks",
    "messaging_optins",
    "message_reads",
    "messaging_referrals",
    "message_echoes",
    "messaging_customer_information",
    "messaging_feedback",
    "messaging_policy_enforcement",
    "feed",
    "inbox_labels",
    "live_videos",
    "standby",
    "messaging_handovers",
];
const MARKETING_SUBSCRIBE_FIELDS: &[&str] = &[
    "marketing_message_echoes",
    "marketing_message_deliveries",
    "marketing_message_delivery_failed",
];
const AUTHORIZED: &[&str] = &["authorized"];
/// The `detail` of a `connected` outcome whose webhook subscription failed.
const DEGRADED_DETAIL: &str = "Connected, but the provider webhook subscription failed — the matching entry in `connections[]` shows `status: \"degraded\"`; retry from the connection's settings.";

/// An OAuth provider adapter (the `ConnectionProvider` OAuth members).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Adapter {
    pub provider: &'static str,
    /// `credentialType` (the platform credential).
    pub credential_type: &'static str,
    /// The callback path slug (`OAUTH_CALLBACK_SLUG`).
    pub slug: &'static str,
    pub multi_account: bool,
}

/// The ported OAuth adapters.
pub fn adapter(provider: &str) -> Option<Adapter> {
    match provider {
        "messenger" => Some(Adapter {
            provider: "messenger",
            credential_type: "messenger",
            slug: "messenger",
            multi_account: true,
        }),
        "tiktok" | "zalo" => Some(Adapter {
            provider: if provider == "tiktok" {
                "tiktok"
            } else {
                "zalo"
            },
            credential_type: if provider == "tiktok" {
                "tiktok"
            } else {
                "zalo"
            },
            slug: if provider == "tiktok" {
                "tiktok"
            } else {
                "zalo"
            },
            multi_account: false,
        }),
        "instagram" => Some(Adapter {
            provider: "instagram",
            credential_type: "instagram",
            slug: "instagram",
            multi_account: false,
        }),
        "instagramFacebook" => Some(Adapter {
            provider: "instagramFacebook",
            credential_type: "instagramFacebook",
            slug: "instagram-facebook",
            multi_account: true,
        }),
        "googleAds" => Some(Adapter {
            provider: "googleAds",
            credential_type: "googleAds",
            slug: "google-ads",
            multi_account: true,
        }),
        _ => None,
    }
}

/// The provider of a callback path slug (`OAUTH_CALLBACK_SLUG`; else the
/// slug is the provider).
pub fn provider_for_slug(slug: &str) -> &str {
    match slug {
        "google-ads" => "googleAds",
        "instagram-facebook" => "instagramFacebook",
        other => other,
    }
}

/// `connectionNotConfiguredException`.
pub fn not_configured(provider: &str) -> BusinessError {
    BusinessError::exception(
        format!("Connection provider \"{provider}\" is not configured."),
        "connectionNotConfigured",
        400,
    )
}

/// A provider call failure: `Rejected` (a 4xx other than 408 / 429:
/// `ConnectionProviderRejectedError`) or `Unavailable` (502 / 503:
/// `providerFailureStatus`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderFailure {
    Rejected(String),
    Unavailable(u16),
}

pub(super) async fn call(
    upstream: &impl Upstream,
    request: UpstreamRequest,
) -> std::result::Result<Value, ProviderFailure> {
    match upstream.send(request).await {
        Ok((status, body)) if (200..300).contains(&status) => {
            Ok(serde_json::from_str(&body).unwrap_or(Value::Null))
        }
        Ok((status, body)) if (400..500).contains(&status) && status != 408 && status != 429 => {
            let message = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                .unwrap_or_else(|| "The provider rejected the request.".into());
            Err(ProviderFailure::Rejected(message))
        }
        Ok((408 | 429, _)) => Err(ProviderFailure::Unavailable(503)),
        Ok(_) => Err(ProviderFailure::Unavailable(502)),
        Err(_) => Err(ProviderFailure::Unavailable(503)),
    }
}

pub(super) fn get(url: String) -> UpstreamRequest {
    UpstreamRequest {
        method: "GET",
        url,
        headers: Vec::new(),
        body: None,
    }
}

pub(super) fn query_url(base: &str, pairs: &[(&str, &str)]) -> String {
    let mut url =
        url::Url::parse(base).unwrap_or_else(|_| url::Url::parse(GRAPH).expect("graph url"));
    url.query_pairs_mut().extend_pairs(pairs.iter().copied());
    url.to_string()
}

/// The decrypted platform credential and the provider `redirect_uri`.
#[derive(Debug, Clone, PartialEq)]
pub struct OAuthCredential {
    pub config: Value,
    pub callback_url: String,
}

/// `resolveOAuthCredential` (`platformCredentialService.resolveForOwner`):
/// the owner's own row (not set to use the platform one), else the platform
/// row, decrypted with its owner-bound AAD. `None` = not configured (no
/// adapter, no row, or a row that does not decrypt).
pub async fn resolve_credential(
    db: &Db,
    encryption: &Encryption,
    app_url: &str,
    provider: &str,
    owner_id: Option<i64>,
) -> Result<Option<OAuthCredential>> {
    let Some(adapter) = adapter(provider) else {
        return Ok(None);
    };
    let kind = adapter.credential_type;
    let rows: Vec<(Option<i64>, String, bool)> = tx::read(db, move |conn| {
        let own = conn
            .query_row(
                r#"SELECT "userId", "value", "usePlatformCredential" FROM "PlatformCredential"
                   WHERE "userId" = ?1 AND "type" = ?2 LIMIT 1"#,
                params![owner_id, kind],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(repo_err)?;
        let platform = conn
            .query_row(
                r#"SELECT "userId", "value", "usePlatformCredential" FROM "PlatformCredential"
                   WHERE "userId" IS NULL AND "type" = ?1 LIMIT 1"#,
                params![kind],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(repo_err)?;
        Ok(own
            .into_iter()
            .filter(|row: &(Option<i64>, String, bool)| !row.2)
            .chain(platform)
            .collect())
    })
    .await?;
    let Some((user_id, value, _)) = rows.into_iter().next() else {
        return Ok(None);
    };
    let aad = match user_id {
        Some(user) => user_credential_aad(&user.to_string(), kind),
        None => platform_credential_aad(kind),
    };
    let config = serde_json::from_str::<EncryptedData>(&value).ok().and_then(
        |blob| match encryption.decrypt_object::<Value>(&blob, Some(&aad)) {
            Ok(config) => Some(config),
            Err(error) => {
                tracing::error!(error = %error, kind, "Failed to decrypt platform credential");
                None
            }
        },
    );
    Ok(config.map(|config| OAuthCredential {
        config,
        callback_url: format!(
            "{}/integrations/{}/callback",
            app_url.trim_end_matches('/'),
            adapter.slug
        ),
    }))
}

pub(super) fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

fn version(credential: &Value) -> String {
    credential["version"]
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or(MESSENGER_DEFAULT_VERSION)
        .to_string()
}

/// `authorizeUrl` (Messenger: `buildFacebookDialogUrl`, `authType: "rerequest"`).
fn authorize_url(provider: &str, credential: &OAuthCredential, state: &str) -> String {
    let config = &credential.config;
    if oauth_channels::handles(provider) {
        return oauth_channels::authorize_url(provider, config, &credential.callback_url, state);
    }
    if provider == oauth_google_ads::PROVIDER {
        return oauth_google_ads::authorize_url(config, &credential.callback_url, state);
    }
    if oauth_instagram::handles(provider) {
        return oauth_instagram::authorize_url(provider, config, &credential.callback_url, state);
    }
    query_url(
        &format!("{FACEBOOK}/{}/dialog/oauth", version(config)),
        &[
            ("client_id", text(config, "clientId")),
            ("redirect_uri", &credential.callback_url),
            ("scope", &MESSENGER_SCOPES.join(",")),
            ("response_type", "code"),
            ("state", state),
            ("auth_type", "rerequest"),
        ],
    )
}

/// `oauth2Auth(config, callbackUrl, tokens, metadata)`.
pub(super) fn oauth2_auth(
    config: &Value,
    callback_url: &str,
    tokens: Value,
    metadata: Option<Value>,
) -> Value {
    let mut auth = json!({
        "authType": "oauth2",
        "clientId": config["clientId"],
        "clientSecret": config["clientSecret"],
        "redirectUrl": callback_url,
        "tokens": tokens,
    });
    for key in ["verifyToken", "version"] {
        if let Some(value) = config[key].as_str().filter(|v| !v.is_empty()) {
            auth[key] = json!(value);
        }
    }
    if let Some(metadata) = metadata {
        auth["metadata"] = metadata;
    }
    auth
}

/// `exchangeCode` (Messenger: the short-lived token, then the long-lived one).
async fn exchange_code(
    upstream: &impl Upstream,
    provider: &str,
    credential: &OAuthCredential,
    code: &str,
) -> std::result::Result<Value, ProviderFailure> {
    let config = &credential.config;
    if oauth_channels::handles(provider) {
        return oauth_channels::exchange_code(
            upstream,
            provider,
            config,
            &credential.callback_url,
            code,
        )
        .await;
    }
    if provider == oauth_google_ads::PROVIDER {
        return oauth_google_ads::exchange_code(upstream, config, &credential.callback_url, code)
            .await;
    }
    if oauth_instagram::handles(provider) {
        return oauth_instagram::exchange_code(
            upstream,
            provider,
            config,
            &credential.callback_url,
            code,
        )
        .await;
    }
    let endpoint = format!("{GRAPH}/{}/oauth/access_token", version(config));
    let short = call(
        upstream,
        get(query_url(
            &endpoint,
            &[
                ("client_id", text(config, "clientId")),
                ("client_secret", text(config, "clientSecret")),
                ("redirect_uri", &credential.callback_url),
                ("code", code),
            ],
        )),
    )
    .await?;
    let long = call(
        upstream,
        get(query_url(
            &endpoint,
            &[
                ("grant_type", "fb_exchange_token"),
                ("client_id", text(config, "clientId")),
                ("client_secret", text(config, "clientSecret")),
                ("fb_exchange_token", text(&short, "access_token")),
            ],
        )),
    )
    .await?;
    Ok(oauth2_auth(
        config,
        &credential.callback_url,
        json!({ "accessToken": text(&long, "access_token") }),
        None,
    ))
}

/// One connectable account of a grant (`ConnectionCandidate`).
fn candidate(source_id: &str, display_name: &str, auth: Value) -> Value {
    json!({ "sourceId": source_id, "displayName": display_name, "auth": auth })
}

/// `listCandidates` (`getUserPages`, direct pages only, as the original:
/// the Business Manager lookup is disabled there): pages with a token and
/// every admin task, each with its own page token.
async fn list_candidates(
    upstream: &impl Upstream,
    provider: &str,
    auth: &Value,
) -> std::result::Result<Vec<Value>, ProviderFailure> {
    // A single-account provider's grant is its one `describe` candidate.
    if oauth_channels::handles(provider) {
        return Ok(oauth_channels::describe(provider, auth)
            .map(|(id, name)| vec![candidate(&id, &name, auth.clone())])
            .unwrap_or_default());
    }
    if provider == oauth_google_ads::PROVIDER {
        return oauth_google_ads::list_candidates(upstream, auth).await;
    }
    // Instagram login: the grant is its one account.
    if provider == "instagram" {
        return Ok(oauth_instagram::describe(auth)
            .map(|(id, name)| vec![candidate(&id, &name, auth.clone())])
            .unwrap_or_default());
    }
    if provider == "instagramFacebook" {
        return Ok(oauth_instagram::list_facebook_candidates(upstream, auth)
            .await?
            .into_iter()
            .map(|(id, name, auth)| candidate(&id, &name, auth))
            .collect());
    }
    if auth["authType"] != "oauth2" {
        return Ok(Vec::new());
    }
    let version = auth["version"]
        .as_str()
        .unwrap_or(MESSENGER_DEFAULT_VERSION)
        .to_string();
    let token = auth["tokens"]["accessToken"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let endpoint = format!("{GRAPH}/{version}/me/accounts");
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    // ponytail: one page size (50, up to 2000 rows); the original retries a
    // "data too large" answer with 25 and 10.
    for _ in 0..40 {
        let mut pairs = vec![
            ("fields", "id,name,access_token,category,tasks".to_string()),
            ("access_token", token.clone()),
            ("limit", "50".to_string()),
        ];
        if let Some(cursor) = &after {
            pairs.push(("after", cursor.clone()));
        }
        let pairs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let page = call(upstream, get(query_url(&endpoint, &pairs))).await?;
        for item in page["data"].as_array().into_iter().flatten() {
            let tasks: Vec<&str> = item["tasks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let page_token = text(item, "access_token");
            if page_token.is_empty() || !ADMIN_PAGE_TASKS.iter().all(|t| tasks.contains(t)) {
                continue;
            }
            let (id, name) = (text(item, "id"), text(item, "name"));
            let mut config = auth.clone();
            config["version"] = json!(version);
            let page_auth = oauth2_auth(
                &config,
                text(auth, "redirectUrl"),
                json!({ "accessToken": page_token }),
                Some(json!({ "pageId": id, "pageName": name, "version": version })),
            );
            out.push(candidate(id, name, page_auth));
        }
        after = page["paging"]["next"]
            .as_str()
            .and_then(|_| page["paging"]["cursors"]["after"].as_str())
            .map(str::to_string);
        if after.is_none() {
            break;
        }
    }
    Ok(out)
}

/// `webhook.subscribe` (`subscribePageToAppWebhook`): the base fields plus
/// the Marketing Messages ones, retried once with the base fields only.
pub async fn subscribe_webhook(
    upstream: &impl Upstream,
    auth: &Value,
) -> std::result::Result<(), ProviderFailure> {
    subscribe_webhook_fields(upstream, auth, &[]).await
}

/// [`subscribe_webhook`] with `extra` base fields (`LEAD_ADS_PAGE_SUBSCRIBE_FIELDS` = `["leadgen"]`).
pub async fn subscribe_webhook_fields(
    upstream: &impl Upstream,
    auth: &Value,
    extra: &[&str],
) -> std::result::Result<(), ProviderFailure> {
    let version = auth["metadata"]["version"]
        .as_str()
        .unwrap_or(MESSENGER_DEFAULT_VERSION);
    let token = auth["tokens"]["accessToken"].as_str().unwrap_or_default();
    let post = |fields: String| UpstreamRequest {
        method: "POST",
        url: format!("{GRAPH}/{version}/me/subscribed_apps"),
        headers: vec![("authorization".into(), format!("Bearer {token}"))],
        body: Some(json!({ "subscribed_fields": fields }).to_string()),
    };
    let base = [PAGE_SUBSCRIBE_FIELDS, extra].concat().join(",");
    let all = [PAGE_SUBSCRIBE_FIELDS, extra, MARKETING_SUBSCRIBE_FIELDS]
        .concat()
        .join(",");
    if call(upstream, post(all)).await.is_ok() {
        return Ok(());
    }
    tracing::warn!(
        "Page subscription with marketing message fields failed; retrying with base fields"
    );
    call(upstream, post(base)).await.map(|_| ())
}

/// `describe`: the page id and name of a page auth.
fn describe(auth: &Value) -> Result<(String, String)> {
    let page_id = auth["metadata"]["pageId"].as_str().unwrap_or_default();
    if page_id.is_empty() {
        return Err(BusinessError::Repo(repo::Error::msg(
            "Messenger auth has no page identity",
        )));
    }
    let name = auth["metadata"]["pageName"].as_str().unwrap_or("Messenger");
    Ok((page_id.to_string(), name.to_string()))
}

/// The session's actor (`ConnectSessionActor`): a builder user or a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    User(i64),
    Token(i64),
}

/// The input of [`start_session`].
#[derive(Debug, Clone)]
pub struct StartSession {
    pub workspace_id: i64,
    pub provider: String,
    pub purpose: &'static str,
    pub target_connection_id: Option<i64>,
    pub platform_owner_id: Option<i64>,
    pub origin_host: Option<String>,
    pub return_url: Option<String>,
    pub actor: Option<Actor>,
}

/// `startSession` (and the `reconnect` start): the `pending` session whose
/// `nextAction` opens the provider dialog with `state =
/// "{sessionId}.{nonce}"`.
pub async fn start_session(
    db: &Db,
    credential: &OAuthCredential,
    input: StartSession,
) -> Result<SessionRow> {
    adapter(&input.provider).ok_or_else(|| not_configured(&input.provider))?;
    if let Some(target) = input.target_connection_id {
        super::get_for_workspace(db, target, input.workspace_id)
            .await?
            .ok_or_else(|| BusinessError::not_found("Connection not found"))?;
    }
    let credential = credential.clone();
    let provider = input.provider.clone();
    connect_session::create(
        db,
        NewSession {
            workspace_id: input.workspace_id,
            provider: input.provider,
            purpose: input.purpose.to_string(),
            target_connection_id: input.target_connection_id,
            actor_user_id: match input.actor {
                Some(Actor::User(id)) => Some(id),
                _ => None,
            },
            actor_token_id: match input.actor {
                Some(Actor::Token(id)) => Some(id),
                _ => None,
            },
            platform_owner_id: input.platform_owner_id.map(|id| id.to_string()),
            origin_host: input.origin_host,
            return_url: input.return_url,
        },
        move |id, nonce| json!({ "type": "open_url", "url": authorize_url(&provider, &credential, &format!("{id}.{nonce}")) }),
    )
    .await
}

fn aad_candidates(session_id: i64) -> String {
    format!("connect-session:{session_id}")
}

fn aad_authorization(session_id: i64) -> String {
    format!("connect-session:{session_id}:authorization")
}

fn encrypt(encryption: &Encryption, value: &Value, aad: &str) -> Result<String> {
    encryption
        .encrypt_object(value, Some(aad))
        .map_err(|e| BusinessError::Repo(repo::Error::msg(e.to_string())))
        .and_then(|blob| {
            serde_json::to_string(&blob)
                .map_err(|e| BusinessError::Repo(repo::Error::msg(e.to_string())))
        })
}

fn decrypt(encryption: &Encryption, text: &str, aad: &str) -> Result<Value> {
    let blob: EncryptedData = serde_json::from_str(text)
        .map_err(|e| BusinessError::Repo(repo::Error::msg(e.to_string())))?;
    encryption
        .decrypt_object(&blob, Some(aad))
        .map_err(|e| BusinessError::Repo(repo::Error::msg(e.to_string())))
}

fn credentials_rejected(message: &str) -> BusinessError {
    BusinessError::exception(message, "connectionCredentialsRejected", 400)
}

fn provider_unavailable(status: u16) -> BusinessError {
    BusinessError::exception(
        "The provider is temporarily unavailable. Please try again.",
        "connectionProviderUnavailable",
        status,
    )
}

/// `connectionStateMismatchException`.
pub fn state_mismatch() -> BusinessError {
    BusinessError::exception(
        "This connect session could not be verified. Please start the connection again.",
        "connectionStateMismatch",
        400,
    )
}

/// `completeAuthorization`: resolves the session by its nonce, claims it,
/// exchanges `code`, then lists and attaches the candidates (a reconnect
/// session completes the reconnect instead).
pub async fn complete_authorization(
    db: &Db,
    encryption: &Encryption,
    upstream: &impl Upstream,
    session_id: i64,
    nonce: &str,
    code: &str,
    credential: &OAuthCredential,
) -> Result<SessionRow> {
    let session = connect_session::find_by_nonce(db, nonce)
        .await?
        .filter(|row| row.id == session_id)
        .ok_or_else(state_mismatch)?;
    adapter(&session.provider).ok_or_else(|| not_configured(&session.provider))?;
    if session.status == "authorized"
        && let Some(text) = &session.encrypted_auth
    {
        let auth = decrypt(encryption, text, &aad_authorization(session.id))?;
        return list_and_attach(db, encryption, upstream, &session, &auth).await;
    }
    if session.status != "pending" {
        return Err(connect_session::expired(
            "This connect session is no longer active.",
        ));
    }
    connect_session::claim_authorization(db, session.id, session.workspace_id).await?;
    let auth = match exchange_code(upstream, &session.provider, credential, code).await {
        Ok(auth) => auth,
        Err(ProviderFailure::Unavailable(status)) => {
            tracing::warn!(
                session_id = session.id,
                "connection OAuth: authorization code exchange failed"
            );
            connect_session::release_authorization(db, session.id, session.workspace_id).await?;
            return Err(provider_unavailable(status));
        }
        Err(ProviderFailure::Rejected(_)) => {
            connect_session::fail(db, &session, "exchange_failed", Some(AUTHORIZED)).await;
            return Err(credentials_rejected(
                "The provider rejected the authorization.",
            ));
        }
    };
    if session.purpose == "reconnect" && session.target_connection_id.is_some() {
        return complete_reconnect(db, upstream, &session, &auth).await;
    }
    let stored = match encrypt(encryption, &auth, &aad_authorization(session.id)) {
        Ok(text) => {
            connect_session::store_authorization(db, session.id, session.workspace_id, text).await
        }
        Err(error) => Err(error),
    };
    let stored = match stored {
        Ok(row) => row,
        Err(error) => {
            connect_session::fail(db, &session, "internal_error", Some(AUTHORIZED)).await;
            return Err(error);
        }
    };
    list_and_attach(db, encryption, upstream, &stored, &auth).await
}

/// The workspace (if any) where `(provider, sourceId)` has an active
/// connection (`findByProviderAndSourceIdsAnyWorkspace`, first row).
/// `instagram` and `instagramFacebook` share the `igId` identity: an
/// account connected under the sibling provider is not selectable either
/// (`findActiveWorkspacesByIgIds`).
fn active_workspace_of(conn: &Connection, provider: &str, source_id: &str) -> Result<Option<i64>> {
    let sibling = match provider {
        "instagram" => "instagramFacebook",
        "instagramFacebook" => "instagram",
        other => other,
    };
    conn.query_row(
        r#"SELECT "workspaceId" FROM "Connection" WHERE "provider" IN (?1, ?3) AND "sourceId" = ?2
               AND "status" IN ('connected', 'degraded')
           ORDER BY "id" LIMIT 1"#,
        params![provider, source_id, sibling],
        |r| r.get(0),
    )
    .optional()
    .map_err(repo_err)
}

/// `listAndAttachCandidates`: the candidate list (encrypted, bound to the
/// session) and the public targets (an account connected anywhere is not
/// selectable).
pub async fn list_and_attach(
    db: &Db,
    encryption: &Encryption,
    upstream: &impl Upstream,
    session: &SessionRow,
    auth: &Value,
) -> Result<SessionRow> {
    let candidates = match list_candidates(upstream, &session.provider, auth).await {
        Ok(list) => list,
        Err(ProviderFailure::Unavailable(status)) => return Err(provider_unavailable(status)),
        Err(ProviderFailure::Rejected(_)) => {
            connect_session::fail(db, session, "provider_error", None).await;
            return Err(credentials_rejected("Failed to list accounts."));
        }
    };
    if candidates.is_empty() {
        connect_session::fail(db, session, "no_candidates", None).await;
        return Err(BusinessError::exception(
            "No connectable accounts were found for this authorization.",
            "connectionNoCandidates",
            400,
        ));
    }
    let provider = session.provider.clone();
    let workspace_id = session.workspace_id;
    let list = candidates.clone();
    let targets = tx::read(db, move |conn| {
        let mut targets = Vec::new();
        for candidate in &list {
            let (id, name) = (text(candidate, "sourceId"), text(candidate, "displayName"));
            match active_workspace_of(conn, &provider, id)? {
                Some(ws) => {
                    let scope = if ws == workspace_id { "this_workspace" } else { "other_workspace" };
                    targets.push(json!({ "id": id, "name": name, "selectable": false, "alreadyConnected": scope }));
                }
                None => targets.push(json!({ "id": id, "name": name, "selectable": true })),
            }
        }
        Ok(Value::Array(targets))
    })
    .await?;
    let encrypted = encrypt(
        encryption,
        &Value::Array(candidates),
        &aad_candidates(session.id),
    )?;
    connect_session::attach_authorization(db, session.id, session.workspace_id, encrypted, targets)
        .await
}

/// `tryReuseFacebookSsoToken`: the user's Facebook sign-in token when
/// Meta's `/debug_token` (app token `clientId|clientSecret`) says it is
/// valid and carries every Messenger scope but `email` / `public_profile`.
/// Any failure is `None` (the normal OAuth dialog runs).
pub async fn facebook_sso_token(
    db: &Db,
    upstream: &impl Upstream,
    user_id: i64,
    credential: &OAuthCredential,
) -> Option<String> {
    let token: Option<String> = tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT "accessToken" FROM "Account" WHERE "userId" = ?1 AND "providerId" = 'facebook'
               ORDER BY "id" LIMIT 1"#,
            params![user_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(repo_err)
    })
    .await
    .ok()
    .flatten()
    .filter(|t| !t.is_empty());
    let token = token?;
    let config = &credential.config;
    let app_token = format!("{}|{}", text(config, "clientId"), text(config, "clientSecret"));
    let debug = call(
        upstream,
        get(query_url(
            &format!("{GRAPH}/{}/debug_token", version(config)),
            &[("input_token", &token), ("access_token", &app_token)],
        )),
    )
    .await
    .ok()?;
    let data = &debug["data"];
    let scopes: Vec<&str> = data["scopes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let reusable = data["is_valid"] == true
        && MESSENGER_SCOPES
            .iter()
            .filter(|s| !matches!(**s, "email" | "public_profile"))
            .all(|s| scopes.contains(s));
    reusable.then_some(token)
}

/// `tryMessengerSsoReuse`: with a reusable Facebook sign-in token, a
/// `connect` session goes straight to the page list (no OAuth dialog).
/// `None` = the dialog runs (a failed attempt fails its session).
pub async fn start_messenger_with_sso(
    db: &Db,
    encryption: &Encryption,
    upstream: &impl Upstream,
    credential: &OAuthCredential,
    workspace_id: i64,
    user_id: i64,
    platform_owner_id: i64,
) -> Option<SessionRow> {
    let token = facebook_sso_token(db, upstream, user_id, credential).await?;
    let session = match connect_session::create(
        db,
        NewSession {
            workspace_id,
            provider: "messenger".into(),
            purpose: "connect".into(),
            target_connection_id: None,
            actor_user_id: Some(user_id),
            actor_token_id: None,
            platform_owner_id: Some(platform_owner_id.to_string()),
            origin_host: None,
            return_url: None,
        },
        |_, _| Value::Null,
    )
    .await
    {
        Ok(session) => session,
        Err(error) => {
            tracing::error!(error = %error, workspace_id, "Facebook SSO reuse failed for messenger connect; falling back to OAuth");
            return None;
        }
    };
    let auth = oauth2_auth(
        &credential.config,
        &credential.callback_url,
        json!({ "accessToken": token }),
        None,
    );
    match list_and_attach(db, encryption, upstream, &session, &auth).await {
        Ok(session) => Some(session),
        Err(error) => {
            tracing::error!(error = %error, session_id = session.id, "Facebook SSO reuse failed for messenger connect; falling back to OAuth");
            connect_session::fail(db, &session, "internal_error", None).await;
            None
        }
    }
}

/// The `Connection` of `(workspace, provider, sourceId)`.
fn find_by_source(
    conn: &Connection,
    workspace_id: i64,
    provider: &str,
    source_id: &str,
) -> Result<Option<ConnectionRow>> {
    conn.query_row(
        &format!(
            r#"SELECT {COLUMNS} FROM "Connection"
               WHERE "workspaceId" = ?1 AND "provider" = ?2 AND "sourceId" = ?3"#
        ),
        params![workspace_id, provider, source_id],
        row,
    )
    .optional()
    .map_err(repo_err)
}

fn already_connected() -> BusinessError {
    super::credentials::already_connected()
}

fn unique_violation(error: &chatbotx_db::rusqlite::Error) -> bool {
    error.to_string().contains("UNIQUE constraint failed")
}

/// `connectAndPersist` of a Messenger page: a new `Inbox` (sourceId = page
/// id), the `IntegrationMessenger` row and the `connected` `Connection` (an
/// inactive row of the same page is revived onto the new inbox). A page
/// bound elsewhere (unique violation) is `connectionAlreadyConnected`.
fn persist_messenger(
    conn: &Connection,
    workspace_id: i64,
    auth: &Value,
    actor_user_id: Option<i64>,
    now: i64,
) -> Result<ConnectionRow> {
    let (page_id, name) = describe(auth)?;
    let (existing, inbox_id) =
        new_channel_inbox(conn, workspace_id, "messenger", &page_id, &name, now)?;
    conn.execute(
        r#"INSERT INTO "IntegrationMessenger" ("id", "createdAt", "updatedAt", "auth", "pageId",
               "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, '[]', '[]', '[]', ?6, ?7)"#,
        params![new_id(), now, auth.to_string(), page_id, name, workspace_id, inbox_id],
    )
    .map_err(unique_or_repo)?;
    upsert_channel_connection(
        conn,
        workspace_id,
        "messenger",
        existing,
        inbox_id,
        &page_id,
        &name,
        actor_user_id,
        now,
    )
}

/// A unique violation is `connectionAlreadyConnected`.
pub(super) fn unique_or_repo(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    if unique_violation(&error) {
        already_connected()
    } else {
        repo_err(error)
    }
}

/// The first step of a channel `connectAndPersist`: refuses an active
/// connection of `(workspace, provider, sourceId)` and creates the new
/// `Inbox`. Returns the inactive connection to revive (if any) and the
/// inbox id.
pub(super) fn new_channel_inbox(
    conn: &Connection,
    workspace_id: i64,
    provider: &str,
    source_id: &str,
    name: &str,
    now: i64,
) -> Result<(Option<ConnectionRow>, i64)> {
    let existing = find_by_source(conn, workspace_id, provider, source_id)?;
    if existing
        .as_ref()
        .is_some_and(|row| super::is_active(&row.status))
    {
        return Err(already_connected());
    }
    let inbox_id = new_id();
    conn.execute(
        r#"INSERT INTO "Inbox" ("id", "createdAt", "updatedAt", "name", "channel", "sourceId",
               "workspaceId", "status")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, 'connected')"#,
        params![
            inbox_id,
            now,
            name,
            providers::to_channel_type(provider),
            source_id,
            workspace_id
        ],
    )
    .map_err(unique_or_repo)?;
    Ok((existing, inbox_id))
}

/// The `connected` `Connection` of a channel connect (an inactive row of
/// the same account is revived onto the new inbox).
#[allow(clippy::too_many_arguments)]
pub(super) fn upsert_channel_connection(
    conn: &Connection,
    workspace_id: i64,
    provider: &str,
    existing: Option<ConnectionRow>,
    inbox_id: i64,
    source_id: &str,
    name: &str,
    actor_user_id: Option<i64>,
    now: i64,
) -> Result<ConnectionRow> {
    let id = match existing {
        Some(existing) => {
            conn.execute(
                r#"UPDATE "Connection" SET "inboxId" = ?3, "displayName" = ?4, "status" = 'connected',
                       "statusReason" = NULL, "lastError" = NULL, "connectedAt" = ?5,
                       "disconnectedAt" = NULL, "updatedAt" = ?5
                   WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![existing.id, workspace_id, inbox_id, name, now],
            )
            .map_err(repo_err)?;
            existing.id
        }
        None => {
            let id = new_id();
            conn.execute(
                r#"INSERT INTO "Connection" ("id", "createdAt", "updatedAt", "workspaceId", "provider",
                       "kind", "channel", "inboxId", "sourceId", "displayName", "status", "createdBy",
                       "connectedAt")
                   VALUES (?1, ?2, ?2, ?3, ?4, 'channel', ?5, ?6, ?7, ?8, 'connected', ?9, ?2)"#,
                params![id, now, workspace_id, provider, providers::to_channel_type(provider), inbox_id, source_id, name, actor_user_id],
            )
            .map_err(unique_or_repo)?;
            id
        }
    };
    find_or_fail(conn, id, workspace_id)
}

/// `subscribeWebhookBestEffort`: a failed subscription marks the
/// connection `degraded` (`verify.failed_non_auth`, `verify_failed`).
async fn subscribe_best_effort(
    db: &Db,
    upstream: &impl Upstream,
    auth: &Value,
    connection: ConnectionRow,
) -> Result<ConnectionRow> {
    // TikTok / Zalo webhooks are set in the developer portal (no `webhook`).
    let subscribed = match connection.provider.as_str() {
        "messenger" => subscribe_webhook(upstream, auth).await.is_ok(),
        provider if oauth_instagram::handles(provider) => {
            oauth_instagram::subscribe_webhook(upstream, provider, auth)
                .await
                .is_ok()
        }
        _ => true,
    };
    if subscribed {
        return Ok(connection);
    }
    tracing::warn!(
        connection_id = connection.id,
        "connect: webhook subscribe failed, marking connection degraded"
    );
    tx::write(db, move |conn| {
        conn.execute(
            r#"UPDATE "Connection" SET "status" = 'degraded', "statusReason" = 'verify_failed',
                   "updatedAt" = ?3 WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![connection.id, connection.workspace_id, repo::now_ms()],
        )
        .map_err(repo_err)?;
        find_or_fail(conn, connection.id, connection.workspace_id)
    })
    .await
}

/// `completeReconnect`: the re-granted page must be the connection's own
/// page; its new token is saved and the connection is `connected` again.
async fn complete_reconnect(
    db: &Db,
    upstream: &impl Upstream,
    session: &SessionRow,
    auth: &Value,
) -> Result<SessionRow> {
    let target = session.target_connection_id.unwrap_or_default();
    let Some(connection) = super::get_for_workspace(db, target, session.workspace_id).await? else {
        connect_session::fail(db, session, "internal_error", None).await;
        return Err(BusinessError::not_found("Connection not found"));
    };
    let candidates = match list_candidates(upstream, &session.provider, auth).await {
        Ok(list) => list,
        Err(_) => {
            connect_session::fail(db, session, "internal_error", Some(AUTHORIZED)).await;
            return Err(credentials_rejected("Failed to list accounts."));
        }
    };
    let legacy = connection.source_id.starts_with("legacy:");
    let found = if legacy {
        (candidates.len() == 1).then(|| candidates[0].clone())
    } else {
        candidates
            .into_iter()
            .find(|c| text(c, "sourceId") == connection.source_id)
    };
    let Some(candidate) = found else {
        connect_session::fail(db, session, "provider_denied", Some(AUTHORIZED)).await;
        return Err(BusinessError::exception(
            "The reauthorized account does not match the connection being reconnected.",
            "connectionIdentityMismatch",
            400,
        ));
    };
    let page_auth = candidate["auth"].clone();
    let provider = session.provider.clone();
    let (page_id, _) = if oauth_channels::handles(&provider) {
        oauth_channels::describe(&provider, &page_auth)?
    } else if provider == oauth_google_ads::PROVIDER {
        oauth_google_ads::describe(&page_auth)?
    } else {
        describe(&page_auth)?
    };
    let table = oauth_channels::satellite(&provider).map_or("IntegrationMessenger", |(t, _)| t);
    let row = session.clone();
    let saved_auth = page_auth.clone();
    let result = tx::write(db, move |conn| {
        let now = repo::now_ms();
        let updated = if provider == oauth_google_ads::PROVIDER {
            usize::from(oauth_google_ads::save_reconnect_auth(
                conn,
                connection.integration_id,
                connection.workspace_id,
                &saved_auth,
                now,
            )?)
        } else {
            conn.execute(
                &format!(
                    r#"UPDATE "{table}" SET "auth" = ?3, "tokenRefreshError" = NULL, "updatedAt" = ?4
                       WHERE "inboxId" = ?1 AND "workspaceId" = ?2"#
                ),
                params![connection.inbox_id, connection.workspace_id, saved_auth.to_string(), now],
            )
            .map_err(repo_err)?
        };
        if updated == 0 {
            return Err(not_configured(&provider));
        }
        conn.execute(
            r#"UPDATE "Connection" SET "sourceId" = ?3, "lastError" = NULL, "status" = 'connected',
                   "statusReason" = NULL, "connectedAt" = ?4, "disconnectedAt" = NULL, "updatedAt" = ?4
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![connection.id, connection.workspace_id, page_id, now],
        )
        .map_err(repo_err)?;
        let transitioned = find_or_fail(conn, connection.id, connection.workspace_id)?;
        let done = connect_session::complete_reconnect(
            conn,
            &row,
            json!({ "targetId": page_id, "status": "connected", "connectionId": connection.id.to_string() }),
            connection.id,
            now,
        )?;
        Ok((done, transitioned))
    })
    .await;
    match result {
        Ok((done, transitioned)) => {
            subscribe_best_effort(db, upstream, &page_auth, transitioned).await?;
            Ok(done)
        }
        Err(error) => {
            connect_session::fail(db, session, "internal_error", Some(AUTHORIZED)).await;
            Err(error)
        }
    }
}

/// The result of [`connect_targets`].
#[derive(Debug, Clone)]
pub struct TargetsResult {
    pub session: SessionRow,
    pub connections: Vec<ConnectionRow>,
    pub outcomes: Vec<Value>,
}

/// `connectTargets`: each requested target is leased, connected and
/// reported (`connected` / `duplicated` / `failed`); the outcomes are
/// merged into the session.
pub async fn connect_targets(
    db: &Db,
    encryption: &Encryption,
    upstream: &impl Upstream,
    session_id: i64,
    workspace_id: i64,
    target_ids: &[String],
    actor_user_id: Option<i64>,
) -> Result<TargetsResult> {
    let session = connect_session::find_for_workspace(db, session_id, workspace_id)
        .await?
        .ok_or_else(|| BusinessError::not_found("Connect session not found"))?;
    let Some(encrypted) = session
        .encrypted_auth
        .clone()
        .filter(|_| session.status == "awaiting_selection")
    else {
        return Err(connect_session::expired(
            "This connect session is not awaiting target selection.",
        ));
    };
    let candidates = decrypt(encryption, &encrypted, &aad_candidates(session.id))?;
    let candidates = candidates.as_array().cloned().unwrap_or_default();
    let targets = session.targets.as_array().cloned().unwrap_or_default();
    let mut outcomes = Vec::new();
    let mut connections = Vec::new();
    for target_id in target_ids {
        let target = targets.iter().find(|t| text(t, "id") == target_id);
        let candidate = candidates.iter().find(|c| text(c, "sourceId") == target_id);
        let (Some(target), Some(candidate)) = (target, candidate) else {
            outcomes
                .push(json!({ "targetId": target_id, "status": "failed", "reason": "unknown" }));
            continue;
        };
        if target["selectable"] != true {
            outcomes.push(if target["alreadyConnected"].is_string() {
                json!({ "targetId": target_id, "status": "duplicated", "reason": "alreadyConnected" })
            } else {
                json!({ "targetId": target_id, "status": "failed", "reason": "notSelectable" })
            });
            continue;
        }
        let owner = new_id().to_string();
        if !connect_session::claim_target(
            db,
            session.id,
            workspace_id,
            target_id.clone(),
            owner.clone(),
        )
        .await?
        {
            let (provider, source) = (session.provider.clone(), target_id.clone());
            let active = tx::read(db, move |conn| {
                Ok(find_by_source(conn, workspace_id, &provider, &source)?
                    .is_some_and(|row| super::is_active(&row.status)))
            })
            .await?;
            outcomes.push(if active {
                json!({ "targetId": target_id, "status": "duplicated", "reason": "alreadyConnected" })
            } else {
                json!({ "targetId": target_id, "status": "failed", "reason": "inProgress" })
            });
            continue;
        }
        let auth = candidate["auth"].clone();
        let saved = auth.clone();
        let provider = session.provider.clone();
        let persisted = tx::write(db, move |conn| {
            let now = repo::now_ms();
            if oauth_channels::handles(&provider) {
                oauth_channels::persist(conn, &provider, workspace_id, &saved, actor_user_id, now)
            } else if provider == oauth_google_ads::PROVIDER {
                oauth_google_ads::persist(conn, workspace_id, &saved, actor_user_id, now)
            } else if oauth_instagram::handles(&provider) {
                oauth_instagram::persist(conn, &provider, workspace_id, &saved, actor_user_id, now)
            } else {
                persist_messenger(conn, workspace_id, &saved, actor_user_id, now)
            }
        })
        .await;
        let outcome = match persisted {
            Ok(connection) => match subscribe_best_effort(db, upstream, &auth, connection).await {
                Ok(connection) => {
                    let mut outcome = json!({ "targetId": target_id, "status": "connected", "connectionId": connection.id.to_string() });
                    if connection.status == "degraded" {
                        outcome["detail"] = json!(DEGRADED_DETAIL);
                    }
                    connections.push(connection);
                    outcome
                }
                Err(error) => failure_outcome(target_id, &error),
            },
            Err(error) => failure_outcome(target_id, &error),
        };
        outcomes.push(outcome);
        connect_session::release_target(db, session.id, workspace_id, target_id.clone(), owner)
            .await?;
    }
    let durable: Vec<Value> = outcomes
        .iter()
        .filter(|o| o["reason"] != "inProgress")
        .cloned()
        .collect();
    let ids = connections.iter().map(|c| c.id).collect();
    let session =
        connect_session::record_results(db, session.id, workspace_id, durable, ids).await?;
    Ok(TargetsResult {
        session,
        connections,
        outcomes,
    })
}

/// `toFailureOutcome`.
fn failure_outcome(target_id: &str, error: &BusinessError) -> Value {
    match error.code() {
        Some("channelLimitReached") => {
            json!({ "targetId": target_id, "status": "limitReached", "reason": "workspaceLimit" })
        }
        Some("connectionAlreadyConnected") => {
            json!({ "targetId": target_id, "status": "duplicated", "reason": "alreadyConnected" })
        }
        _ => {
            tracing::error!(error = %error, target_id, "connectTargets: candidate connect failed internally");
            json!({ "targetId": target_id, "status": "failed", "reason": "internalError", "detail": "Connect failed" })
        }
    }
}

/// The workspace owner (`resolveOwnerForWorkspace`).
pub async fn workspace_owner(db: &Db, workspace_id: i64) -> Result<Option<i64>> {
    tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT "ownerId" FROM "Workspace" WHERE "id" = ?1"#,
            [workspace_id],
            |r| r.get(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(repo_err)
    })
    .await
}
