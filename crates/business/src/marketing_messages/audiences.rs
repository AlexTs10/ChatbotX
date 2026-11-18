//! `facebookMarketingMessageAudienceService` (`facebook-marketing-message-audience`).

use std::collections::{HashMap, HashSet};

use chatbotx_db::Db;
use chatbotx_db::repo::facebook_marketing_message_audience::{
    self as repo, CreateInput as RepoCreate, ListFilter,
};
use chatbotx_db::repo::models::FacebookMarketingMessageAudience;
use chatbotx_db::repo::{iso, new_id, now_ms};
use serde_json::{Map, Value, json};

use super::{Grant, customer_file, graph, page_token, require_grant};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `audienceRowSchema`.
pub fn resource(row: &FacebookMarketingMessageAudience) -> Value {
    json!({
        "id": row.id.to_string(),
        "customAudienceId": row.custom_audience_id,
        "adAccountId": row.ad_account_id,
        "pageId": row.page_id,
        "name": row.name,
        "description": row.description,
        "rowsUploaded": row.rows_uploaded,
        "readyAt": row.ready_at.map(iso),
        "createdAt": iso(row.created_at),
    })
}

/// `listMarketingMessagesAudiences`.
pub async fn list(
    db: &Db,
    workspace_id: i64,
    ad_account_id: Option<String>,
    page_id: Option<String>,
    ready_only: bool,
) -> Result<Vec<Value>> {
    let filter = ListFilter {
        ad_account_id,
        page_id,
        ready_only,
    };
    let rows = tx::read(db, move |conn| {
        Ok(repo::list_by_workspace_id(conn, workspace_id, &filter)?)
    })
    .await?;
    Ok(rows.iter().map(resource).collect())
}

/// `audienceError(code)` re-thrown translated (`withAudienceErrors`): a
/// plain `ChatbotXException(message)` (`systemError`, 400).
fn audience_error(message: impl Into<String>) -> BusinessError {
    super::system_error(message)
}

/// `{n, number}` (en): thousands separators.
fn fmt_int(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

fn invalid(message: impl Into<String>) -> Value {
    json!({ "status": "invalid", "message": message.into() })
}

fn partial(received: i64) -> Value {
    json!({
        "status": "partial",
        "received": received,
        "message": format!(
            "The audience was created, but the upload stopped after {} rows. Use \"Upload more rows\" to retry.",
            fmt_int(received)
        ),
    })
}

/// A `ChatbotXException` of the preflight becomes an `invalid` outcome; a
/// Graph (SDK) error is rethrown.
fn preflight_outcome(error: BusinessError) -> Result<Value> {
    match error {
        BusinessError::Exception { message, code, .. } if code != "BAD_REQUEST" => {
            Ok(invalid(message))
        }
        other => Err(other),
    }
}

/// `CustomerFilePayload` from the table, or the `invalid` message.
fn prepare_payload(
    headers: &[String],
    rows: &[Vec<String>],
) -> std::result::Result<customer_file::Payload, &'static str> {
    let columns =
        customer_file::map_headers(headers).ok_or("The file needs an email or phone column.")?;
    let payload = customer_file::build_payload(rows, &columns, chrono::Utc::now());
    if payload.data.is_empty() {
        return Err("No row has a valid email or phone number.");
    }
    Ok(payload)
}

async fn increment(db: &Db, id: i64, workspace_id: i64, by: i64) -> Result<()> {
    tx::write(db, move |tx| {
        Ok(repo::increment_rows_uploaded(tx, id, workspace_id, by)?)
    })
    .await
}

/// `upload`: the session upload and the `rowsUploaded` increment.
async fn upload(
    db: &Db,
    row: &FacebookMarketingMessageAudience,
    grant: &Grant,
    payload: &customer_file::Payload,
) -> Result<Value> {
    let schema: Vec<&str> = payload.schema.to_vec();
    match graph::upload_audience_users(
        &grant.access_token,
        &grant.version,
        &row.custom_audience_id,
        &schema,
        &payload.data,
    )
    .await
    {
        Ok((received, invalid_rows)) => {
            increment(db, row.id, row.workspace_id, received).await?;
            Ok(json!({
                "status": "ok",
                "id": row.id.to_string(),
                "customAudienceId": row.custom_audience_id,
                "received": received,
                "skipped": payload.skipped,
                "invalid": invalid_rows,
            }))
        }
        Err((received, invalid_rows, error)) => {
            if received > 0 {
                if let Err(e) = increment(db, row.id, row.workspace_id, received).await {
                    tracing::error!(error = %e, "Failed to persist the partial audience upload count");
                }
            }
            tracing::error!(message = %error.message, invalid = invalid_rows, received, "Marketing messages audience upload failed mid-session");
            Ok(partial(received))
        }
    }
}

