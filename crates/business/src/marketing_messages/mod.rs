//! Facebook Marketing Messages: `features/facebook-marketing-messages`
//! (`lib/{grant,campaign-operations,read-operations}.ts`), the business
//! services `facebook-marketing-messages`, `facebook-marketing-message-delivery`
//! and the Graph calls of `integration-facebook-ads`.
//! ponytail: no audit rows (the other Q.D7 services write none either).

pub mod audiences;
pub mod campaigns;
pub mod content;
pub mod customer_file;
pub mod delivery;
pub mod graph;
pub mod statistics;

use chatbotx_db::Db;
use chatbotx_db::repo::facebook_marketing_message as repo;
use chatbotx_db::repo::models::FacebookMarketingMessagesAuth;
use chatbotx_db::repo::now_ms;
use serde_json::Value;

use crate::errors::{BusinessError, Result};
use crate::meta_catalog::process_encryption;
use crate::tx;

/// `facebookMarketingMessages.errors.*` (en).
fn message_of(code: &str) -> &'static str {
    match code {
        "mmNotFound" => "Marketing message not found.",
        "mmInvalidState" => "This action is not available in the campaign's current state.",
        "mmCampaignEnded" => {
            "The Meta message campaign has ended, so it cannot be sent or resumed."
        }
        "mmGrantInvalid" => "Connect Marketing Messages in the ChatbotX app first.",
        "mmPageNotConnected" => "The Page for this campaign is not connected to this workspace.",
        "mmContentLocked" => "The content cannot be changed after the first message has been sent.",
        _ => "The request could not be processed",
    }
}

/// `withMmDeliveryErrors(mmDeliveryError(code))`: the translated message,
/// 404 for `mmNotFound`, else 400.
pub fn mm_error(code: &str) -> BusinessError {
    let status = if code == "mmNotFound" { 404 } else { 400 };
    BusinessError::exception(message_of(code), code, status)
}

/// A `ChatbotXException(t(...), code)` thrown outside `withMmDeliveryErrors` (400).
pub fn mm_error_400(code: &str) -> BusinessError {
    BusinessError::exception(message_of(code), code, 400)
}

/// `new ChatbotXException(message)` (`systemError`, 400).
pub fn system_error(message: impl Into<String>) -> BusinessError {
    BusinessError::exception(message, "systemError", 400)
}

/// A usable grant (`MarketingMessagesGrant` `valid`).
#[derive(Debug, Clone, PartialEq)]
pub struct Grant {
    pub access_token: String,
    pub version: String,
    pub facebook_user_id: Option<String>,
}

async fn auth_row(db: &Db, workspace_id: i64) -> Result<Option<FacebookMarketingMessagesAuth>> {
    tx::read(db, move |conn| {
        Ok(repo::auth_find_by_workspace_id(conn, workspace_id)?)
    })
    .await
}

/// `decryptAuth` (`facebookAdsAuthSchema`): `(accessToken, version)`, `None`
/// for an undecryptable blob.
fn decrypt_auth(row: &FacebookMarketingMessagesAuth) -> Option<(String, Option<String>)> {
    let data = serde_json::from_value(row.auth.0.clone()).ok()?;
    let auth: Value = process_encryption().decrypt_object(&data, None).ok()?;
    let token = auth["accessToken"].as_str().filter(|t| !t.is_empty())?;
    Some((
        token.to_owned(),
        auth["version"].as_str().map(str::to_owned),
    ))
}

/// `markAuthInvalid`.
pub async fn mark_invalid(db: &Db, workspace_id: i64) -> Result<()> {
    tx::write(db, move |tx| {
        repo::auth_update_status(tx, workspace_id, "invalid")?;
        Ok(())
    })
    .await
}

/// The owner's Messenger app (`clientId|clientSecret`, `version`).
async fn messenger_app(db: &Db, workspace_id: i64) -> Result<Option<(String, Option<String>)>> {
    let owner =
        crate::tenant::resolve_credential_owner_id_for_workspace(db, workspace_id, None).await?;
    tx::read(db, move |conn| {
        let credential = crate::platform_credential::resolve_for_owner(
            conn,
            process_encryption(),
            owner,
            "messenger",
            false,
        )?;
        Ok(credential.map(|c| {
            (
                format!(
                    "{}|{}",
                    c.text("clientId").unwrap_or_default(),
                    c.text("clientSecret").unwrap_or_default()
                ),
                c.text("version").map(str::to_owned),
            )
        }))
    })
    .await
}

