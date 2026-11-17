//! The TikTok and Zalo `connection` adapters
//! (`integrations/tiktok/src/integration.ts` with `apis/auth.ts`,
//! `apis/user.ts`, `lib/scopes.ts`; `integrations/zalo/src/integration.ts`
//! with `api/auth.ts`) for the OAuth connect engine in [`super::oauth`]:
//! authorize URL, code exchange, `describe` and the `connectAndPersist`
//! satellite rows (`IntegrationTiktok` / `IntegrationZalo`, `keep_row`).

use chatbotx_db::repo::new_id;
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::credentials::{Upstream, UpstreamRequest};
use super::oauth::{ProviderFailure, call, get, oauth2_auth, query_url, text};
use super::{ConnectionRow, repo_err};
use crate::{BusinessError, Result};

const TIKTOK_AUTHORIZE: &str = "https://www.tiktok.com/v2/auth/authorize/";
const TIKTOK_API: &str = "https://business-api.tiktok.com/open_api/v1.3/";
const ZALO_OAUTH: &str = "https://oauth.zaloapp.com";
const ZALO_API: &str = "https://openapi.zalo.me";

const TIKTOK_IDENTITY: &[&str] = &["user.info.basic", "user.info.username"];
const TIKTOK_MESSAGING: &[&str] = &[
    "message.list.read",
    "message.list.send",
    "message.list.manage",
];
const TIKTOK_PROFILE: &[&str] = &["user.info.profile", "user.info.stats", "user.account.type"];
const TIKTOK_COMMENT_CORE: &[&str] = &["comment.list", "comment.list.manage"];
const TIKTOK_COMMENT_AUTOMATION: &[&str] = &["comment.list", "comment.list.manage", "video.list"];

/// `true` for the providers of this module.
pub fn handles(provider: &str) -> bool {
    matches!(provider, "tiktok" | "zalo")
}

/// The satellite table and its identity column.
pub fn satellite(provider: &str) -> Option<(&'static str, &'static str)> {
    match provider {
        "tiktok" => Some(("IntegrationTiktok", "openId")),
        "zalo" => Some(("IntegrationZalo", "oaId")),
        _ => None,
    }
}

/// `resolveTiktokFeatures`: `(messaging, comments)`, both default on.
fn tiktok_features(config: &Value) -> (bool, bool) {
    let flag = |key: &str| config["features"][key].as_bool().unwrap_or(true);
    (flag("messaging"), flag("comments"))
}

/// `tiktokRequestedScopes`.
pub fn tiktok_requested_scopes(config: &Value) -> String {
    let (messaging, comments) = tiktok_features(config);
    let mut scopes: Vec<&str> = TIKTOK_IDENTITY.to_vec();
    if messaging {
        scopes.extend(TIKTOK_MESSAGING);
    }
    scopes.extend(TIKTOK_PROFILE);
    if comments {
        scopes.extend(TIKTOK_COMMENT_AUTOMATION);
    }
    scopes.join(",")
}

/// `tiktokRequiredScopes`.
fn tiktok_required_scopes(config: &Value) -> Vec<&'static str> {
    if tiktok_features(config).0 {
        [TIKTOK_IDENTITY, TIKTOK_MESSAGING].concat()
    } else {
        [TIKTOK_IDENTITY, TIKTOK_COMMENT_CORE].concat()
    }
}

/// `parseTiktokScopes`.
fn parse_scopes(scope: &str) -> Vec<String> {
    scope
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// `assertTiktokGrant`: the granted scopes, or the missing required ones.
pub fn assert_tiktok_grant(
    scope: &str,
    config: &Value,
) -> std::result::Result<Vec<String>, Vec<&'static str>> {
    let granted = parse_scopes(scope);
    let missing: Vec<&str> = tiktok_required_scopes(config)
        .into_iter()
        .filter(|s| !granted.iter().any(|g| g == s))
        .collect();
    if missing.is_empty() {
        Ok(granted)
    } else {
        Err(missing)
    }
}

