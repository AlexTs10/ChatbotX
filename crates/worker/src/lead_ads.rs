//! `handlers/lead-ads/index.ts` `processLeadgen`: one Facebook `leadgen`
//! webhook → match an automation, claim the lead, read its answers, resolve
//! the contact by PSID, write the mapped fields, bump the counter.

use chatbotx_business::contact_custom_field::{SetValueByKeyInput, set_value_by_key};
use chatbotx_business::facebook_lead_ads as service;
use chatbotx_business::questionnaire::is_rich_system_contact_field;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_jobs::{JobCtx, JobError};
use serde_json::{Value, json};

use crate::channel::IncomingContact;
use crate::{Deps, str_of};

pub const PROCESS_LEADGEN: &str = "processLeadgen";

fn find_value(field_data: &[Value], name: &str) -> Option<String> {
    field_data
        .iter()
        .find(|f| f["name"] == json!(name))?
        .pointer("/values/0")?
        .as_str()
        .map(str::to_owned)
}

/// `extractPsidFromInboxUrl`: the last path segment, digits only.
fn psid_of(inbox_url: &str) -> Option<String> {
    let url = url::Url::parse(inbox_url).ok()?;
    let last = url.path_segments()?.rfind(|s| !s.is_empty())?;
    (!last.is_empty() && last.bytes().all(|b| b.is_ascii_digit())).then(|| last.to_owned())
}

/// `applyField`: a failure is only logged.
async fn apply_field(deps: &Deps, workspace_id: i64, contact_id: i64, target: &str, value: String) {
    let result = if target == "gender" {
        let normalized = value.trim().to_lowercase();
        if !matches!(normalized.as_str(), "male" | "female" | "unknown") {
            return;
        }
        service::set_contact_columns(
            &deps.db,
            workspace_id,
            contact_id,
            vec![("gender", Some(normalized))],
        )
        .await
    } else if is_rich_system_contact_field(target) {
        let columns = service::rich_system_columns(target, &value);
        service::set_contact_columns(&deps.db, workspace_id, contact_id, columns).await
    } else {
        set_value_by_key(
            &deps.db,
            SetValueByKeyInput {
                workspace_id,
                contact_id,
                keyword: target.to_owned(),
                value,
                ..SetValueByKeyInput::default()
            },
        )
        .await
        .map(drop)
    };
    if let Err(error) = result {
        tracing::warn!(%error, target, "processLeadgen: failed to set contact field");
    }
}

fn fail(error: impl std::fmt::Display) -> JobError {
    JobError::retryable(error.to_string())
}

