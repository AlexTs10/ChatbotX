//! `features/platform-credentials` (Q.D11 slice 7): the credential settings
//! actions (`POST /rpc/platformCredentialActions/<name>`, `authActionClient`)
//! and `platformCredentialsAPI.getGiphyApiKeyAPI`.
//!
//! The bound `credentialScopeSchema` arg is the input field `scope`
//! (`"user"` | `"platform"`). Server-side messages are the English
//! `messages/en.json` text. Not ported here: `updateStripeSettings` /
//! `deleteStripeSettings` (Stripe Connect webhook, P3.6-D1o) and
//! `updateTiktokSetting` (TikTok webhook subscription, Q.I2); the WhatsApp
//! app-level `calls` webhook subscription after a save is Q.I1.

use chatbotx_business::BusinessError;
use chatbotx_business::platform_credential::{self as pc, DecryptOptions, UpsertCredential};
use chatbotx_core::encryption::Encryption;
use serde_json::{Map, Value, json};

use super::RpcContext;
use super::actions::{ActionContext, ActionError, ActionResult, Gate};
use super::workspace::RpcOut;
use crate::handlers::input::{Input, Str, key};
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
async fn stored(
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

async fn save(
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
        _ => Err(ActionError::Internal(format!("unknown action {name}"))),
    }
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
