//! The Meta ads OAuth connects: `connectLeadAdsAction`,
//! `connectMarketingMessagesAction`, `connectMessagingAdsAction`,
//! `connectMetaCatalogAction` (`generateCatalogAuthUrl`,
//! `storeMetaCatalogConnection`) and their
//! `flow` branches of `app/integrations/[...integration]/callback.ts`
//! (`callback-stores.ts`). The original's unsigned base64 JSON `state` is a
//! signed connect session here (`{sessionId}.{nonce}`, provider `messenger`,
//! purpose = the flow), bound to the user who pressed Connect.
use chatbotx_core::encryption::Encryption;
use chatbotx_db::Db;
use chatbotx_db::repo::messaging_ads_connection::{
    self as connections, CreateMessagingAdsConnectionInput,
};
use chatbotx_db::repo::{Json, new_id, now_ms};
use chatbotx_db::rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use crate::connect_session::{self, NewSession, SessionRow};
use crate::connection::credentials::{Upstream, UpstreamRequest};
use crate::connection::oauth::{self, OAuthCredential};
use crate::errors::{BusinessError, Result};
use crate::meta_catalog::process_encryption;
use crate::tx;

const FACEBOOK: &str = "https://www.facebook.com";
const GRAPH: &str = "https://graph.facebook.com";
const DEFAULT_VERSION: &str = "v23.0";
const LEAD_ADS_SCOPES: &[&str] = &[
    "leads_retrieval",
    "pages_manage_ads",
    "pages_manage_metadata",
    "pages_show_list",
];
const FACEBOOK_ADS_SCOPES: &[&str] = &[
    "ads_read",
    "ads_management",
    "pages_manage_ads",
    "pages_read_engagement",
    "pages_show_list",
];
pub const LEAD_ADS: &str = "facebookLeadAds";
pub const MARKETING_MESSAGES: &str = "facebookMarketingMessages";
pub const MESSAGING_ADS: &str = "messagingAds";
pub const META_CATALOG: &str = "metaCatalog";
/// `META_CATALOG_SCOPES`.
const META_CATALOG_SCOPES: &[&str] = &["catalog_management"];
/// The connect-session purposes of these flows.
pub const PURPOSES: &[&str] = &[LEAD_ADS, MARKETING_MESSAGES, MESSAGING_ADS, META_CATALOG];
const AUTHORIZED: &[&str] = &["authorized"];

/// One connect flow (`stateParams.flow`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Flow {
    LeadAds,
    MarketingMessages,
    MessagingAds {
        channel: String,
        integration_id: i64,
    },
    MetaCatalog,
}

impl Flow {
    fn purpose(&self) -> &'static str {
        match self {
            Flow::LeadAds => LEAD_ADS,
            Flow::MarketingMessages => MARKETING_MESSAGES,
            Flow::MessagingAds { .. } => MESSAGING_ADS,
            Flow::MetaCatalog => META_CATALOG,
        }
    }

    /// The "no app credential" error (`metaCatalog.errors.invalidAppSettings`
    /// for the catalog, else `facebookAds.errors.invalidAppSettings`; en).
    fn invalid_app_settings(&self) -> BusinessError {
        match self {
            Flow::MetaCatalog => BusinessError::exception(
                "Meta app settings are not configured.",
                "systemError",
                400,
            ),
            _ => invalid_app_settings(),
        }
    }
}

fn invalid_app_settings() -> BusinessError {
    BusinessError::exception("Facebook App settings are not valid", "systemError", 400)
}

fn version(config: &Value) -> &str {
    config["version"]
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or(DEFAULT_VERSION)
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}

/// `messagingAdsScopesForChannel`.
fn messaging_ads_scopes(channel: &str) -> Vec<&'static str> {
    let mut scopes = FACEBOOK_ADS_SCOPES.to_vec();
    scopes.extend(match channel {
        "whatsapp" => &["whatsapp_business_management"][..],
        "instagram" => &["instagram_basic"][..],
        _ => &[][..],
    });
    scopes
}