/// `createAudience` input (`createAudiencePublicRequest`).
pub struct CreateInput {
    pub ad_account_id: String,
    pub page_id: String,
    pub name: String,
    pub description: Option<String>,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// `createAudience`: an `UploadOutcome` (`ok` / `partial` / `invalid`).
pub async fn create(db: &Db, workspace_id: i64, input: CreateInput) -> Result<Value> {
    let grant = match require_grant(db, workspace_id).await {
        Ok(grant) => grant,
        Err(e) => return preflight_outcome(e),
    };
    if let Err(e) = super::campaigns::require_tos_account(&grant, &input.ad_account_id).await {
        return preflight_outcome(e);
    }
    if !page_connected(db, workspace_id, &input.page_id).await? {
        return Ok(invalid(
            "The selected Page is not connected to this workspace.",
        ));
    }
    let payload = match prepare_payload(&input.headers, &input.rows) {
        Ok(p) => p,
        Err(message) => return Ok(invalid(message)),
    };
    let description = input.description.filter(|d| !d.is_empty());
    let custom_audience_id = match graph::create_audience(
        &grant.access_token,
        &grant.version,
        &input.ad_account_id,
        &input.page_id,
        &input.name,
        description.as_deref(),
    )
    .await
    {
        Ok(id) => id,
        Err(e) if e.code.is_some() => {
            return Ok(invalid(format!(
                "Meta rejected the audience: {}",
                e.message
            )));
        }
        Err(e) => return Err(e.into()),
    };
    let row = RepoCreate {
        id: new_id(),
        workspace_id,
        custom_audience_id: custom_audience_id.clone(),
        ad_account_id: input.ad_account_id,
        page_id: input.page_id,
        name: input.name,
        description,
        facebook_user_id: grant.facebook_user_id.clone(),
    };
    let stored = match tx::write(db, move |tx| Ok(repo::create(tx, &row)?)).await {
        Ok(stored) => stored,
        Err(error) => {
            tracing::error!(%error, workspace_id, custom_audience_id, "Marketing messages audience failed after it was created on Meta");
            return Ok(partial(0));
        }
    };
    upload(db, &stored, &grant, &payload).await
}

async fn page_connected(db: &Db, workspace_id: i64, page_id: &str) -> Result<bool> {
    let page_id = page_id.to_owned();
    tx::read(db, move |conn| {
        Ok(conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM \"IntegrationMessenger\" WHERE \"workspaceId\" = ?1 AND \"pageId\" = ?2)",
            chatbotx_db::rusqlite::params![workspace_id, page_id],
            |r| r.get::<_, bool>(0),
        )?)
    })
    .await
}

async fn find(
    db: &Db,
    workspace_id: i64,
    id: &str,
) -> Result<Option<FacebookMarketingMessageAudience>> {
    let Ok(id) = id.parse::<i64>() else {
        return Ok(None);
    };
    tx::read(db, move |conn| {
        Ok(repo::find_for_workspace(conn, id, workspace_id)?)
    })
    .await
}

/// `uploadMoreAudience`: the grant is required (thrown), the rest is an outcome.
pub async fn upload_more(
    db: &Db,
    workspace_id: i64,
    id: &str,
    headers: &[String],
    rows: &[Vec<String>],
) -> Result<Value> {
    let grant = require_grant(db, workspace_id).await?;
    let Some(row) = find(db, workspace_id, id).await? else {
        return Ok(invalid("Custom audience not found."));
    };
    match prepare_payload(headers, rows) {
        Ok(payload) => upload(db, &row, &grant, &payload).await,
        Err(message) => Ok(invalid(message)),
    }
}

/// `deleteAudience`: refused while a campaign targets it; Meta first, the
/// row only once Meta deleted it (or no longer has it).
pub async fn delete(db: &Db, workspace_id: i64, id: &str) -> Result<()> {
    let grant = require_grant(db, workspace_id).await?;
    let row = find(db, workspace_id, id)
        .await?
        .ok_or_else(|| audience_error("Custom audience not found."))?;
    let audience_id = row.id;
    let used_by = tx::read(db, move |conn| {
        Ok(
            chatbotx_db::repo::facebook_marketing_message::count_by_audience_id(
                conn,
                audience_id,
                workspace_id,
            )?,
        )
    })
    .await?;
    if used_by > 0 {
        let noun = if used_by == 1 {
            "marketing message"
        } else {
            "marketing messages"
        };
        return Err(audience_error(format!(
            "This custom audience is used by {} {noun} and cannot be deleted.",
            fmt_int(used_by)
        )));
    }
    graph::delete_custom_audience(&grant.access_token, &grant.version, &row.custom_audience_id)
        .await?;
    tx::write(db, move |tx| {
        Ok(repo::remove(tx, audience_id, workspace_id)?)
    })
    .await
}

