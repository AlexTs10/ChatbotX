//! `features/platform-credentials` (Q.D11 slice 7): the credential settings
//! actions (`POST /rpc/platformCredentialActions/<name>`, `authActionClient`)
//! and `platformCredentialsAPI.getGiphyApiKeyAPI`.
//!
//! The bound `credentialScopeSchema` arg is the input field `scope`
//! (`"user"` | `"platform"`). Server-side messages are the English
//! `messages/en.json` text. `updateStripeSettings` / `deleteStripeSettings`
//! are `savePlatformStripe` / `removePlatformStripe` (the Connect webhook on
//! the platform's Stripe account; only a super admin writes the platform's).
//! The WhatsApp app-level `calls` webhook subscription after a save is Q.I1.

use chatbotx_business::BusinessError;
use chatbotx_business::connection::credentials::HttpUpstream;
use chatbotx_business::connection::oauth_channels::{self, TiktokSettings};
use chatbotx_business::platform_credential::{self as pc, DecryptOptions, UpsertCredential};
use chatbotx_core::encryption::Encryption;
use serde_json::{Map, Value, json};

use super::RpcContext;
use super::actions::{ActionContext, ActionError, ActionResult, Gate};
use super::workspace::RpcOut;
use crate::handlers::input::{Input, Str, child, key};
use crate::handlers::{ApiState, LISTING, business_error};
use crate::orpc::OrpcError;

/// `(action name, credential type)` of the plain delete actions.
const DELETES: &[(&str, &str)] = &[
    ("deleteGiphySettings", "giphy"),
    ("deleteGoogleAdsSettings", "googleAds"),
    ("deleteGoogleSettings", "google"),
    ("deleteInstagramFacebookSettings", "instagramFacebook"),
    ("deleteInstagramSettings", "instagram"),
    ("deleteMessengerSettings", "messenger"),
    ("deleteThreadsSettings", "threads"),
    ("deleteTiktokSettings", "tiktok"),
    ("deleteWhatsappSettings", "whatsapp"),
    ("deleteZaloSettings", "zalo"),
];

/// `(action name, credential type)` of the Meta-style app updates
/// (`clientId`, `version`, `verifyToken`, `clientSecret`).
const APP_UPDATES: &[(&str, &str)] = &[
    ("updateInstagramSetting", "instagram"),
    ("updateInstagramFacebookSetting", "instagramFacebook"),
    ("updateThreadsSetting", "threads"),
    ("updateZaloSettings", "zalo"),
];

/// The gate of an action (`None` = not ported).
pub fn action_gate(name: &str) -> Option<Gate> {
    let known = DELETES.iter().any(|(n, _)| *n == name)
        || APP_UPDATES.iter().any(|(n, _)| *n == name)
        || matches!(
            name,
            "updateWhatsappSettings"
                | "updateMessengerSetting"
                | "updateGoogleSettings"
                | "updateGoogleAdsSettings"
                | "clearGoogleAdsDeveloperToken"
                | "updateGiphySettings"
                | "updateMakeSettings"
                | "cloneFromMessenger"
                | "updateTiktokSetting"
                | "updateStripeSettings"
                | "deleteStripeSettings"
        );
    known.then_some(Gate::Auth)
}

pub(super) fn unauthorized() -> ActionError {
    ActionError::Exception(BusinessError::exception(
        "Unauthorized",
        "unauthorized",
        403,
    ))
}

fn encryption(api: &ApiState) -> Result<&Encryption, ActionError> {
    api.encryption
        .as_deref()
        .ok_or_else(|| ActionError::Internal("ENCRYPTION_KEY is not set".into()))
}

/// `isSuperAdmin`: a `PlatformAdmin` row.
pub async fn is_super_admin(api: &ApiState, user_id: i64) -> Result<bool, ActionError> {
    Ok(api
        .db
        .read(move |conn| {
            chatbotx_db::repo::platform_admin::find_for_auth(conn, user_id)
                .map_err(|e| chatbotx_db::DbError::other(e.message()))
        })
        .await
        .map_err(|e| ActionError::Internal(e.to_string()))?
        .is_some())
}