/// `authorizeUrl`.
pub fn authorize_url(provider: &str, config: &Value, callback_url: &str, state: &str) -> String {
    match provider {
        "tiktok" => query_url(
            TIKTOK_AUTHORIZE,
            &[
                ("client_key", text(config, "clientId")),
                ("response_type", "code"),
                ("scope", &tiktok_requested_scopes(config)),
                ("redirect_uri", callback_url),
                ("disable_auto_auth", "1"),
                ("state", state),
            ],
        ),
        _ => query_url(
            &format!("{ZALO_OAUTH}/v4/oa/permission"),
            &[
                ("app_id", text(config, "clientId")),
                ("redirect_uri", callback_url),
                ("state", state),
            ],
        ),
    }
}

/// `now + expiresIn * 1000` as an ISO string (`expires_in` may be a numeric
/// string, as Zalo sends it; JS coerces it).
fn iso_in(expires_in: &Value) -> Value {
    let seconds = expires_in
        .as_i64()
        .or_else(|| expires_in.as_str().and_then(|s| s.trim().parse().ok()))
        .unwrap_or_default();
    let at = chatbotx_db::repo::now_ms() + seconds.saturating_mul(1000);
    json!(chatbotx_core::js::to_iso_string(at))
}

/// A TikTok Business API answer with `code != 0` (`TiktokAPIException`,
/// status 400: a rejection).
fn tiktok_ok(answer: Value, fallback: &str) -> std::result::Result<Value, ProviderFailure> {
    if answer["code"].as_i64() == Some(0) {
        return Ok(answer);
    }
    let message = answer["message"].as_str().unwrap_or(fallback);
    Err(ProviderFailure::Rejected(message.to_string()))
}

/// A Zalo answer with a non-zero `error` or an `error_name`
/// (`ZaloHttpClient.handleResponse`, status 400: a rejection).
fn zalo_ok(answer: Value) -> std::result::Result<Value, ProviderFailure> {
    let numeric = match &answer["error"] {
        Value::Null => false,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => s.trim().parse::<f64>() != Ok(0.0),
        Value::Bool(b) => *b,
        _ => true,
    };
    if !(numeric || answer["error_name"].is_string()) {
        return Ok(answer);
    }
    let message = ["message", "error_description", "error_reason", "error_name"]
        .iter()
        .find_map(|key| answer[*key].as_str())
        .unwrap_or("Zalo OA API error");
    Err(ProviderFailure::Rejected(message.to_string()))
}

/// `exchangeCode`.
pub async fn exchange_code(
    upstream: &impl Upstream,
    provider: &str,
    config: &Value,
    callback_url: &str,
    code: &str,
) -> std::result::Result<Value, ProviderFailure> {
    match provider {
        "tiktok" => tiktok_exchange(upstream, config, callback_url, code).await,
        _ => zalo_exchange(upstream, config, callback_url, code).await,
    }
}

async fn tiktok_exchange(
    upstream: &impl Upstream,
    config: &Value,
    callback_url: &str,
    code: &str,
) -> std::result::Result<Value, ProviderFailure> {
    let body = json!({
        "client_id": config["clientId"],
        "client_secret": config["clientSecret"],
        "grant_type": "authorization_code",
        "auth_code": code,
        "redirect_uri": callback_url,
    });
    let token = call(
        upstream,
        UpstreamRequest {
            method: "POST",
            url: format!("{TIKTOK_API}tt_user/oauth2/token/"),
            headers: vec![("content-type".into(), "application/json".into())],
            body: Some(body.to_string()),
        },
    )
    .await?;
    let token = tiktok_ok(token, "Token exchange failed")?;
    let data = &token["data"];
    let scopes = assert_tiktok_grant(text(data, "scope"), config).map_err(|missing| {
        ProviderFailure::Rejected(format!(
            "TikTok authorization is missing required scopes: {}",
            missing.join(", ")
        ))
    })?;
    let access_token = text(data, "access_token");
    let mut request = get(query_url(
        &format!("{TIKTOK_API}user/info/"),
        &[("fields", "open_id,display_name,avatar_url,username")],
    ));
    request
        .headers
        .push(("Access-Token".into(), access_token.to_string()));
    let user = tiktok_ok(call(upstream, request).await?, "TikTok API error")?;
    let user = &user["data"]["user"];
    let (messaging, comments) = tiktok_features(config);
    Ok(oauth2_auth(
        config,
        callback_url,
        json!({
            "accessToken": access_token,
            "refreshToken": data["refresh_token"],
            "expiresAt": iso_in(&data["expires_in"]),
            "refreshTokenExpiresAt": iso_in(&data["refresh_token_expires_in"]),
        }),
        Some(json!({
            "openId": data["open_id"],
            "username": user["username"],
            "displayName": user["display_name"],
            "scopes": scopes,
            "features": { "messaging": messaging, "comments": comments },
        })),
    ))
}

