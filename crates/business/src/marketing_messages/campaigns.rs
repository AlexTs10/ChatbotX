//! The campaign routes: `lib/{campaign-operations,read-operations}.ts`, the
//! `facebookMarketingMessagesService` and `getMarketingMessageStats`.
//! Meta first, database second: a row is stored only once Graph returned
//! the message campaign id, and deleted only once Meta deleted it.

use chatbotx_db::Db;
use chatbotx_db::repo::facebook_marketing_message::{
    self as repo, CreateInput as RepoCreate, UpdateInput,
};
use chatbotx_db::repo::facebook_marketing_message_audience as audiences;
use chatbotx_db::repo::facebook_marketing_message_recipient as recipients;
use chatbotx_db::repo::models::FacebookMarketingMessage;
use chatbotx_db::repo::{iso, new_id, now_ms};
use serde_json::{Value, json};

use super::{
    Grant, content, grant_token, graph, mm_error, mm_error_400, require_grant, system_error,
};
use crate::errors::Result;
use crate::tx;

/// `MM_START_DELAY_MS`.
const START_DELAY_MS: i64 = 60 * 60 * 1000;
/// Meta's ad-account currencies with offset 1 (not ISO 4217: `TWD` is 1).
const ZERO_DECIMAL: &[&str] = &[
    "CLP", "COP", "CRC", "HUF", "IDR", "ISK", "JPY", "KRW", "PYG", "TWD", "VND",
];

/// `currencyOffset`.
pub fn currency_offset(currency: &str) -> i64 {
    if ZERO_DECIMAL.contains(&currency.to_uppercase().as_str()) {
        1
    } else {
        100
    }
}

/// `toMinorUnits` (`Math.round`).
pub fn to_minor_units(major: f64, currency: &str) -> i64 {
    (major * currency_offset(currency) as f64 + 0.5).floor() as i64
}

fn opt_iso(ms: Option<i64>) -> Value {
    ms.map_or(Value::Null, |ms| json!(iso(ms)))
}

/// `toCampaignResource`.
pub fn resource(row: &FacebookMarketingMessage) -> Value {
    json!({
        "id": row.id.to_string(),
        "name": row.name,
        "pageId": row.page_id,
        "adAccountId": row.ad_account_id,
        "currency": row.currency,
        "budgetType": row.budget_type,
        "budgetMinorUnits": row.budget_minor_units,
        "content": row.content.0,
        "targetType": row.target_type,
        "audienceId": row.audience_id.map(|id| id.to_string()),
        "campaignId": row.campaign_id,
        "status": row.status,
        "statusReason": row.status_reason,
        "statusErrorCode": row.status_error_code,
        "statusErrorMessage": row.status_error_message,
        "startsAt": opt_iso(row.starts_at),
        "startedAt": opt_iso(row.started_at),
        "firstSentAt": opt_iso(row.first_sent_at),
        "completedAt": opt_iso(row.completed_at),
        "nextDayAt": opt_iso(row.next_day_at),
        "tokenCount": row.token_count,
        "createdAt": iso(row.created_at),
    })
}