/// `credentialScopeSchema` of the bound arg.
fn scope_of(input: &mut Input) -> Option<String> {
    let value = input.get("scope").cloned();
    input.enum_at(&key("scope"), value.as_ref(), &["user", "platform"], true)
}

/// `resolveCredentialScopedUserId`: `platform` = NULL (super admin only);
/// `user` = the caller on cloud (with the own-credential entitlement unless
/// `allow_without_entitlement`), else the platform scope.
pub async fn resolve_scope(
    api: &ApiState,
    context: &ActionContext,
    scope: &str,
    allow_without_entitlement: bool,
) -> Result<Option<i64>, ActionError> {
    if scope == "platform" {
        if !is_super_admin(api, context.user_id).await? {
            return Err(unauthorized());
        }
        return Ok(None);
    }
    if !api.edition.is_cloud() {
        return Ok(None);
    }
    if !allow_without_entitlement {
        let kv = api.kv.as_deref().map(|kv| &kv.durable);
        if !pc::can_use_own_credentials(&api.db, kv, api.edition, context.user_id).await? {
            return Err(unauthorized());
        }
    }
    Ok(Some(context.user_id))
}

fn input_of(raw: &Value) -> Input {
    Input::from_object(raw.as_object().cloned().unwrap_or_default())
}

/// `z.string().trim().min(1)`.
const REQUIRED: Str = Str {
    trim: true,
    min: Some(1),
    max: None,
    pattern: None,
};
/// `z.string().trim()`.
const TRIMMED: Str = Str {
    trim: true,
    min: None,
    max: None,
    pattern: None,
};

/// Reads `fields` (`(name, required)`) into a config object.
fn fields(input: &mut Input, fields: &[(&str, bool)]) -> Map<String, Value> {
    let mut out = Map::new();
    for &(name, required) in fields {
        let value = if required {
            input.string(name, REQUIRED)
        } else {
            input.opt_string(name, TRIMMED)
        };
        if let Some(value) = value {
            out.insert(name.to_string(), Value::String(value));
        }
    }
    out
}

fn text<'a>(config: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    config
        .get(name)
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
}

async fn write(
    api: &ApiState,
    f: impl FnOnce(&chatbotx_db::rusqlite::Connection, &Encryption) -> chatbotx_business::Result<Value>
    + Send
    + 'static,
) -> Result<Value, ActionError> {
    let encryption = encryption(api)?.clone();
    Ok(chatbotx_business::tx::write(&api.db, move |conn| f(conn, &encryption)).await?)
}

/// The stored config of the scope (`strict`).
pub(super) async fn stored(
    api: &ApiState,
    user: Option<i64>,
    credential_type: &'static str,
) -> Result<Option<Map<String, Value>>, ActionError> {
    let encryption = encryption(api)?.clone();
    let found = chatbotx_business::tx::read(&api.db, move |conn| {
        pc::find_decrypted(
            conn,
            &encryption,
            user,
            credential_type,
            DecryptOptions {
                strict: true,
                ..Default::default()
            },
        )
    })
    .await?;
    Ok(found.map(|c| c.config))
}

pub(super) async fn save(
    api: &ApiState,
    user: Option<i64>,
    credential_type: &'static str,
    config: Map<String, Value>,
) -> Result<(), ActionError> {
    write(api, move |conn, encryption| {
        pc::upsert(
            conn,
            encryption,
            UpsertCredential::new(user, credential_type, Value::Object(config)),
        )?;
        Ok(Value::Null)
    })
    .await?;
    Ok(())
}

fn secrets_required(message: &str, code: &str) -> ActionError {
    ActionError::Exception(BusinessError::exception(message, code, 422))
}

