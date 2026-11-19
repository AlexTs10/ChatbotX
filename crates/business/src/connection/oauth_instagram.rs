//! The Instagram OAuth `connection` adapters (Q.D12-channels):
//! `instagram` (Instagram login, `integrations/instagram/src/integration.ts`,
//! one account per grant) and `instagramFacebook` (Instagram accounts linked
//! to the user's Facebook pages, `integrations/instagram-facebook`). Both
//! persist an `IntegrationInstagram` row (`type` = `instagram` / `facebook`,
//! `store-bindings-channels.ts`) keyed by the shared `igId`.

use chatbotx_db::repo::new_id;
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::ConnectionRow;
use super::credentials::{Upstream, UpstreamRequest};
use super::oauth::{
    ProviderFailure, call, get, new_channel_inbox, oauth2_auth, query_url, text, unique_or_repo,
    upsert_channel_connection,
};
use crate::{BusinessError, Result};

const INSTAGRAM_OAUTH: &str = "https://api.instagram.com";
const INSTAGRAM_GRAPH: &str = "https://graph.instagram.com";
const FACEBOOK_GRAPH: &str = "https://graph.facebook.com";
/// `integrations/instagram` `DEFAULT_API_VERSION`.
const INSTAGRAM_VERSION: &str = "v22.0";
/// `integrations/instagram-facebook` `DEFAULT_API_VERSION`.
const FACEBOOK_VERSION: &str = "v23.0";
const BUSINESS_SCOPES: &[&str] = &[
    "instagram_business_basic",
    "instagram_business_manage_messages",
    "instagram_business_manage_comments",
];
const FACEBOOK_SCOPES: &[&str] = &[
    "instagram_basic",
    "instagram_manage_comments",
    "instagram_manage_engagement",
    "instagram_manage_messages",
    "pages_manage_metadata",
    "pages_show_list",
    "pages_messaging",
    "pages_read_engagement",
    "business_management",
];
const VALID_ACCOUNT_TYPES: &[&str] = &["BUSINESS", "CREATOR", "MEDIA_CREATOR"];
const INSTAGRAM_SUBSCRIBE_FIELDS: &[&str] = &[
    "messages",
    "messaging_postbacks",
    "messaging_optins",
    "messaging_seen",
    "messaging_referral",
    "comments",
    "live_comments",
];
const FACEBOOK_SUBSCRIBE_FIELDS: &[&str] = &[
    "messages",
    "messaging_postbacks",
    "messaging_optins",
    "message_reads",
    "messaging_referrals",
    "message_echoes",
    "feed",
];

/// `true` for the providers of this module.
pub fn handles(provider: &str) -> bool {
    matches!(provider, "instagram" | "instagramFacebook")
}

fn facebook_version(config: &Value) -> String {
    config["version"]
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or(FACEBOOK_VERSION)
        .to_string()
}

/// `authorizeUrl`.
pub fn authorize_url(provider: &str, config: &Value, callback_url: &str, state: &str) -> String {
    if provider == "instagram" {
        return query_url(
            "https://www.instagram.com/oauth/authorize",
            &[
                ("client_id", text(config, "clientId")),
                ("redirect_uri", callback_url),
                ("response_type", "code"),
                ("state", state),
                ("scope", &BUSINESS_SCOPES.join(",")),
            ],
        );
    }
    // `buildFacebookDialogUrl` (no `auth_type`).
    query_url(
        &format!(
            "https://www.facebook.com/{}/dialog/oauth",
            facebook_version(config)
        ),
        &[
            ("client_id", text(config, "clientId")),
            ("redirect_uri", callback_url),
            ("scope", &FACEBOOK_SCOPES.join(",")),
            ("response_type", "code"),
            ("state", state),
        ],
    )
}