/// `resolveMarketingMessagesGrant`: `Some` only for a valid grant. Every
/// failure degrades to `None`; a token Graph reports dead (or without the
/// scope) marks the grant invalid, a failed `/debug_token` does not.
pub async fn resolve_grant(db: &Db, workspace_id: i64) -> Result<Option<Grant>> {
    let Some(row) = auth_row(db, workspace_id).await? else {
        return Ok(None);
    };
    if row.status == "invalid" || row.token_expires_at.is_some_and(|at| at <= now_ms()) {
        return Ok(None);
    }
    let Some((token, auth_version)) = decrypt_auth(&row) else {
        return Ok(None);
    };
    let Some((app_token, app_version)) = messenger_app(db, workspace_id).await? else {
        return Ok(None);
    };
    let version = auth_version
        .or(app_version)
        .unwrap_or_else(|| graph::VERSION.to_owned());
    match graph::debug_token(&token, &app_token, &version).await {
        Ok(data)
            if data["is_valid"] == Value::Bool(false) || !graph::has_marketing_messages(&data) =>
        {
            mark_invalid(db, workspace_id).await?;
            Ok(None)
        }
        Ok(_) => Ok(Some(Grant {
            access_token: token,
            version,
            facebook_user_id: row.facebook_user_id,
        })),
        Err(error) => {
            tracing::warn!(message = %error.message, "Marketing Messages debug_token failed");
            Ok(None)
        }
    }
}

/// The grant state the screens show: `missing` (never granted), `valid`
/// or `expired` (every other outcome of [`resolve_grant`]).
pub async fn grant_state(db: &Db, workspace_id: i64) -> Result<&'static str> {
    if auth_row(db, workspace_id).await?.is_none() {
        return Ok("missing");
    }
    Ok(if resolve_grant(db, workspace_id).await?.is_some() {
        "valid"
    } else {
        "expired"
    })
}

/// `requireValidGrant`.
pub async fn require_grant(db: &Db, workspace_id: i64) -> Result<Grant> {
    resolve_grant(db, workspace_id)
        .await?
        .ok_or_else(|| mm_error_400("mmGrantInvalid"))
}

/// `resolveGrantToken`: the stored token of an active grant (no Graph call).
pub async fn grant_token(db: &Db, workspace_id: i64) -> Result<Option<String>> {
    Ok(auth_row(db, workspace_id)
        .await?
        .filter(|row| row.status == "active")
        .as_ref()
        .and_then(decrypt_auth)
        .map(|(token, _)| token))
}

/// `resolveMarketingMessagePageToken`: `(accessToken, version)` of the
/// workspace's Messenger integration for the Page.
pub async fn page_token(
    db: &Db,
    workspace_id: i64,
    page_id: &str,
) -> Result<Option<(String, String)>> {
    let page_id = page_id.to_owned();
    let raw: Option<String> = tx::read(db, move |conn| {
        use chatbotx_db::rusqlite::OptionalExtension;
        Ok(conn
            .query_row(
                "SELECT \"auth\" FROM \"IntegrationMessenger\" WHERE \"workspaceId\" = ?1 AND \"pageId\" = ?2 LIMIT 1",
                chatbotx_db::rusqlite::params![workspace_id, page_id],
                |r| r.get(0),
            )
            .optional()?)
    })
    .await?;
    let Some(auth) = raw.and_then(|raw| crate::email_marketing::open_auth(&raw)) else {
        return Ok(None);
    };
    let Some(token) = auth["tokens"]["accessToken"]
        .as_str()
        .filter(|t| !t.is_empty())
    else {
        return Ok(None);
    };
    let version = auth["metadata"]["version"]
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or(graph::VERSION);
    Ok(Some((token.to_owned(), version.to_owned())))
}