/// `isValidGiphyApiKey`: `GET https://api.giphy.com/v1/gifs/random`.
async fn is_valid_giphy_api_key(api_key: &str) -> bool {
    let url = url::form_urlencoded::Serializer::new(String::from(
        "https://api.giphy.com/v1/gifs/random?",
    ))
    .append_pair("api_key", api_key)
    .finish();
    let client = reqwest::Client::new();
    match client
        .get(url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => true,
        Ok(response) => {
            tracing::error!(status = %response.status(), "Invalid GIPHY API key");
            false
        }
        Err(err) => {
            tracing::error!(err = %err, "Invalid GIPHY API key");
            false
        }
    }
}

/// `tiktokCredentialUpdateSchema.features`: both default on; not both off.
fn tiktok_features(input: &mut Input) -> (bool, bool) {
    let path = key("features");
    match input.get("features").cloned() {
        None => (true, true),
        Some(Value::Object(features)) => {
            let messaging =
                input.bool_at(&child(&path, "messaging"), features.get("messaging"), true);
            let comments = input.bool_at(&child(&path, "comments"), features.get("comments"), true);
            if messaging == Some(false) && comments == Some(false) {
                input.issue(&child(&path, "messaging"), "custom", "Invalid input");
            }
            (messaging.unwrap_or(true), comments.unwrap_or(true))
        }
        Some(_) => {
            input.issue(&path, "invalid_type", "Invalid input: expected object");
            (true, true)
        }
    }
}