/// `exchangeCode`. Instagram login: the short token (form POST), the
/// long-lived one (`ig_exchange_token`), then `GET /me` (a personal account
/// is refused). Instagram-Facebook: the Facebook short and long tokens.
pub async fn exchange_code(
    upstream: &impl Upstream,
    provider: &str,
    config: &Value,
    callback_url: &str,
    code: &str,
) -> std::result::Result<Value, ProviderFailure> {
    if provider == "instagramFacebook" {
        let endpoint = format!("{FACEBOOK_GRAPH}/{}/oauth/access_token", facebook_version(config));
        let short = call(
            upstream,
            get(query_url(
                &endpoint,
                &[
                    ("client_id", text(config, "clientId")),
                    ("client_secret", text(config, "clientSecret")),
                    ("redirect_uri", callback_url),
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
        return Ok(oauth2_auth(
            config,
            callback_url,
            json!({ "accessToken": text(&long, "access_token") }),
            None,
        ));
    }
    let form = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("client_id", text(config, "clientId")),
            ("client_secret", text(config, "clientSecret")),
            ("redirect_uri", callback_url),
            ("code", code),
            ("grant_type", "authorization_code"),
        ])
        .finish();
    let short = call(
        upstream,
        UpstreamRequest {
            method: "POST",
            url: format!("{INSTAGRAM_OAUTH}/oauth/access_token"),
            headers: vec![(
                "content-type".into(),
                "application/x-www-form-urlencoded".into(),
            )],
            body: Some(form),
        },
    )
    .await?;
    let long = call(
        upstream,
        get(query_url(
            &format!("{INSTAGRAM_GRAPH}/access_token"),
            &[
                ("grant_type", "ig_exchange_token"),
                ("client_secret", text(config, "clientSecret")),
                ("access_token", text(&short, "access_token")),
            ],
        )),
    )
    .await?;
    let token = text(&long, "access_token").to_string();
    let me = call(
        upstream,
        get(query_url(
            &format!("{INSTAGRAM_GRAPH}/me"),
            &[
                ("fields", "id,user_id,username,name,profile_picture_url,account_type"),
                ("access_token", &token),
            ],
        )),
    )
    .await?;
    if !VALID_ACCOUNT_TYPES.contains(&text(&me, "account_type")) {
        return Err(ProviderFailure::Rejected(
            "Instagram account is not a supported Business/Creator account.".into(),
        ));
    }
    let username = text(&me, "username");
    let name = Some(text(&me, "name"))
        .filter(|n| !n.is_empty())
        .unwrap_or(username);
    let user_id = match &me["user_id"] {
        Value::Number(n) => n.to_string(),
        other => other.as_str().unwrap_or_default().to_string(),
    };
    let mut metadata = json!({
        "igId": user_id,
        "igName": name,
        "pageId": text(&me, "id"),
        "username": username,
    });
    if let Some(version) = config["version"].as_str().filter(|v| !v.is_empty()) {
        metadata["version"] = json!(version);
    }
    Ok(oauth2_auth(
        config,
        callback_url,
        json!({ "accessToken": token }),
        Some(metadata),
    ))
}

/// `listCandidates` of Instagram-Facebook (`getUserInstagramAccounts`): the
/// user's pages with a linked business account; an account that does not
/// load is skipped. Each candidate carries its page token.
pub async fn list_facebook_candidates(
    upstream: &impl Upstream,
    auth: &Value,
) -> std::result::Result<Vec<(String, String, Value)>, ProviderFailure> {
    if auth["authType"] != "oauth2" {
        return Ok(Vec::new());
    }
    let version = facebook_version(auth);
    let token = text(&auth["tokens"], "accessToken").to_string();
    let pages = call(
        upstream,
        get(query_url(
            &format!("{FACEBOOK_GRAPH}/{version}/me/accounts"),
            &[
                ("fields", "id,name,access_token,instagram_business_account"),
                ("access_token", &token),
            ],
        )),
    )
    .await?;
    let mut out = Vec::new();
    for page in pages["data"].as_array().into_iter().flatten() {
        let ig_id = text(&page["instagram_business_account"], "id");
        if ig_id.is_empty() {
            continue;
        }
        let page_token = text(page, "access_token");
        let Ok(account) = call(
            upstream,
            get(query_url(
                &format!("{FACEBOOK_GRAPH}/{version}/{ig_id}"),
                &[
                    ("fields", "id,name,username,profile_picture_url"),
                    ("access_token", page_token),
                ],
            )),
        )
        .await
        else {
            continue;
        };
        let (id, name) = (text(&account, "id"), text(&account, "name"));
        let mut config = auth.clone();
        config["version"] = json!(version);
        let account_auth = oauth2_auth(
            &config,
            text(auth, "redirectUrl"),
            json!({ "accessToken": page_token }),
            Some(json!({
                "igId": id,
                "igName": name,
                "pageId": text(page, "id"),
                "version": version,
                "username": text(&account, "username"),
            })),
        );
        out.push((id.to_string(), name.to_string(), account_auth));
    }
    Ok(out)
}

/// `describe`: `(igId, igName)`.
pub fn describe(auth: &Value) -> Result<(String, String)> {
    let meta = &auth["metadata"];
    let id = text(meta, "igId");
    if id.is_empty() {
        return Err(BusinessError::Repo(chatbotx_db::repo::Error::msg(
            "instagram auth has no account identity",
        )));
    }
    Ok((id.to_string(), text(meta, "igName").to_string()))
}

/// `connectAndPersist`: a new `Inbox`, the `IntegrationInstagram` row
/// (`candidateToConfig` = `pageId`, `username`) and the `connected`
/// `Connection`. An `igId` bound elsewhere (unique) is
/// `connectionAlreadyConnected`.
pub fn persist(
    conn: &Connection,
    provider: &str,
    workspace_id: i64,
    auth: &Value,
    actor_user_id: Option<i64>,
    now: i64,
) -> Result<ConnectionRow> {
    let (ig_id, name) = describe(auth)?;
    let (existing, inbox_id) =
        new_channel_inbox(conn, workspace_id, provider, &ig_id, &name, now)?;
    let meta = &auth["metadata"];
    let kind = if provider == "instagram" {
        "instagram"
    } else {
        "facebook"
    };
    conn.execute(
        r#"INSERT INTO "IntegrationInstagram" ("id", "createdAt", "updatedAt", "auth", "igId",
               "pageId", "name", "username", "conversationStarters", "persistentMenus",
               "workspaceId", "inboxId", "type")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, '[]', '[]', ?8, ?9, ?10)"#,
        params![
            new_id(),
            now,
            auth.to_string(),
            ig_id,
            text(meta, "pageId"),
            name,
            text(meta, "username"),
            workspace_id,
            inbox_id,
            kind
        ],
    )
    .map_err(unique_or_repo)?;
    upsert_channel_connection(
        conn,
        workspace_id,
        provider,
        existing,
        inbox_id,
        &ig_id,
        &name,
        actor_user_id,
        now,
    )
}