fn count(meta: Option<&Value>, key: &str) -> Value {
    meta.map(|m| &m[key])
        .filter(|v| v.is_number())
        .cloned()
        .unwrap_or(Value::Null)
}

fn status_of(value: &Value) -> Value {
    match value["code"].as_i64() {
        Some(code) => {
            json!({ "code": code, "description": value["description"].as_str().unwrap_or_default() })
        }
        None => Value::Null,
    }
}

/// `getLiveStatus`: Meta counts per ad account plus the subscription-token
/// ready check of the pending audiences, keyed by `customAudienceId`.
/// ponytail: the Graph reads run one after another (the original fans out with
/// a concurrency of 5); a workspace has a handful of audiences.
pub async fn live_status(db: &Db, workspace_id: i64) -> Result<Value> {
    let grant = require_grant(db, workspace_id).await?;
    let rows = tx::read(db, move |conn| {
        Ok(repo::list_by_workspace_id(
            conn,
            workspace_id,
            &ListFilter::default(),
        )?)
    })
    .await?;
    let mut meta: HashMap<String, Value> = HashMap::new();
    let mut failed: HashSet<String> = HashSet::new();
    let mut accounts: Vec<&str> = rows.iter().map(|r| r.ad_account_id.as_str()).collect();
    accounts.sort_unstable();
    accounts.dedup();
    for act in accounts {
        match graph::mm_audiences(&grant.access_token, &grant.version, act).await {
            Ok(list) => {
                for audience in list {
                    if let Some(id) = audience["id"].as_str() {
                        meta.insert(id.to_owned(), audience.clone());
                    }
                }
            }
            Err(error) => {
                tracing::error!(message = %error.message, workspace_id, ad_account_id = act, "Failed to load marketing messages audiences");
                failed.insert(act.to_owned());
            }
        }
    }
    let mut result = Map::new();
    for row in &rows {
        let m = meta.get(&row.custom_audience_id);
        result.insert(
            row.custom_audience_id.clone(),
            json!({
                "lower": count(m, "approximate_count_lower_bound"),
                "upper": count(m, "approximate_count_upper_bound"),
                "deliveryStatus": m.map_or(Value::Null, |m| status_of(&m["delivery_status"])),
                "ready": row.ready_at.is_some(),
                "missingOnMeta": m.is_none() && !failed.contains(&row.ad_account_id),
                "pageDisconnected": false,
            }),
        );
    }
    let mut pages: Vec<&str> = rows
        .iter()
        .filter(|r| r.ready_at.is_none() && result[&r.custom_audience_id]["missingOnMeta"] == false)
        .map(|r| r.page_id.as_str())
        .collect();
    pages.sort_unstable();
    pages.dedup();
    let mut newly_ready = Vec::new();
    for page_id in pages {
        let pending: Vec<_> = rows
            .iter()
            .filter(|r| {
                r.page_id == page_id
                    && r.ready_at.is_none()
                    && result[&r.custom_audience_id]["missingOnMeta"] == false
            })
            .collect();
        let page = match page_token(db, workspace_id, page_id).await {
            Ok(page) => page,
            Err(error) => {
                tracing::error!(%error, workspace_id, page_id, "Failed to resolve Page token for marketing messages audiences");
                continue;
            }
        };
        let Some((token, version)) = page else {
            for row in pending {
                result[&row.custom_audience_id]["pageDisconnected"] = json!(true);
            }
            continue;
        };
        for row in pending {
            match graph::token_audience_ids(&token, &version, page_id, &row.custom_audience_id)
                .await
            {
                Ok(ids) if ids.contains(&row.custom_audience_id) => {
                    result[&row.custom_audience_id]["ready"] = json!(true);
                    newly_ready.push(row.id);
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::error!(message = %error.message, workspace_id, page_id, "Marketing messages audience token check failed");
                }
            }
        }
    }
    if !newly_ready.is_empty() {
        let now = now_ms();
        if let Err(error) = tx::write(db, move |tx| {
            Ok(repo::mark_ready(tx, &newly_ready, workspace_id, now)?)
        })
        .await
        {
            tracing::error!(%error, workspace_id, "Failed to persist marketing messages audience ready state");
        }
    }
    Ok(Value::Object(result))
}