/// Runs one credential action.
pub async fn run_action(
    api: &ApiState,
    name: &str,
    context: &ActionContext,
    raw: &Value,
) -> ActionResult {
    let mut input = input_of(raw);
    let scope = scope_of(&mut input);

    if let Some(&(_, credential_type)) = DELETES.iter().find(|(n, _)| *n == name) {
        input.finish()?;
        let user = resolve_scope(api, context, &scope.unwrap_or_default(), true).await?;
        let db_type = credential_type.to_string();
        chatbotx_business::tx::write(&api.db, move |conn| Ok(pc::remove(conn, user, &db_type)?))
            .await?;
        return Ok(None);
    }

    if let Some(&(_, credential_type)) = APP_UPDATES.iter().find(|(n, _)| *n == name) {
        let config = fields(
            &mut input,
            &[
                ("clientId", true),
                ("version", true),
                ("verifyToken", true),
                ("clientSecret", true),
            ],
        );
        input.finish()?;
        let user = resolve_scope(api, context, &scope.unwrap_or_default(), false).await?;
        save(api, user, credential_type, config).await?;
        return Ok(None);
    }

    match name {
        "updateMessengerSetting" => {
            let config = fields(
                &mut input,
                &[
                    ("clientId", true),
                    ("version", true),
                    ("verifyToken", true),
                    ("clientSecret", true),
                    ("marketingMessagesConfigId", false),
                ],
            );
            input.finish()?;
            let user = resolve_scope(api, context, &scope.unwrap_or_default(), false).await?;
            save(api, user, "messenger", config).await?;
            Ok(None)
        }
        "updateGoogleSettings" => {
            let config = fields(
                &mut input,
                &[
                    ("clientId", true),
                    ("clientSecret", true),
                    ("verifyToken", true),
                ],
            );
            input.finish()?;
            let user = resolve_scope(api, context, &scope.unwrap_or_default(), false).await?;
            save(api, user, "google", config).await?;
            Ok(None)
        }
        "updateMakeSettings" => {
            let invite = input.string("inviteUrl", REQUIRED);
            if let Some(url) = &invite
                && !chatbotx_core::utils::zod::is_url(url)
            {
                input.issue(&key("inviteUrl"), "invalid_format", "Invalid URL");
            }
            input.finish()?;
            let user = resolve_scope(api, context, &scope.unwrap_or_default(), false).await?;
            let mut config = Map::new();
            config.insert(
                "inviteUrl".into(),
                Value::String(invite.unwrap_or_default()),
            );
            save(api, user, "make", config).await?;
            Ok(None)
        }
        // A blank secret keeps the stored one (read `strict` only then).
        "updateWhatsappSettings" => {
            let mut config = fields(
                &mut input,
                &[
                    ("clientId", true),
                    ("version", true),
                    ("configId", true),
                    ("systemUserId", true),
                    ("businessId", false),
                    ("businessName", true),
                    ("verifyToken", true),
                    ("clientSecret", false),
                    ("systemUserToken", false),
                    ("capiConfigId", false),
                ],
            );
            input.finish()?;
            let user = resolve_scope(api, context, &scope.unwrap_or_default(), false).await?;
            let needs_stored = text(&config, "clientSecret").is_none()
                || text(&config, "systemUserToken").is_none();
            let previous = if needs_stored {
                stored(api, user, "whatsapp").await?
            } else {
                None
            };
            for secret in ["clientSecret", "systemUserToken"] {
                let value = text(&config, secret).map(str::to_string).or_else(|| {
                    previous
                        .as_ref()
                        .and_then(|p| text(p, secret))
                        .map(str::to_string)
                });
                match value {
                    Some(value) => {
                        config.insert(secret.into(), Value::String(value));
                    }
                    None => {
                        return Err(secrets_required(
                            "Enter the app secret and system user token. They aren't stored yet.",
                            "whatsapp.credentialSecretsRequired",
                        ));
                    }
                }
            }
            save(api, user, "whatsapp", config).await?;
            // ponytail: no `calls` webhook subscription warning until Q.I1 ports the Graph call.
            Ok(Some(json!({})))
        }
        "updateGoogleAdsSettings" => {
            let mut config = fields(
                &mut input,
                &[
                    ("clientId", true),
                    ("clientSecret", false),
                    ("developerToken", false),
                ],
            );
            let upload = {
                let value = input.get("uploadMethod").cloned();
                input.enum_at(
                    &key("uploadMethod"),
                    value.as_ref(),
                    &["dataManager", "legacy"],
                    false,
                )
            };
            input.finish()?;
            let user = resolve_scope(api, context, &scope.unwrap_or_default(), false).await?;
            let needs_stored = text(&config, "clientSecret").is_none()
                || text(&config, "developerToken").is_none();
            let previous = if needs_stored {
                stored(api, user, "googleAds").await?
            } else {
                None
            };
            let keep = |name: &str, config: &Map<String, Value>| {
                text(config, name).map(str::to_string).or_else(|| {
                    previous
                        .as_ref()
                        .and_then(|p| text(p, name))
                        .map(str::to_string)
                })
            };
            let secret = keep("clientSecret", &config);
            let token = keep("developerToken", &config);
            let upload = upload.or_else(|| {
                previous
                    .as_ref()
                    .and_then(|p| text(p, "uploadMethod"))
                    .map(str::to_string)
            });
            let Some(secret) = secret else {
                return Err(secrets_required(
                    "Enter the client secret. It isn't stored yet.",
                    "googleAds.credentialSecretsRequired",
                ));
            };
            config.insert("clientSecret".into(), Value::String(secret));
            config.remove("developerToken");
            if let Some(token) = token {
                config.insert("developerToken".into(), Value::String(token));
            }
            if let Some(upload) = upload {
                config.insert("uploadMethod".into(), Value::String(upload));
            }
            save(api, user, "googleAds", config).await?;
            Ok(None)
        }
        "clearGoogleAdsDeveloperToken" => {
            input.finish()?;
            let user = resolve_scope(api, context, &scope.unwrap_or_default(), false).await?;
            let Some(mut config) = stored(api, user, "googleAds").await? else {
                return Err(secrets_required(
                    "Enter the client secret. It isn't stored yet.",
                    "googleAds.credentialSecretsRequired",
                ));
            };
            config.remove("developerToken");
            save(api, user, "googleAds", config).await?;
            Ok(None)
        }
        "updateGiphySettings" => {
            let api_key = input
                .opt_string("apiKey", TRIMMED)
                .filter(|k| !k.is_empty());
            input.finish()?;
            let user = resolve_scope(api, context, &scope.unwrap_or_default(), false).await?;
            let encryption = encryption(api)?.clone();
            let existing = chatbotx_business::tx::read(&api.db, move |conn| {
                pc::find_decrypted(conn, &encryption, user, "giphy", DecryptOptions::default())
            })
            .await?;
            if let Some(key) = &api_key
                && !is_valid_giphy_api_key(key).await
            {
                return Err(ActionError::ValidationErrors(
                    json!({ "apiKey": { "_errors": ["Invalid GIPHY API key"] } }),
                ));
            }
            let api_key = api_key.or_else(|| {
                existing
                    .as_ref()
                    .and_then(|c| c.text("apiKey"))
                    .map(str::to_string)
            });
            let Some(api_key) = api_key else {
                return Err(ActionError::Internal(
                    "API Key is required to configure GIPHY.".into(),
                ));
            };
            let mut config = Map::new();
            config.insert("apiKey".into(), Value::String(api_key));
            save(api, user, "giphy", config).await?;
            Ok(None)
        }
        "cloneFromMessenger" => {
            input.finish()?;
            let user = resolve_scope(api, context, &scope.unwrap_or_default(), false).await?;
            let encryption = encryption(api)?.clone();
            let messenger = chatbotx_business::tx::read(&api.db, move |conn| {
                pc::find_decrypted(
                    conn,
                    &encryption,
                    user,
                    "messenger",
                    DecryptOptions::default(),
                )
            })
            .await?;
            let Some(messenger) = messenger else {
                return Err(ActionError::Internal(
                    "Messenger credentials not configured.".into(),
                ));
            };
            save(api, user, "instagramFacebook", messenger.config).await?;
            Ok(None)
        }
        "updateTiktokSetting" => {
            let config = fields(&mut input, &[("clientId", true), ("clientSecret", true)]);
            let features = tiktok_features(&mut input);
            input.finish()?;
            let user = resolve_scope(api, context, &scope.unwrap_or_default(), false).await?;
            let broker = std::env::var("NEXT_PUBLIC_BROKER_URL")
                .ok()
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| api.app_url.clone());
            let broker_origin = url::Url::parse(&broker)
                .map(|u| u.origin().ascii_serialization())
                .map_err(|e| ActionError::Internal(format!("broker URL: {e}")))?;
            let text_of = |name: &str| text(&config, name).unwrap_or_default().to_string();
            oauth_channels::update_tiktok_settings(
                &api.db,
                encryption(api)?,
                &HttpUpstream,
                TiktokSettings {
                    user_id: user,
                    cloud: api.edition.is_cloud(),
                    broker_origin,
                    client_id: text_of("clientId"),
                    client_secret: text_of("clientSecret"),
                    features,
                },
            )
            .await?;
            Ok(None)
        }
        "updateStripeSettings" => {
            let mut config = fields(
                &mut input,
                &[
                    ("publishableKey", true),
                    ("verifyToken", true),
                    ("secretKey", true),
                ],
            );
            if let Some(client_id) = input.string(
                "connectClientId",
                Str {
                    pattern: Some(is_connect_client_id),
                    ..TRIMMED
                },
            ) {
                config.insert("connectClientId".into(), Value::String(client_id));
            }
            input.finish()?;
            let user = stripe_scope(api, context, &scope.unwrap_or_default(), false).await?;
            save_platform_stripe(api, user, config).await?;
            Ok(None)
        }
        "deleteStripeSettings" => {
            input.finish()?;
            let user = stripe_scope(api, context, &scope.unwrap_or_default(), true).await?;
            remove_platform_stripe(api, user).await?;
            Ok(None)
        }
        _ => Err(ActionError::Internal(format!("unknown action {name}"))),
    }
}