async fn zalo_exchange(
    upstream: &impl Upstream,
    config: &Value,
    callback_url: &str,
    code: &str,
) -> std::result::Result<Value, ProviderFailure> {
    let form = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("code", code)
        .append_pair("app_id", text(config, "clientId"))
        .append_pair("redirect_uri", callback_url)
        .append_pair("grant_type", "authorization_code")
        .finish();
    let tokens = call(
        upstream,
        UpstreamRequest {
            method: "POST",
            url: format!("{ZALO_OAUTH}/v4/oa/access_token"),
            headers: vec![
                (
                    "content-type".into(),
                    "application/x-www-form-urlencoded".into(),
                ),
                ("secret_key".into(), text(config, "clientSecret").to_string()),
            ],
            body: Some(form),
        },
    )
    .await?;
    let tokens = zalo_ok(tokens)?;
    let access_token = text(&tokens, "access_token");
    let mut request = get(format!("{ZALO_API}/v2.0/oa/getoa"));
    request
        .headers
        .push(("access_token".into(), access_token.to_string()));
    let profile = zalo_ok(call(upstream, request).await?)?;
    let profile = &profile["data"];
    let mut metadata = json!({ "oaName": profile["name"] });
    if let Some(version) = config["version"].as_str() {
        metadata["version"] = json!(version);
    }
    let mut auth = oauth2_auth(
        config,
        callback_url,
        json!({
            "accessToken": access_token,
            "refreshToken": tokens["refresh_token"],
            "expiresAt": iso_in(&tokens["expires_in"]),
        }),
        Some(metadata),
    );
    auth["oaId"] = match &profile["oa_id"] {
        Value::Number(n) => json!(n.to_string()),
        other => other.clone(),
    };
    Ok(auth)
}

/// `describe`: `(sourceId, displayName)`.
pub fn describe(provider: &str, auth: &Value) -> Result<(String, String)> {
    let (source, name) = match provider {
        "tiktok" => {
            let meta = &auth["metadata"];
            let name = [text(meta, "displayName"), text(meta, "username")]
                .into_iter()
                .find(|s| !s.is_empty())
                .unwrap_or("TikTok");
            (text(meta, "openId"), name)
        }
        _ => {
            let name = Some(text(&auth["metadata"], "oaName"))
                .filter(|s| !s.is_empty())
                .unwrap_or("Zalo");
            (text(auth, "oaId"), name)
        }
    };
    if source.is_empty() {
        return Err(BusinessError::Repo(chatbotx_db::repo::Error::msg(format!(
            "{provider} auth has no account identity"
        ))));
    }
    Ok((source.to_string(), name.to_string()))
}