/// `processLeadgen`.
pub async fn process_leadgen(deps: &Deps, job: &JobCtx) -> Result<(), JobError> {
    let data = &job.data["data"];
    let kind = str_of(data, "integrationType").unwrap_or("");
    let page_id = str_of(data, "integrationIdentifier")
        .unwrap_or("")
        .to_owned();
    let leadgen_id = str_of(data, "leadgenId").unwrap_or("").to_owned();
    let form_id = str_of(data, "formId").unwrap_or("").to_owned();
    let adgroup_id = str_of(data, "adgroupId").map(str::to_owned);
    let (_, ctx, inbox) = deps
        .channels
        .resolve_by_identifier(&deps.db, deps.realtime.as_ref(), kind, &page_id)
        .await
        .map_err(|e| JobError::unrecoverable(e.to_string()))?
        .ok_or_else(|| {
            JobError::unrecoverable(format!("Integration not found: {kind} {page_id}"))
        })?;
    let ws = inbox.workspace_id;
    let Some(automation) = service::find_matching(&deps.db, ws, page_id.clone(), form_id.clone())
        .await
        .map_err(fail)?
    else {
        return Ok(());
    };
    // Claim before any external work so re-deliveries no-op.
    let Some(claim) = service::claim_lead(
        &deps.db,
        automation.id,
        leadgen_id.clone(),
        form_id.clone(),
        automation.form_name.clone(),
        adgroup_id,
    )
    .await
    .map_err(fail)?
    else {
        return Ok(());
    };
    let mut psid: Option<String> = None;
    let mut contact_id: Option<i64> = None;
    let outcome: Result<(), JobError> = async {
        let field_data = service::get_lead(&ctx.auth, &leadgen_id)
            .await
            .map_err(fail)?;
        let Some(lead_psid) = find_value(&field_data, "inbox_url").and_then(|u| psid_of(&u)) else {
            tracing::info!(
                leadgen_id,
                "processLeadgen: lead has no inbox_url/PSID, skipping"
            );
            return Ok(());
        };
        psid = Some(lead_psid.clone());
        let incoming = IncomingContact {
            source_id: lead_psid,
            first_name: find_value(&field_data, "full_name"),
            ..IncomingContact::default()
        };
        let (contact_inbox, _, _) =
            crate::inbound::detect_contact_and_conversation(deps, &inbox, &incoming, "ads").await?;
        let contact = contact_inbox.contact_id;
        service::set_lead_column(&deps.db, claim, "contactId", Sql::Integer(contact))
            .await
            .map_err(fail)?;
        contact_id = Some(contact);
        if automation.form_name.is_none() {
            // Best-effort: a failed lookup leaves the name empty.
            match service::get_form_name(&ctx.auth, &page_id, &form_id).await {
                Ok(Some(name)) => {
                    service::set_lead_column(&deps.db, claim, "formName", Sql::Text(name))
                        .await
                        .map_err(fail)?;
                }
                Ok(None) => tracing::info!(
                    form_id,
                    "processLeadgen: form not found among the page's forms"
                ),
                Err(error) => {
                    tracing::warn!(%error, form_id, "processLeadgen: failed to resolve form name")
                }
            }
        }
        // An explicit mapping, else auto-map the standard fields.
        let mapping: Vec<(String, Option<String>)> = match automation.field_mapping.as_array() {
            Some(items) if !items.is_empty() => items
                .iter()
                .map(|m| {
                    (
                        m["key"].as_str().unwrap_or_default().to_owned(),
                        m["target"].as_str().map(str::to_owned),
                    )
                })
                .collect(),
            _ => field_data
                .iter()
                .filter_map(|f| {
                    let name = f["name"].as_str()?;
                    let target = service::standard_field_target(name)?;
                    Some((name.to_owned(), Some(target.to_owned())))
                })
                .collect(),
        };
        for (key, target) in mapping {
            let Some(target) = target.filter(|t| !t.is_empty()) else {
                continue;
            };
            if key == "inbox_url" {
                continue;
            }
            if let Some(value) = find_value(&field_data, &key) {
                apply_field(deps, ws, contact, &target, value).await;
            }
        }
        service::increment_leads_handled(&deps.db, automation.id)
            .await
            .map_err(fail)
    }
    .await;
    let Err(error) = outcome else {
        return Ok(());
    };
    // Release the claim so the retry can pick the lead back up.
    if let Err(release_error) = service::release_lead(&deps.db, claim).await {
        tracing::error!(%release_error, leadgen_id, "processLeadgen: failed to release lead claim");
    }
    if job.is_final_attempt() {
        let input = crate::events::ProviderError {
            provider: "facebook-ads".to_owned(),
            workspace_id: ws.to_string(),
            contact_id: contact_id.map(|c| c.to_string()),
            source_id: psid,
            error: json!({ "message": error.to_string() }),
            ..Default::default()
        };
        crate::events::log_provider_errors(deps.events.as_ref(), &[input]).await;
    }
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_psid_from_inbox_url() {
        assert_eq!(
            psid_of("https://business.facebook.com/latest/123456?nav_ref=x").as_deref(),
            Some("123456")
        );
        assert_eq!(
            psid_of("https://business.facebook.com/latest/123/").as_deref(),
            Some("123")
        );
        assert_eq!(psid_of("https://business.facebook.com/latest/inbox"), None);
        assert_eq!(psid_of("not a url"), None);
    }
}