/// The Facebook dialog URL of a flow (`generateLeadAdsAuthUrl`,
/// `generateMarketingMessagesAuthUrl`, `generateAdsAuthUrl`).
pub fn auth_url(flow: &Flow, credential: &OAuthCredential, state: &str) -> Result<String> {
    let config = &credential.config;
    let client_id = text(config, "clientId");
    let redirect = credential.callback_url.as_str();
    let scope = |scopes: &[&str]| scopes.join(",");
    let pairs: Vec<(&str, String)> = match flow {
        Flow::LeadAds => vec![
            ("auth_type", "rerequest".into()),
            ("client_id", client_id.into()),
            ("redirect_uri", redirect.into()),
            ("scope", scope(LEAD_ADS_SCOPES)),
            ("response_type", "code".into()),
            ("state", state.into()),
        ],
        Flow::MarketingMessages => {
            let config_id = text(config, "marketingMessagesConfigId");
            if config_id.is_empty() {
                return Err(BusinessError::exception(
                    "Facebook Marketing Messages is not configured. Ask your administrator to set the Marketing Messages Config ID.",
                    "systemError",
                    400,
                ));
            }
            vec![
                ("app_id", client_id.into()),
                ("client_id", client_id.into()),
                ("config_id", config_id.into()),
                ("auth_type", "rerequest".into()),
                ("override_default_response_type", "true".into()),
                ("redirect_uri", redirect.into()),
                ("response_type", "code".into()),
                ("state", state.into()),
            ]
        }
        Flow::MessagingAds { channel, .. } => vec![
            ("client_id", client_id.into()),
            ("redirect_uri", redirect.into()),
            ("scope", scope(&messaging_ads_scopes(channel))),
            ("response_type", "code".into()),
            ("state", state.into()),
        ],
        // `auth_type=rerequest`: Facebook otherwise skips a permission the
        // user declined before and the token misses `catalog_management`.
        Flow::MetaCatalog => vec![
            ("client_id", client_id.into()),
            ("redirect_uri", redirect.into()),
            ("auth_type", "rerequest".into()),
            ("scope", scope(META_CATALOG_SCOPES)),
            ("response_type", "code".into()),
            ("state", state.into()),
        ],
    };
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    let version = match flow {
        Flow::MetaCatalog => config["version"]
            .as_str()
            .filter(|v| !v.is_empty())
            .unwrap_or(crate::meta_catalog::DEFAULT_API_VERSION),
        _ => version(config),
    };
    Ok(format!("{FACEBOOK}/{version}/dialog/oauth?{query}"))
}

/// The connect action: a fresh `pending` session (older pending ones of the
/// same flow cancelled) and the Facebook dialog URL to open.
pub async fn start(
    db: &Db,
    encryption: &Encryption,
    app_url: &str,
    workspace_id: i64,
    user_id: i64,
    flow: Flow,
    return_path: String,
) -> Result<String> {
    let owner =
        crate::tenant::resolve_credential_owner_id_for_workspace(db, workspace_id, None).await?;
    let credential = oauth::resolve_credential(db, encryption, app_url, "messenger", Some(owner))
        .await?
        .filter(|c| !text(&c.config, "clientId").is_empty())
        .ok_or_else(|| flow.invalid_app_settings())?;
    // Fails before a session exists when the flow is not configured.
    auth_url(&flow, &credential, "")?;
    let purpose = flow.purpose();
    tx::write(db, move |conn| {
        conn.execute(
            r#"UPDATE "ConnectSession" SET "status" = 'cancelled', "updatedAt" = ?3
               WHERE "workspaceId" = ?1 AND "provider" = 'messenger' AND "purpose" = ?2
                 AND "status" = 'pending'"#,
            params![workspace_id, purpose, now_ms()],
        )?;
        Ok(())
    })
    .await?;
    let origin_host = url::Url::parse(app_url).ok().and_then(|u| {
        u.host_str().map(|h| match u.port() {
            Some(port) => format!("{h}:{port}"),
            None => h.to_string(),
        })
    });
    let flow_for_action = flow.clone();
    let session = connect_session::create(
        db,
        NewSession {
            workspace_id,
            provider: "messenger".into(),
            purpose: purpose.into(),
            actor_user_id: Some(user_id),
            platform_owner_id: Some(owner.to_string()),
            origin_host,
            return_url: Some(return_path),
            ..NewSession::default()
        },
        move |id, nonce| {
            let url = auth_url(&flow_for_action, &credential, &format!("{id}.{nonce}"))
                .unwrap_or_default();
            let mut action = json!({ "type": "open_url", "url": url });
            if let Flow::MessagingAds {
                channel,
                integration_id,
            } = &flow_for_action
            {
                action["channel"] = json!(channel);
                action["integrationId"] = json!(integration_id.to_string());
            }
            action
        },
    )
    .await?;
    session
        .next_action
        .as_ref()
        .and_then(|a| a["url"].as_str())
        .filter(|u| !u.is_empty())
        .map(str::to_string)
        .ok_or_else(invalid_app_settings)
}

