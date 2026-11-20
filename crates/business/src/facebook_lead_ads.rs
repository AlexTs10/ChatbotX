//! `facebook-lead-ads-automation/service.ts` (automation + lead claim rows)
//! and the builder lib the public routes use (`lib/pages.ts`,
//! `lib/create-automation.ts`): page eligibility, lead forms, the page
//! `leadgen` subscription.

use chatbotx_db::Db;
use chatbotx_db::repo::utils::{order_by_sql, parse_order_by};
use chatbotx_db::repo::{
    PaginationInput, SortItem, get_pagination_with_defaults, is_unique_violation_error, iso,
    like_contains, new_id, now_ms,
};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter};
use serde_json::{Value, json};

use crate::connection::credentials::{HttpUpstream, Upstream, UpstreamRequest};
use crate::email_marketing::open_auth;
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `ALL_FORMS_ID`.
pub const ALL_FORMS_ID: &str = "*";
const GRAPH: &str = "https://graph.facebook.com";
/// `integration-messenger` `DEFAULT_API_VERSION`.
const MESSENGER_VERSION: &str = "v23.0";
const COLUMNS: &[&str] = &[
    "id",
    "createdAt",
    "updatedAt",
    "name",
    "workspaceId",
    "pageId",
    "pageName",
    "formId",
    "formName",
    "fieldMapping",
    "leadsHandledCount",
];

/// One `FacebookLeadAdsAutomation` row.
#[derive(Debug, Clone, PartialEq)]
pub struct Automation {
    pub id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub name: String,
    pub workspace_id: i64,
    pub page_id: String,
    pub page_name: Option<String>,
    pub form_id: String,
    pub form_name: Option<String>,
    pub field_mapping: Value,
    pub leads_handled_count: i64,
}

fn select() -> String {
    format!(
        "SELECT {} FROM \"FacebookLeadAdsAutomation\"",
        COLUMNS
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn from_row(r: &Row<'_>) -> chatbotx_db::rusqlite::Result<Automation> {
    let mapping: String = r.get(9)?;
    Ok(Automation {
        id: r.get(0)?,
        created_at: r.get(1)?,
        updated_at: r.get(2)?,
        name: r.get(3)?,
        workspace_id: r.get(4)?,
        page_id: r.get(5)?,
        page_name: r.get(6)?,
        form_id: r.get(7)?,
        form_name: r.get(8)?,
        field_mapping: serde_json::from_str(&mapping).unwrap_or_else(|_| json!([])),
        leads_handled_count: r.get(10)?,
    })
}

/// `facebookLeadAdPublicResource` (no `workspaceId`).
pub fn resource(row: &Automation) -> Value {
    json!({
        "id": row.id.to_string(),
        "createdAt": iso(row.created_at),
        "updatedAt": iso(row.updated_at),
        "name": row.name,
        "pageId": row.page_id,
        "pageName": row.page_name,
        "formId": row.form_id,
        "formName": row.form_name,
        "fieldMapping": row.field_mapping,
        "leadsHandledCount": row.leads_handled_count,
    })
}

fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<Automation>> {
    Ok(conn
        .query_row(
            &format!("{} WHERE \"id\" = ?1 AND \"workspaceId\" = ?2", select()),
            params![id, workspace_id],
            from_row,
        )
        .optional()?)
}

/// `list`: newest-first by default; `keyword` matches the name.
pub async fn list(
    db: &Db,
    workspace_id: i64,
    page: i64,
    per_page: i64,
    keyword: Option<String>,
    sort: Option<Vec<SortItem>>,
) -> Result<(Vec<Automation>, i64)> {
    tx::read(db, move |conn| {
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: Some(page),
            per_page: Some(per_page),
        });
        let keyword = keyword
            .as_deref()
            .filter(|k| !k.is_empty())
            .map(like_contains);
        let filter = "\"workspaceId\" = ?1 AND (?2 IS NULL OR ilike(\"name\", ?2))";
        let order = parse_order_by(COLUMNS, sort.as_deref(), None);
        let sql = format!(
            "{} WHERE {filter} {} LIMIT ?3 OFFSET ?4",
            select(),
            order_by_sql(&order, None)
        );
        let args = vec![
            Sql::Integer(workspace_id),
            keyword.clone().map_or(Sql::Null, Sql::Text),
            Sql::Integer(pagination.limit),
            Sql::Integer(pagination.offset),
        ];
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params_from_iter(args), from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let total: i64 = conn.query_row(
            &format!("SELECT count(*) FROM \"FacebookLeadAdsAutomation\" WHERE {filter}"),
            params![workspace_id, keyword],
            |r| r.get(0),
        )?;
        let per_page = per_page.max(1);
        Ok((rows, (total + per_page - 1) / per_page))
    })
    .await
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Facebook Lead Ads automation not found")
}