/// `^ca_[A-Za-z0-9]+$` (the Stripe Connect OAuth client id).
fn is_connect_client_id(text: &str) -> bool {
    text.strip_prefix("ca_")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// `resolveStripeCredentialScope`: the platform-wide Stripe settings (self
/// hosted, or the `platform` scope) are written by a super admin only.
async fn stripe_scope(
    api: &ApiState,
    context: &ActionContext,
    scope: &str,
    allow_without_entitlement: bool,
) -> Result<Option<i64>, ActionError> {
    let user = resolve_scope(api, context, scope, allow_without_entitlement).await?;
    if user.is_none() && !is_super_admin(api, context.user_id).await? {
        return Err(unauthorized());
    }
    Ok(user)
}

/// `ecommercePaymentRepository.countByPlatformCredential`.
async fn count_connected(api: &ApiState, credential_id: i64) -> Result<i64, ActionError> {
    Ok(chatbotx_business::tx::read(&api.db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT COUNT(*) FROM "EcommercePaymentProvider" WHERE "platformCredentialId" = ?1"#,
                [credential_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(chatbotx_db::repo::Error::from)?)
    })
    .await?)
}

/// `stripePlatformAccountInUseException`.
fn stripe_account_in_use(count: i64) -> ActionError {
    ActionError::Exception(
        BusinessError::exception(
            format!(
                "{count} store(s) are connected through the current Stripe account. A key of another Stripe account would cut them off; disconnect them first."
            ),
            "stripePlatformAccountInUse",
            409,
        )
        .with_data(json!({ "count": count })),
    )
}