/// The flow a session was started for (`None` = not an ads session).
pub fn flow_of(session: &SessionRow) -> Option<Flow> {
    if session.provider != "messenger" {
        return None;
    }
    match session.purpose.as_str() {
        LEAD_ADS => Some(Flow::LeadAds),
        MARKETING_MESSAGES => Some(Flow::MarketingMessages),
        META_CATALOG => Some(Flow::MetaCatalog),
        MESSAGING_ADS => {
            let action = session.next_action.as_ref()?;
            Some(Flow::MessagingAds {
                channel: action["channel"].as_str()?.to_string(),
                integration_id: action["integrationId"].as_str()?.parse().ok()?,
            })
        }
        _ => None,
    }
}

/// A Graph GET; `Err` carries the Graph message.
async fn graph_get(upstream: &impl Upstream, url: String) -> std::result::Result<Value, String> {
    let (status, body) = upstream
        .send(UpstreamRequest {
            method: "GET",
            url,
            headers: Vec::new(),
            body: None,
        })
        .await?;
    let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    if (200..300).contains(&status) {
        Ok(body)
    } else {
        Err(body["error"]["message"]
            .as_str()
            .unwrap_or("Facebook API error")
            .to_string())
    }
}

fn query_url(path: &str, pairs: &[(&str, &str)]) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    format!("{GRAPH}/{path}?{query}")
}

/// `exchangeCodeForToken` + `exchangeLongLivedToken`: the long-lived token
/// and its expiry (ms).
async fn exchange(
    upstream: &impl Upstream,
    credential: &OAuthCredential,
    code: &str,
) -> std::result::Result<(String, Option<i64>), String> {
    let config = &credential.config;
    let endpoint = format!("{}/oauth/access_token", version(config));
    let short = graph_get(
        upstream,
        query_url(
            &endpoint,
            &[
                ("client_id", text(config, "clientId")),
                ("client_secret", text(config, "clientSecret")),
                ("redirect_uri", &credential.callback_url),
                ("code", code),
            ],
        ),
    )
    .await?;
    let long = graph_get(
        upstream,
        query_url(
            &endpoint,
            &[
                ("grant_type", "fb_exchange_token"),
                ("client_id", text(config, "clientId")),
                ("client_secret", text(config, "clientSecret")),
                ("fb_exchange_token", text(&short, "access_token")),
            ],
        ),
    )
    .await?;
    let token = text(&long, "access_token");
    if token.is_empty() {
        return Err("Facebook returned no access token".into());
    }
    let expires = long["expires_in"].as_i64().map(|s| now_ms() + s * 1000);
    Ok((token.to_string(), expires))
}