/// `findById` (404 when missing).
pub async fn get(db: &Db, workspace_id: i64, id: i64) -> Result<Automation> {
    tx::read(db, move |conn| {
        find_in(conn, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// `createFacebookLeadAdAutomationRequest` (parsed).
#[derive(Debug, Clone)]
pub struct CreateInput {
    pub name: String,
    pub page_id: String,
    pub page_name: Option<String>,
    pub form_id: String,
    pub form_name: Option<String>,
    pub field_mapping: Value,
}

/// `createLeadAdAutomation`: subscribe the page to `leadgen` first (a failure
/// blocks the create), then insert; a duplicate (page, form) is a 422.
pub async fn create(db: &Db, workspace_id: i64, input: CreateInput) -> Result<Automation> {
    let page_id = input.page_id.clone();
    let auth = page_auth(db, workspace_id, page_id.clone()).await?;
    let subscribed = match &auth {
        Some(auth) => {
            crate::connection::oauth::subscribe_webhook_fields(&HttpUpstream, auth, &["leadgen"])
                .await
                .is_ok()
        }
        None => false,
    };
    if !subscribed {
        tracing::error!(
            workspace_id,
            page_id,
            "createLeadAdAutomation: failed to subscribe page to leadgen"
        );
        return Err(BusinessError::exception(
            "Failed to subscribe the page to lead webhooks. Try again.",
            "systemError",
            400,
        ));
    }
    tx::write(db, move |tx| {
        let id = new_id();
        let now = now_ms();
        let inserted = tx.execute(
            "INSERT INTO \"FacebookLeadAdsAutomation\" (\"id\", \"createdAt\", \"updatedAt\", \"name\", \
             \"workspaceId\", \"pageId\", \"pageName\", \"formId\", \"formName\", \"fieldMapping\") \
             VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                now,
                input.name,
                workspace_id,
                input.page_id,
                input.page_name,
                input.form_id,
                input.form_name,
                input.field_mapping.to_string()
            ],
        );
        match inserted {
            Err(error) => {
                let error = chatbotx_db::repo::Error::from(error);
                if is_unique_violation_error(&error, None) {
                    Err(BusinessError::validation(
                        "formId",
                        "An automation for this page and form already exists.",
                    ))
                } else {
                    Err(BusinessError::Repo(error))
                }
            }
            Ok(_) => find_in(tx, workspace_id, id)?.ok_or_else(not_found),
        }
    })
    .await
}

/// `update` (`name` / `fieldMapping`; 404 when missing).
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    name: Option<String>,
    field_mapping: Option<Value>,
) -> Result<Automation> {
    tx::write(db, move |tx| {
        let changed = tx.execute(
            "UPDATE \"FacebookLeadAdsAutomation\" SET \"name\" = COALESCE(?1, \"name\"), \
             \"fieldMapping\" = COALESCE(?2, \"fieldMapping\"), \"updatedAt\" = ?3 \
             WHERE \"id\" = ?4 AND \"workspaceId\" = ?5",
            params![
                name,
                field_mapping.map(|m| m.to_string()),
                now_ms(),
                id,
                workspace_id
            ],
        )?;
        if changed == 0 {
            return Err(not_found());
        }
        find_in(tx, workspace_id, id)?.ok_or_else(not_found)
    })
    .await
}

/// `deleteMany` (one id; a missing row is fine, as the original).
pub async fn delete(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    tx::write(db, move |tx| {
        tx.execute(
            "DELETE FROM \"FacebookLeadAdsAutomation\" WHERE \"workspaceId\" = ?1 AND \"id\" = ?2",
            params![workspace_id, id],
        )?;
        Ok(())
    })
    .await
}

/// `findMatching`: the form-specific automation wins over the `*` one.
pub async fn find_matching(
    db: &Db,
    workspace_id: i64,
    page_id: String,
    form_id: String,
) -> Result<Option<Automation>> {
    tx::read(db, move |conn| {
        let mut stmt = conn.prepare(&format!(
            "{} WHERE \"workspaceId\" = ?1 AND \"pageId\" = ?2 AND \"formId\" IN (?3, ?4)",
            select()
        ))?;
        let rows = stmt
            .query_map(
                params![workspace_id, page_id, form_id, ALL_FORMS_ID],
                from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows
            .iter()
            .find(|r| r.form_id == form_id)
            .or_else(|| rows.iter().find(|r| r.form_id == ALL_FORMS_ID))
            .cloned())
    })
    .await
}

/// `facebookLeadAdsLeadService.claim`: the new lead row id, `None` when the
/// (automation, leadgen) pair was already claimed.
pub async fn claim_lead(
    db: &Db,
    automation_id: i64,
    leadgen_id: String,
    form_id: String,
    form_name: Option<String>,
    adgroup_id: Option<String>,
) -> Result<Option<i64>> {
    tx::write(db, move |tx| {
        let id = new_id();
        let inserted = tx.execute(
            "INSERT INTO \"FacebookLeadAdsLead\" (\"id\", \"createdAt\", \"updatedAt\", \"automationId\", \
             \"leadgenId\", \"formId\", \"formName\", \"adgroupId\") VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT DO NOTHING",
            params![id, now_ms(), automation_id, leadgen_id, form_id, form_name, adgroup_id],
        )?;
        Ok((inserted > 0).then_some(id))
    })
    .await
}

/// The page's Messenger auth (`messengerIntegrationService.findByPageId`).
async fn page_auth(db: &Db, workspace_id: i64, page_id: String) -> Result<Option<Value>> {
    let raw: Option<String> = tx::read(db, move |conn| {
        Ok(conn
            .query_row(
                "SELECT \"auth\" FROM \"IntegrationMessenger\" WHERE \"workspaceId\" = ?1 AND \"pageId\" = ?2 LIMIT 1",
                params![workspace_id, page_id],
                |r| r.get(0),
            )
            .optional()?)
    })
    .await?;
    Ok(raw.and_then(|raw| open_auth(&raw)))
}

fn version_of(auth: &Value) -> &str {
    auth["metadata"]["version"]
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or(MESSENGER_VERSION)
}

async fn graph_get(url: String, bearer: Option<String>) -> Result<Value> {
    let headers = bearer
        .map(|t| vec![("authorization".to_owned(), format!("Bearer {t}"))])
        .unwrap_or_default();
    let reply = HttpUpstream
        .send(UpstreamRequest {
            method: "GET",
            url,
            headers,
            body: None,
        })
        .await;
    match reply {
        Ok((status, body)) if (200..300).contains(&status) => {
            Ok(serde_json::from_str(&body).unwrap_or(Value::Null))
        }
        Ok((status, body)) => {
            let body: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let message = chatbotx_core::utils::graph_error::format_graph_error(body.get("error"))
                .unwrap_or_else(|| "Meta Graph request failed".to_owned());
            // A `MessengerAPIException` (SdkException): `toKnownOrpcError`
            // answers `BAD_REQUEST` with the Graph status.
            Err(BusinessError::exception(
                message,
                "BAD_REQUEST",
                status.clamp(400, 599),
            ))
        }
        Err(error) => Err(BusinessError::exception(error, "businessError", 502)),
    }
}

fn graph_url(path: &str, query: &[(&str, &str)]) -> String {
    let mut url = format!("{GRAPH}/{path}");
    if !query.is_empty() {
        url.push('?');
        url.push_str(
            &url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(query)
                .finish(),
        );
    }
    url
}

/// `isTokenLeadEligible`: the page token is valid and carries `leads_retrieval`
/// (`/debug_token` with the minting app's own `clientId|clientSecret`).
async fn token_lead_eligible(auth: &Value) -> Result<bool> {
    let (Some(client_id), Some(secret)) = (
        auth["clientId"].as_str().filter(|s| !s.is_empty()),
        auth["clientSecret"].as_str().filter(|s| !s.is_empty()),
    ) else {
        return Ok(false);
    };
    let token = auth["tokens"]["accessToken"].as_str().unwrap_or_default();
    let body = graph_get(
        graph_url(
            &format!("{}/debug_token", version_of(auth)),
            &[("input_token", token)],
        ),
        Some(format!("{client_id}|{secret}")),
    )
    .await?;
    let data = &body["data"];
    let scopes = data["scopes"].as_array().cloned().unwrap_or_default();
    Ok(data["is_valid"] != json!(false) && scopes.iter().any(|s| s == "leads_retrieval"))
}

/// `listEligibleLeadAdsPages`: a page whose check fails is dropped.
pub async fn list_pages(db: &Db, workspace_id: i64) -> Result<Vec<Value>> {
    let rows: Vec<(String, Option<String>, String)> = tx::read(db, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT \"pageId\", \"name\", \"auth\" FROM \"IntegrationMessenger\" WHERE \"workspaceId\" = ?1 ORDER BY \"createdAt\", \"id\"",
        )?;
        Ok(stmt
            .query_map([workspace_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    })
    .await?;
    let mut pages = Vec::new();
    for (page_id, name, auth) in rows {
        let auth = open_auth(&auth).unwrap_or(Value::Null);
        if let Ok(eligible) = token_lead_eligible(&auth).await {
            pages.push(json!({ "pageId": page_id, "pageName": name, "eligible": eligible }));
        }
    }
    Ok(pages)
}

/// `enableLeadgenForWorkspacePages` (the `facebookLeadAds` OAuth return):
/// every lead-eligible page subscribes to `leadgen`; a page that fails is
/// skipped (`Promise.allSettled`).
pub async fn enable_leadgen_for_workspace_pages(db: &Db, workspace_id: i64) -> Result<()> {
    let rows: Vec<String> = tx::read(db, move |conn| {
        let mut stmt = conn
            .prepare("SELECT \"auth\" FROM \"IntegrationMessenger\" WHERE \"workspaceId\" = ?1")?;
        Ok(stmt
            .query_map([workspace_id], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    })
    .await?;
    for raw in rows {
        let Some(auth) = open_auth(&raw) else {
            continue;
        };
        if !token_lead_eligible(&auth).await.unwrap_or(false) {
            continue;
        }
        if let Err(error) =
            crate::connection::oauth::subscribe_webhook_fields(&HttpUpstream, &auth, &["leadgen"])
                .await
        {
            tracing::warn!(?error, workspace_id, "leadgen page subscription failed");
        }
    }
    Ok(())
}

/// `listPageLeadForms`: `[]` for a page with no Messenger integration.
pub async fn list_forms(db: &Db, workspace_id: i64, page_id: String) -> Result<Vec<Value>> {
    let Some(auth) = page_auth(db, workspace_id, page_id.clone()).await? else {
        return Ok(Vec::new());
    };
    let token = auth["tokens"]["accessToken"].as_str().unwrap_or_default();
    let body = graph_get(
        graph_url(
            &format!("{}/{page_id}/leadgen_forms", version_of(&auth)),
            &[
                ("fields", "id,name,status,questions"),
                ("limit", "100"),
                ("access_token", token),
            ],
        ),
        None,
    )
    .await?;
    // `listFacebookLeadAdsFormsPublicResponse` strips unknown keys.
    let forms = body["data"].as_array().cloned().unwrap_or_default();
    Ok(forms.into_iter().map(public_form).collect())
}

/// One form as the public schema: `{ id, name, status, questions? }`, each
/// question `{ key, label, type, id }`.
pub fn public_form(form: Value) -> Value {
    let pick = |value: &Value, keys: &[&str]| {
        let mut out = serde_json::Map::new();
        for k in keys {
            if let Some(v) = value.get(*k) {
                out.insert((*k).to_owned(), v.clone());
            }
        }
        out
    };
    let mut out = pick(&form, &["id", "name", "status"]);
    if let Some(Value::Array(questions)) = form.get("questions") {
        let questions = questions
            .iter()
            .map(|q| Value::Object(pick(q, &["key", "label", "type", "id"])))
            .collect();
        out.insert("questions".into(), Value::Array(questions));
    }
    Value::Object(out)
}

/// `getLead`: the lead's `field_data`.
pub async fn get_lead(auth: &Value, leadgen_id: &str) -> Result<Vec<Value>> {
    let token = auth["tokens"]["accessToken"].as_str().unwrap_or_default();
    let body = graph_get(
        graph_url(
            &format!("{}/{leadgen_id}", version_of(auth)),
            &[
                ("fields", "id,created_time,field_data"),
                ("access_token", token),
            ],
        ),
        None,
    )
    .await?;
    Ok(body["field_data"].as_array().cloned().unwrap_or_default())
}

/// `getLeadgenFormName`: at most five pages of `id,name`.
pub async fn get_form_name(auth: &Value, page_id: &str, form_id: &str) -> Result<Option<String>> {
    let token = auth["tokens"]["accessToken"].as_str().unwrap_or_default();
    let mut after: Option<String> = None;
    for _ in 0..5 {
        let mut query = vec![
            ("fields", "id,name"),
            ("limit", "100"),
            ("access_token", token),
        ];
        if let Some(after) = &after {
            query.push(("after", after.as_str()));
        }
        let body = graph_get(
            graph_url(
                &format!("{}/{page_id}/leadgen_forms", version_of(auth)),
                &query,
            ),
            None,
        )
        .await?;
        if let Some(form) = body["data"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|f| f["id"] == json!(form_id))
        {
            return Ok(form["name"].as_str().map(str::to_owned));
        }
        after = body["paging"]["next"]
            .as_str()
            .and_then(|_| body["paging"]["cursors"]["after"].as_str())
            .map(str::to_owned);
        if after.is_none() {
            return Ok(None);
        }
    }
    Ok(None)
}

/// `facebookLeadAdsLeadService.release`.
pub async fn release_lead(db: &Db, id: i64) -> Result<()> {
    tx::write(db, move |tx| {
        tx.execute(
            "DELETE FROM \"FacebookLeadAdsLead\" WHERE \"id\" = ?1",
            [id],
        )?;
        Ok(())
    })
    .await
}

/// `facebookLeadAdsLeadService.setContactId` / `setFormName`.
pub async fn set_lead_column(db: &Db, id: i64, column: &'static str, value: Sql) -> Result<()> {
    tx::write(db, move |tx| {
        tx.execute(
            &format!("UPDATE \"FacebookLeadAdsLead\" SET \"{column}\" = ?1, \"updatedAt\" = ?2 WHERE \"id\" = ?3"),
            params![value, now_ms(), id],
        )?;
        Ok(())
    })
    .await
}

/// `incrementLeadsHandled`.
pub async fn increment_leads_handled(db: &Db, id: i64) -> Result<()> {
    tx::write(db, move |tx| {
        tx.execute(
            "UPDATE \"FacebookLeadAdsAutomation\" SET \"leadsHandledCount\" = \"leadsHandledCount\" + 1, \"updatedAt\" = ?1 WHERE \"id\" = ?2",
            params![now_ms(), id],
        )?;
        Ok(())
    })
    .await
}

/// `FB_LEAD_STANDARD_FIELD_TARGET` (an all-forms automation auto-maps these).
pub fn standard_field_target(name: &str) -> Option<&'static str> {
    Some(match name {
        "email" => "email",
        "phone_number" => "phone",
        "full_name" => "full_name",
        "first_name" => "first_name",
        "last_name" => "last_name",
        "gender" => "gender",
        _ => return None,
    })
}

/// `richSystemFieldToContactData` (`full_name` splits on whitespace).
pub fn rich_system_columns(field: &str, value: &str) -> Vec<(&'static str, Option<String>)> {
    match field {
        "phone" | "phone_number" => vec![("phoneNumber", Some(value.to_owned()))],
        "email" => vec![("email", Some(value.to_owned()))],
        "first_name" => vec![("firstName", Some(value.to_owned()))],
        "last_name" => vec![("lastName", Some(value.to_owned()))],
        "full_name" => {
            let mut parts = value.split_whitespace();
            let first = parts.next().map(str::to_owned);
            let rest: Vec<&str> = parts.collect();
            vec![
                ("firstName", first),
                ("lastName", (!rest.is_empty()).then(|| rest.join(" "))),
            ]
        }
        _ => Vec::new(),
    }
}

/// `contactService.setRichSystemFieldByKey` / the `gender` update.
pub async fn set_contact_columns(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    columns: Vec<(&'static str, Option<String>)>,
) -> Result<()> {
    tx::write(db, move |tx| {
        crate::contact::service::update_text_columns(tx, workspace_id, contact_id, &columns)?;
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_rich_fields() {
        assert_eq!(
            rich_system_columns("full_name", "  Ada  Lovelace King "),
            vec![
                ("firstName", Some("Ada".to_owned())),
                ("lastName", Some("Lovelace King".to_owned()))
            ]
        );
        assert_eq!(
            rich_system_columns("full_name", "Ada"),
            vec![("firstName", Some("Ada".to_owned())), ("lastName", None)]
        );
        assert_eq!(rich_system_columns("phone_number", "1")[0].0, "phoneNumber");
        assert_eq!(standard_field_target("phone_number"), Some("phone"));
        assert_eq!(standard_field_target("city"), None);
    }
}