/// `connectAndPersist` of a TikTok account / Zalo OA: a new `Inbox`, the
/// satellite row and the `connected` `Connection`. A kept satellite row of
/// the same identity in this workspace (`keep_row` disconnect) moves to the
/// new inbox with the new auth; an identity bound elsewhere (TikTok
/// `openId` unique) is `connectionAlreadyConnected`.
pub fn persist(
    conn: &Connection,
    provider: &str,
    workspace_id: i64,
    auth: &Value,
    actor_user_id: Option<i64>,
    now: i64,
) -> Result<ConnectionRow> {
    let (table, column) = satellite(provider).ok_or_else(|| super::oauth::not_configured(provider))?;
    let (source_id, name) = describe(provider, auth)?;
    let (existing, inbox_id) =
        super::oauth::new_channel_inbox(conn, workspace_id, provider, &source_id, &name, now)?;
    let moved = conn
        .execute(
            &format!(
                r#"UPDATE "{table}" SET "auth" = ?3, "name" = ?4, "inboxId" = ?5,
                       "tokenRefreshError" = NULL, "updatedAt" = ?6
                   WHERE "workspaceId" = ?1 AND "{column}" = ?2"#
            ),
            params![workspace_id, source_id, auth.to_string(), name, inbox_id, now],
        )
        .map_err(repo_err)?;
    if moved == 0 {
        conn.execute(
            &format!(
                r#"INSERT INTO "{table}" ("id", "createdAt", "updatedAt", "auth", "{column}", "name",
                       "workspaceId", "inboxId")
                   VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7)"#
            ),
            params![new_id(), now, auth.to_string(), source_id, name, workspace_id, inbox_id],
        )
        .map_err(super::oauth::unique_or_repo)?;
    }
    super::oauth::upsert_channel_connection(
        conn,
        workspace_id,
        provider,
        existing,
        inbox_id,
        &source_id,
        &name,
        actor_user_id,
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiktok_scopes_follow_the_features() {
        let all = json!({});
        assert_eq!(
            tiktok_requested_scopes(&all),
            "user.info.basic,user.info.username,message.list.read,message.list.send,message.list.manage,user.info.profile,user.info.stats,user.account.type,comment.list,comment.list.manage,video.list"
        );
        let comments_only = json!({ "features": { "messaging": false } });
        assert_eq!(
            tiktok_requested_scopes(&comments_only),
            "user.info.basic,user.info.username,user.info.profile,user.info.stats,user.account.type,comment.list,comment.list.manage,video.list"
        );
        // A DM grant without the comment scopes still connects.
        assert!(
            assert_tiktok_grant(
                "user.info.basic, user.info.username,message.list.read,message.list.send,message.list.manage",
                &all
            )
            .is_ok()
        );
        assert_eq!(
            assert_tiktok_grant("user.info.basic,message.list.read", &all),
            Err(vec![
                "user.info.username",
                "message.list.send",
                "message.list.manage"
            ])
        );
        assert_eq!(
            assert_tiktok_grant("user.info.basic,user.info.username", &comments_only),
            Err(vec!["comment.list", "comment.list.manage"])
        );
    }

    #[test]
    fn authorize_urls_carry_the_state() {
        let config = json!({ "clientId": "ck" });
        let tiktok = authorize_url("tiktok", &config, "https://app.test/integrations/tiktok/callback", "5.n0");
        assert!(tiktok.starts_with("https://www.tiktok.com/v2/auth/authorize/?client_key=ck&response_type=code&scope="));
        assert!(tiktok.ends_with("&redirect_uri=https%3A%2F%2Fapp.test%2Fintegrations%2Ftiktok%2Fcallback&disable_auto_auth=1&state=5.n0"));
        assert_eq!(
            authorize_url("zalo", &config, "https://app.test/integrations/zalo/callback", "5.n0"),
            "https://oauth.zaloapp.com/v4/oa/permission?app_id=ck&redirect_uri=https%3A%2F%2Fapp.test%2Fintegrations%2Fzalo%2Fcallback&state=5.n0"
        );
    }

    #[test]
    fn zalo_errors_are_rejections() {
        assert!(zalo_ok(json!({ "error": 0, "data": {} })).is_ok());
        assert!(zalo_ok(json!({ "access_token": "a" })).is_ok());
        assert_eq!(
            zalo_ok(json!({ "error": -14, "message": "bad code" })),
            Err(ProviderFailure::Rejected("bad code".into()))
        );
        assert_eq!(
            zalo_ok(json!({ "error_name": "invalid_grant", "error_description": "expired" })),
            Err(ProviderFailure::Rejected("expired".into()))
        );
    }

    #[test]
    fn describe_falls_back_to_the_provider_name() {
        let tiktok = json!({ "metadata": { "openId": "o1", "displayName": "", "username": "u1" } });
        assert_eq!(describe("tiktok", &tiktok).unwrap(), ("o1".into(), "u1".into()));
        let zalo = json!({ "oaId": "9", "metadata": {} });
        assert_eq!(describe("zalo", &zalo).unwrap(), ("9".into(), "Zalo".into()));
        assert!(describe("zalo", &json!({})).is_err());
    }
}