/// `webhook.subscribe` (`subscribePageToInstagramWebhook`): `POST
/// /{pageId}/subscribed_apps`; anything but `success: true` fails.
pub async fn subscribe_webhook(
    upstream: &impl Upstream,
    provider: &str,
    auth: &Value,
) -> std::result::Result<(), ProviderFailure> {
    let meta = &auth["metadata"];
    let (base, default, fields) = if provider == "instagram" {
        (INSTAGRAM_GRAPH, INSTAGRAM_VERSION, INSTAGRAM_SUBSCRIBE_FIELDS)
    } else {
        (FACEBOOK_GRAPH, FACEBOOK_VERSION, FACEBOOK_SUBSCRIBE_FIELDS)
    };
    let version = Some(text(meta, "version"))
        .filter(|v| !v.is_empty())
        .unwrap_or(default);
    let reply = call(
        upstream,
        UpstreamRequest {
            method: "POST",
            url: format!("{base}/{version}/{}/subscribed_apps", text(meta, "pageId")),
            headers: vec![
                (
                    "authorization".into(),
                    format!("Bearer {}", text(&auth["tokens"], "accessToken")),
                ),
                ("content-type".into(), "application/json".into()),
            ],
            body: Some(json!({ "subscribed_fields": fields.join(",") }).to_string()),
        },
    )
    .await?;
    if reply["success"] == true {
        Ok(())
    } else {
        Err(ProviderFailure::Rejected(
            "Subscribe failed for Instagram page".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_urls_carry_the_state_and_scopes() {
        let config = json!({ "clientId": "app" });
        let ig = authorize_url("instagram", &config, "https://x/cb", "1.n");
        assert!(ig.starts_with("https://www.instagram.com/oauth/authorize?client_id=app"));
        assert!(ig.contains("state=1.n") && ig.contains("instagram_business_basic"));
        let fb = authorize_url("instagramFacebook", &config, "https://x/cb", "1.n");
        assert!(fb.starts_with("https://www.facebook.com/v23.0/dialog/oauth?"));
        assert!(fb.contains("pages_messaging") && !fb.contains("auth_type"));
    }

    #[test]
    fn describe_needs_the_ig_id() {
        let auth = json!({ "metadata": { "igId": "17", "igName": "Shop" } });
        assert_eq!(describe(&auth).unwrap(), ("17".into(), "Shop".into()));
        assert!(describe(&json!({})).is_err());
    }
}