/// `messagingAdsIntegrationBelongsToWorkspace`.
async fn integration_belongs(db: &Db, workspace_id: i64, channel: &str, id: i64) -> Result<bool> {
    let table = match channel {
        "whatsapp" => "IntegrationWhatsapp",
        "messenger" => "IntegrationMessenger",
        "instagram" => "IntegrationInstagram",
        _ => return Ok(false),
    };
    tx::read(db, move |conn| {
        Ok(conn
            .query_row(
                &format!(r#"SELECT 1 FROM "{table}" WHERE "id" = ?1 AND "workspaceId" = ?2"#),
                params![id, workspace_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    })
    .await
}

/// Who may finish a session: the member who started it. `super_admin` is
/// the caller's role check (messaging ads needs a super admin).
pub struct Caller {
    pub user_id: Option<i64>,
    pub is_member: bool,
    pub super_admin: bool,
}

/// The callback of an ads session. `Ok(false)` = not this caller's session
/// (the original's `notFound()`); `Err` = the store failed.
pub async fn complete(
    db: &Db,
    encryption: &Encryption,
    app_url: &str,
    upstream: &impl Upstream,
    session: &SessionRow,
    caller: Caller,
    code: &str,
) -> Result<bool> {
    let Some(flow) = flow_of(session) else {
        return Ok(false);
    };
    let flow = &flow;
    let bound = caller.user_id.is_some() && caller.user_id == session.actor_user_id;
    if !(bound && caller.is_member) || session.status != "pending" {
        return Ok(false);
    }
    let ws = session.workspace_id;
    if let Flow::MessagingAds {
        channel,
        integration_id,
    } = flow
        && !(caller.super_admin && integration_belongs(db, ws, channel, *integration_id).await?)
    {
        return Ok(false);
    }
    connect_session::claim_authorization(db, session.id, ws).await?;
    let outcome = store(db, encryption, app_url, upstream, session, flow, code).await;
    match &outcome {
        Ok(()) => {
            let row = session.clone();
            tx::write(db, move |conn| {
                connect_session::complete_authorized(
                    conn,
                    &row,
                    json!({ "ok": true }),
                    None,
                    now_ms(),
                )
                .map(|_| ())
            })
            .await?;
        }
        Err(_) => connect_session::fail(db, session, "exchange_failed", Some(AUTHORIZED)).await,
    }
    outcome.map(|()| true)
}

async fn store(
    db: &Db,
    encryption: &Encryption,
    app_url: &str,
    upstream: &impl Upstream,
    session: &SessionRow,
    flow: &Flow,
    code: &str,
) -> Result<()> {
    let ws = session.workspace_id;
    if *flow == Flow::LeadAds {
        // The original stores no token: it only subscribes the pages.
        return crate::facebook_lead_ads::enable_leadgen_for_workspace_pages(db, ws).await;
    }
    let owner = session
        .platform_owner_id
        .as_deref()
        .and_then(|id| id.parse::<i64>().ok());
    let credential = oauth::resolve_credential(db, encryption, app_url, "messenger", owner)
        .await?
        .ok_or_else(invalid_app_settings)?;
    let (token, expires_at) = exchange(upstream, &credential, code)
        .await
        .map_err(|message| BusinessError::exception(message, "systemError", 400))?;
    let mut auth = json!({
        "authType": "custom",
        "accessToken": token,
        "expiresAt": expires_at.map(chatbotx_db::repo::iso),
    });
    if let Some(v) = credential.config["version"].as_str() {
        auth["version"] = json!(v);
    }
    // `MetaCatalogAuthValue` carries no `authType`.
    if *flow == Flow::MetaCatalog
        && let Some(map) = auth.as_object_mut()
    {
        map.remove("authType");
    }
    let sealed = serde_json::to_value(
        process_encryption()
            .encrypt_object(&auth, None)
            .map_err(|e| BusinessError::exception(e.to_string(), "internalServerError", 500))?,
    )
    .map_err(|e| BusinessError::exception(e.to_string(), "internalServerError", 500))?;
    match flow {
        Flow::MarketingMessages => {
            // `lookupFacebookUser`: best effort.
            let me = graph_get(
                upstream,
                query_url(
                    &format!("{}/me", version(&credential.config)),
                    &[("fields", "id,name"), ("access_token", &token)],
                ),
            )
            .await
            .ok();
            let facebook_user_id = me.and_then(|m| m["id"].as_str().map(str::to_string));
            tx::write(db, move |conn| {
                use chatbotx_db::repo::facebook_marketing_message as mm;
                match mm::auth_find_by_workspace_id(conn, ws)? {
                    Some(existing) => {
                        mm::auth_update_auth(
                            conn,
                            &mm::AuthUpdateInput {
                                id: existing.id,
                                auth: sealed,
                                token_expires_at: expires_at,
                                facebook_user_id,
                            },
                        )?;
                    }
                    None => {
                        mm::auth_create(
                            conn,
                            &mm::AuthCreateInput {
                                id: new_id(),
                                workspace_id: ws,
                                auth: sealed,
                                token_expires_at: expires_at,
                                facebook_user_id,
                            },
                        )?;
                    }
                }
                Ok(())
            })
            .await
        }
        Flow::MessagingAds {
            channel,
            integration_id,
        } => {
            let (channel, integration_id) = (channel.clone(), *integration_id);
            let channel_for_tx = channel.clone();
            tx::write(db, move |conn| {
                let reference =
                    crate::ads::graph::integration_ref(ws, &channel_for_tx, integration_id);
                let auth = Json(sealed);
                match connections::find_for_integration(conn, reference)? {
                    Some(existing) => {
                        connections::update_auth(conn, existing.id, ws, &auth)?;
                    }
                    None => {
                        connections::create(
                            conn,
                            &CreateMessagingAdsConnectionInput {
                                id: new_id(),
                                integration: reference,
                                channel: channel_for_tx,
                                auth,
                            },
                        )?;
                    }
                }
                Ok(())
            })
            .await?;
            super::graph::invalidate(ws, &channel, integration_id);
            Ok(())
        }
        Flow::MetaCatalog => {
            tx::write(db, move |conn| {
                crate::meta_catalog::upsert(conn, ws, &sealed, expires_at).map(|_| ())
            })
            .await
        }
        Flow::LeadAds => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential(config: Value) -> OAuthCredential {
        OAuthCredential {
            config,
            callback_url: "https://app.test/integrations/messenger/callback".into(),
        }
    }

    #[test]
    fn builds_each_flow_url() {
        let c = credential(
            json!({ "clientId": "app1", "version": "v22.0", "marketingMessagesConfigId": "cfg" }),
        );
        let lead = auth_url(&Flow::LeadAds, &c, "1.n").unwrap();
        assert!(lead.starts_with(
            "https://www.facebook.com/v22.0/dialog/oauth?auth_type=rerequest&client_id=app1"
        ));
        assert!(lead.contains(
            "scope=leads_retrieval%2Cpages_manage_ads%2Cpages_manage_metadata%2Cpages_show_list"
        ));
        assert!(lead.contains("state=1.n"));
        let mm = auth_url(&Flow::MarketingMessages, &c, "1.n").unwrap();
        assert!(mm.contains("config_id=cfg") && mm.contains("override_default_response_type=true"));
        assert!(!mm.contains("scope="));
        let ads = Flow::MessagingAds {
            channel: "whatsapp".into(),
            integration_id: 5,
        };
        let ads = auth_url(&ads, &c, "1.n").unwrap();
        assert!(ads.contains("whatsapp_business_management") && !ads.contains("auth_type"));
    }

    /// `integrations/meta-catalog/__tests__/auth.test.ts`.
    #[test]
    fn meta_catalog_asks_facebook_to_rerequest_previously_declined_permissions() {
        let c = credential(json!({ "clientId": "client-id" }));
        let url = url::Url::parse(&auth_url(&Flow::MetaCatalog, &c, "1.n").unwrap()).unwrap();
        let get = |k: &str| {
            url.query_pairs()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.into_owned())
        };
        assert_eq!(get("auth_type").as_deref(), Some("rerequest"));
        assert!(get("scope").unwrap().contains("catalog_management"));
        assert!(url.path().starts_with("/v24.0/"));
    }

    #[test]
    fn marketing_messages_needs_a_config_id() {
        let c = credential(json!({ "clientId": "app1" }));
        assert!(auth_url(&Flow::MarketingMessages, &c, "s").is_err());
        assert!(
            auth_url(&Flow::LeadAds, &c, "s")
                .unwrap()
                .contains("/v23.0/")
        );
    }

    #[test]
    fn messaging_ads_scopes_per_channel() {
        assert!(messaging_ads_scopes("instagram").contains(&"instagram_basic"));
        assert_eq!(
            messaging_ads_scopes("messenger"),
            FACEBOOK_ADS_SCOPES.to_vec()
        );
    }
}