fn stripe_error(error: &chatbotx_business::payments::ProviderError) -> ActionError {
    ActionError::Exception(chatbotx_business::payment_provider::to_exception(
        "stripe", error,
    ))
}

/// The stored Stripe settings of the scope, with the row id.
async fn stored_stripe(
    api: &ApiState,
    user: Option<i64>,
) -> Result<Option<(i64, Map<String, Value>)>, ActionError> {
    let encryption = encryption(api)?.clone();
    let found = chatbotx_business::tx::read(&api.db, move |conn| {
        pc::find_decrypted(conn, &encryption, user, "stripe", DecryptOptions::default())
    })
    .await?;
    Ok(found.map(|c| (c.id, c.config)))
}

/// `savePlatformStripe`: the key's account is read (which also checks the
/// key); a key of another account is refused while stores are connected
/// through the current one; the Connect webhook is kept on the same account,
/// else created (its signing secret is stored with the settings).
async fn save_platform_stripe(
    api: &ApiState,
    user: Option<i64>,
    input: Map<String, Value>,
) -> Result<(), ActionError> {
    use chatbotx_business::payments::stripe;
    let base = stripe::STRIPE_API;
    let secret_key = text(&input, "secretKey").unwrap_or_default().to_string();
    let existing = stored_stripe(api, user).await?;
    let account_id = stripe::read_platform_account_id(base, &secret_key)
        .await
        .map_err(|e| stripe_error(&e))?;
    let previous_account = match &existing {
        Some((_, config)) => match text(config, "platformAccountId") {
            Some(id) => Some(id.to_string()),
            None => stripe::read_platform_account_id(
                base,
                text(config, "secretKey").unwrap_or_default(),
            )
            .await
            .ok(),
        },
        None => None,
    };
    let same_account = existing.is_some() && previous_account.as_deref() == Some(&account_id);
    if let Some((id, _)) = &existing
        && !same_account
    {
        let connected = count_connected(api, *id).await?;
        if connected > 0 {
            return Err(stripe_account_in_use(connected));
        }
    }
    // A webhook belongs to the Stripe account it was created on.
    let kept = existing
        .as_ref()
        .filter(|_| same_account)
        .and_then(|(_, c)| {
            Some((
                text(c, "connectWebhookEndpointId")?.to_string(),
                text(c, "connectWebhookSecret")?.to_string(),
            ))
        });
    let mut config = input;
    config.insert("platformAccountId".into(), json!(account_id));
    if let Some((endpoint, secret)) = &kept {
        config.insert("connectWebhookEndpointId".into(), json!(endpoint));
        config.insert("connectWebhookSecret".into(), json!(secret));
    }
    save(api, user, "stripe", config.clone()).await?;
    if !same_account
        && let Some((_, old)) = &existing
        && let Some(endpoint) = text(old, "connectWebhookEndpointId")
        && let Err(err) =
            stripe::delete_webhook(base, text(old, "secretKey").unwrap_or_default(), endpoint).await
    {
        tracing::warn!(err = %err.message, endpoint, "[payments] could not remove the Stripe Connect webhook");
    }
    let Some((saved_id, _)) = stored_stripe(api, user).await? else {
        return Err(ActionError::Internal(
            "Failed to save the Stripe settings".into(),
        ));
    };
    // The webhook origin: the reseller's own domain for their own app, the
    // broker otherwise (`resolveProviderOriginForCredential`).
    let origin = match user {
        Some(owner) => custom_domain_origin(api, owner).await?,
        None => None,
    }
    .unwrap_or_else(|| crate::handlers::messenger_templates::broker_origin(&api.app_url));
    let url = format!(
        "{}/api/payments/stripe/webhook/{saved_id}",
        origin.trim_end_matches('/')
    );
    let webhook = stripe::ensure_connect_webhook(base, &secret_key, &url, kept.clone())
        .await
        .map_err(|e| stripe_error(&e))?;
    if Some(&webhook) == kept.as_ref() {
        return Ok(());
    }
    config.insert("connectWebhookEndpointId".into(), json!(webhook.0));
    config.insert("connectWebhookSecret".into(), json!(webhook.1));
    save(api, user, "stripe", config).await
}