/// `listMarketingMessagesPages`: the workspace's Messenger Pages (no Graph call).
pub async fn list_pages(db: &Db, workspace_id: i64) -> Result<Vec<Value>> {
    tx::read(db, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT \"pageId\", \"name\" FROM \"IntegrationMessenger\" WHERE \"workspaceId\" = ?1 ORDER BY \"createdAt\", \"id\"",
        )?;
        let rows = stmt
            .query_map([workspace_id], |r| {
                Ok(json!({ "pageId": r.get::<_, String>(0)?, "name": r.get::<_, Option<String>>(1)? }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
}

/// `listMarketingMessagesAdAccounts`.
pub async fn list_ad_accounts(db: &Db, workspace_id: i64) -> Result<Vec<Value>> {
    let grant = require_grant(db, workspace_id).await?;
    let accounts = graph::mm_ad_accounts(&grant.access_token, &grant.version).await?;
    Ok(accounts.iter().map(graph::ad_account_option).collect())
}

/// `requireTosAcceptedAdAccount`.
pub async fn require_tos_account(grant: &Grant, ad_account_id: &str) -> Result<graph::AdAccount> {
    let accounts = graph::mm_ad_accounts(&grant.access_token, &grant.version).await?;
    let account = accounts
        .into_iter()
        .find(|a| a.id == ad_account_id)
        .ok_or_else(|| {
            system_error("Please grant access to use the Marketing Messages feature.")
        })?;
    if !graph::tos_accepted(&account.tos_accepted) {
        return Err(system_error(format!(
            "To use this feature, please accept the custom object terms at {}",
            graph::tos_url(&account.account_id)
        )));
    }
    Ok(account)
}

/// `list` (createdAt DESC).
pub async fn list(db: &Db, workspace_id: i64) -> Result<Vec<FacebookMarketingMessage>> {
    tx::read(db, move |conn| {
        Ok(repo::list_by_workspace_id(conn, workspace_id)?)
    })
    .await
}

/// `find` (`None` for a non-numeric or foreign id).
pub async fn find(
    db: &Db,
    workspace_id: i64,
    id: &str,
) -> Result<Option<FacebookMarketingMessage>> {
    let Ok(id) = id.parse::<i64>() else {
        return Ok(None);
    };
    tx::read(db, move |conn| {
        Ok(repo::find_for_workspace(conn, id, workspace_id)?)
    })
    .await
}

/// `getMarketingMessageStats`: `{ counts, errors }`.
pub async fn stats(db: &Db, campaign_row_id: i64) -> Result<Value> {
    tx::read(db, move |conn| {
        let c = recipients::count_outcomes(conn, campaign_row_id)?;
        let errors: Vec<Value> = recipients::error_breakdown(conn, campaign_row_id)?
            .into_iter()
            .map(|e| json!({ "errorCode": e.error_code, "errorMessage": e.error_message, "count": e.count }))
            .collect();
        Ok(json!({
            "counts": {
                "pending": c.pending,
                "sent": c.sent + c.delivered,
                "delivered": c.delivered,
                "failed": c.failed,
                "unconfirmed": c.unconfirmed,
            },
            "errors": errors,
        }))
    })
    .await
}

/// `campaigns.get`: the resource with `stats` (404 `mmNotFound`).
pub async fn get(db: &Db, workspace_id: i64, id: &str) -> Result<Value> {
    let row = find(db, workspace_id, id)
        .await?
        .ok_or_else(|| mm_error("mmNotFound"))?;
    let mut out = resource(&row);
    out["stats"] = stats(db, row.id).await?;
    Ok(out)
}

/// `resolveContentAttachments`: a media template gets the Page's reusable
/// attachment id (kept when the URL did not change).
async fn resolve_attachments(
    db: &Db,
    workspace_id: i64,
    page_id: &str,
    mut content: Value,
    previous: Option<&Value>,
) -> Result<Value> {
    if content["templateType"] != "media" {
        return Ok(content);
    }
    let url = content["media"]["url"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if let Some(prev) = previous.filter(|p| p["templateType"] == "media")
        && let Some(id) = prev["media"]["attachmentId"]
            .as_str()
            .filter(|s| !s.is_empty())
        && prev["media"]["url"] == json!(url)
    {
        content["media"]["attachmentId"] = json!(id);
        return Ok(content);
    }
    let (token, version) = super::page_token(db, workspace_id, page_id)
        .await?
        .ok_or_else(|| {
            crate::ads::graph::internal(&format!(
                "No Messenger integration for page {page_id} in workspace {workspace_id}"
            ))
        })?;
    let media_type = content["mediaType"].as_str().unwrap_or("image").to_owned();
    let id = graph::upload_attachment(&token, &version, &url, &media_type)
        .await?
        .ok_or_else(|| {
            crate::ads::graph::internal("Facebook returned no attachment id for the uploaded media")
        })?;
    content["media"]["attachmentId"] = json!(id);
    Ok(content)
}

/// `createMarketingMessageSchema` (parsed).
#[derive(Debug, Clone)]
pub struct CreateInput {
    pub name: String,
    pub page_id: String,
    pub ad_account_id: String,
    pub budget_type: String,
    pub budget_major_units: f64,
    pub content: Value,
    pub target_type: String,
    pub audience_id: Option<String>,
}

/// `createMarketingMessage`.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    input: CreateInput,
) -> Result<FacebookMarketingMessage> {
    let grant = require_grant(db, workspace_id).await?;
    let account = require_tos_account(&grant, &input.ad_account_id).await?;
    let mut audience_id = None;
    if input.target_type == "customAudience" {
        let wanted = input
            .audience_id
            .as_deref()
            .and_then(|id| id.parse::<i64>().ok());
        let (act, page) = (input.ad_account_id.clone(), input.page_id.clone());
        let audience = match wanted {
            Some(id) => {
                tx::read(db, move |conn| {
                    Ok(audiences::find_for_workspace(conn, id, workspace_id)?)
                })
                .await?
            }
            None => None,
        };
        let audience = audience
            .filter(|a| a.ad_account_id == act && a.page_id == page && a.ready_at.is_some())
            .ok_or_else(|| {
                system_error(
                    "The selected custom audience is not ready or does not belong to this Page and ad account.",
                )
            })?;
        audience_id = Some(audience.id);
    }
    let content =
        resolve_attachments(db, workspace_id, &input.page_id, input.content, None).await?;
    let minor = to_minor_units(input.budget_major_units, &account.currency);
    let campaign_id = graph::create_message_campaign(
        &grant.access_token,
        &grant.version,
        &input.ad_account_id,
        &input.name,
        &input.page_id,
        &input.budget_type,
        minor,
    )
    .await?;
    let row = RepoCreate {
        id: new_id(),
        workspace_id,
        name: input.name,
        page_id: input.page_id,
        ad_account_id: input.ad_account_id,
        currency_offset: currency_offset(&account.currency),
        currency: account.currency,
        budget_type: input.budget_type,
        budget_minor_units: minor,
        content,
        target_type: input.target_type,
        audience_id,
        campaign_id,
        facebook_user_id: grant.facebook_user_id,
        status: Some("scheduled".to_owned()),
        starts_at: Some(now_ms() + START_DELAY_MS),
    };
    tx::write(db, move |tx| Ok(repo::create(tx, &row)?)).await
}

/// `isContentLocked`.
fn content_locked(row: &FacebookMarketingMessage) -> bool {
    row.first_sent_at.is_some() || !matches!(row.status.as_str(), "notSent" | "scheduled")
}

/// `updateMarketingMessage`: the content lock first (no side effect), then
/// the grant, Meta, the row. `adAccountId`, `pageId`, `currency` and
/// `budgetType` are read from the row.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: &str,
    name: String,
    budget_major_units: f64,
    content: Value,
) -> Result<FacebookMarketingMessage> {
    let existing = find(db, workspace_id, id)
        .await?
        .ok_or_else(|| mm_error_400("mmNotFound"))?;
    if content_locked(&existing) && content::content_changed(&existing.content.0, &content) {
        return Err(mm_error("mmContentLocked"));
    }
    let grant = require_grant(db, workspace_id).await?;
    let content = resolve_attachments(
        db,
        workspace_id,
        &existing.page_id,
        content,
        Some(&existing.content.0),
    )
    .await?;
    let minor = to_minor_units(budget_major_units, &existing.currency);
    graph::update_message_campaign(
        &grant.access_token,
        &grant.version,
        &existing.campaign_id,
        &name,
        &existing.budget_type,
        minor,
    )
    .await?;
    let changes_content = content::content_changed(&existing.content.0, &content);
    let input = UpdateInput {
        id: existing.id,
        workspace_id,
        name,
        budget_type: existing.budget_type.clone(),
        budget_minor_units: minor,
        content: changes_content.then_some(content),
        lock_content: changes_content,
    };
    let row = tx::write(db, move |tx| Ok(repo::update(tx, &input)?)).await?;
    match row {
        Some(row) => Ok(row),
        None if changes_content => Err(mm_error("mmContentLocked")),
        None => Err(mm_error("mmNotFound")),
    }
}

/// `deleteMarketingMessages` (one id): no grant refuses, Meta first; Meta's
/// "object does not exist" (100/33) counts as deleted. An unknown id is a no-op.
pub async fn delete(db: &Db, workspace_id: i64, id: &str) -> Result<()> {
    let Some(row) = find(db, workspace_id, id).await? else {
        return Ok(());
    };
    let token = grant_token(db, workspace_id)
        .await?
        .ok_or_else(|| mm_error_400("mmGrantInvalid"))?;
    if let Err(error) = graph::delete_message_campaign(&token, &row.campaign_id).await
        && !(error.code == Some(100) && error.subcode.as_deref() == Some("33"))
    {
        return Err(crate::errors::BusinessError::exception(
            format!(
                "Meta could not delete this campaign, so it was kept. {}",
                error.message
            ),
            "mmMetaDeleteFailed",
            400,
        ));
    }
    let row_id = row.id;
    tx::write(db, move |tx| Ok(repo::remove(tx, &[row_id], workspace_id)?)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minor_units_use_metas_offsets() {
        assert_eq!(to_minor_units(30.5, "USD"), 3050);
        assert_eq!(to_minor_units(150_000.0, "vnd"), 150_000);
        assert_eq!(to_minor_units(12.34, "EUR"), 1234);
        assert_eq!(currency_offset("TWD"), 1);
    }
}