/// `resolveTenantCustomDomainOrigin` (cloud, the owner's active tenant).
async fn custom_domain_origin(api: &ApiState, owner: i64) -> Result<Option<String>, ActionError> {
    if !api.edition.is_cloud() {
        return Ok(None);
    }
    let Some(tenant) = chatbotx_business::tenant::find_by_owner(&api.db, owner).await? else {
        return Ok(None);
    };
    if tenant.status != "active" {
        return Ok(None);
    }
    Ok(
        chatbotx_business::tenant::find_active_domain_by_tenant(&api.db, tenant.id)
            .await?
            .map(|d| format!("https://{}", d.domain)),
    )
}

/// `removePlatformStripe`: refused while stores are connected through the
/// settings; the Connect webhook is removed best effort.
async fn remove_platform_stripe(api: &ApiState, user: Option<i64>) -> Result<(), ActionError> {
    let existing = stored_stripe(api, user).await?;
    if let Some((id, _)) = &existing {
        let connected = count_connected(api, *id).await?;
        if connected > 0 {
            return Err(stripe_account_in_use(connected));
        }
    }
    chatbotx_business::tx::write(&api.db, move |conn| Ok(pc::remove(conn, user, "stripe")?))
        .await?;
    if let Some((_, old)) = &existing
        && let Some(endpoint) = text(old, "connectWebhookEndpointId")
        && let Err(err) = chatbotx_business::payments::stripe::delete_webhook(
            chatbotx_business::payments::stripe::STRIPE_API,
            text(old, "secretKey").unwrap_or_default(),
            endpoint,
        )
        .await
    {
        tracing::warn!(err = %err.message, endpoint, "[payments] could not remove the Stripe Connect webhook");
    }
    Ok(())
}

/// `platformCredentialsAPI.getGiphyApiKeyAPI`: the caller's (cloud) or the
/// platform GIPHY key.
pub async fn call(
    api: &ApiState,
    path: &str,
    context: &RpcContext,
) -> Option<Result<RpcOut, OrpcError>> {
    if path != "platformCredentialsAPI.getGiphyApiKeyAPI" {
        return None;
    }
    let user = api.edition.is_cloud().then_some(context.session.user_id);
    let Some(encryption) = api.encryption.clone() else {
        return Some(Ok((json!({ "apiKey": null }), Vec::new())));
    };
    let found = chatbotx_business::tx::read(&api.db, move |conn| {
        pc::find_decrypted(conn, &encryption, user, "giphy", DecryptOptions::default())
    })
    .await;
    Some(
        found
            .map(|c| {
                let key = c.as_ref().and_then(|c| c.text("apiKey"));
                (json!({ "apiKey": key }), Vec::new())
            })
            .map_err(|e| business_error(e, LISTING)),
    )
}
