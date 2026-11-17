//! `integrations/active-campaign/src/{client,integration}.ts` and
//! `active-campaign-handler.ts` (`syncActiveCampaignContact`).

use chatbotx_db::Db;
use serde_json::{Value, json};

use super::{
    Http, Outcome, ProviderError, Request, auth_str, call, contact_field_map, field, load_auth,
    run_exclusive, sha256, step_str, step_strings,
};

pub const LOCK_TIMEOUT_SECONDS: u64 = 30;
const SUBSCRIBER_SERIES_ERROR_MESSAGE: &str = "Could not create SubscriberSeries";

/// `normalizeActiveCampaignApiUrl`.
pub fn normalize_api_url(api_url: &str) -> String {
    let url = api_url.trim().trim_end_matches('/');
    let lower = url.to_ascii_lowercase();
    for suffix in ["/api/3/", "/api/3"] {
        if lower.ends_with(suffix) {
            return url[..url.len() - suffix.len()].to_owned();
        }
    }
    url.to_owned()
}

fn error_message(payload: Option<&Value>, status: u16) -> String {
    let fallback = || format!("ActiveCampaign API returned {status}");
    let Some(payload) = payload.filter(|p| p.is_object()) else {
        return fallback();
    };
    if let Some(message) = payload
        .get("message")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
    {
        return message.to_owned();
    }
    let first = payload
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|e| e.first());
    first
        .and_then(|f| f.get("title").and_then(Value::as_str))
        .or_else(|| first.and_then(|f| f.get("detail").and_then(Value::as_str)))
        .map_or_else(fallback, str::to_owned)
}

/// `activeCampaignRequest(auth, "api/3/<path>")`.
pub async fn request(
    http: &dyn Http,
    auth: &Value,
    method: &'static str,
    path: &str,
    body: Option<Value>,
) -> Outcome<Option<Value>> {
    let base = normalize_api_url(auth_str(auth, "apiUrl"));
    let mut req = Request::new(method, format!("{base}/api/3/{path}"))
        .header("Api-Token", auth_str(auth, "apiKey"));
    if let Some(body) = body {
        req = req.json(body);
    }
    let reply = call(http, req).await?;
    let payload = reply.payload();
    if !reply.ok() {
        return Err(ProviderError::http(
            error_message(payload.as_ref(), reply.status),
            reply.status,
            &reply,
        ));
    }
    Ok(payload)
}

/// `z.union([string, number]).transform(String)`.
fn id_of(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Value::Number(n) if n.as_u64().is_some() => Some(n.to_string()),
        _ => None,
    }
}

async fn list(http: &dyn Http, auth: &Value, path: &str) -> Outcome<Vec<Value>> {
    let payload = request(http, auth, "GET", path, None).await?;
    let items = match payload.as_ref().map(|p| p.get(path)) {
        Some(None | Some(Value::Null)) => return Ok(Vec::new()),
        Some(Some(Value::Array(items))) => items.clone(),
        _ => return Err(super::invalid_response("ActiveCampaign")),
    };
    items
        .into_iter()
        .map(|mut item| {
            let id = item.get("id").and_then(id_of);
            let id = id.ok_or_else(|| super::invalid_response("ActiveCampaign"))?;
            item["id"] = json!(id);
            Ok(item)
        })
        .collect()
}

/// `listLists`.
pub async fn list_lists(http: &dyn Http, auth: &Value) -> Outcome<Vec<Value>> {
    list(http, auth, "lists").await
}

/// `listAutomations`.
pub async fn list_automations(http: &dyn Http, auth: &Value) -> Outcome<Vec<Value>> {
    list(http, auth, "automations").await
}

/// `listTags`.
pub async fn list_tags(http: &dyn Http, auth: &Value) -> Outcome<Vec<Value>> {
    list(http, auth, "tags").await
}

/// `listCustomFields`: `{ id, label: label || title || perstag || id }`.
pub async fn list_custom_fields(http: &dyn Http, auth: &Value) -> Outcome<Vec<Value>> {
    Ok(list(http, auth, "fields")
        .await?
        .into_iter()
        .map(|f| {
            let text = |key: &str| {
                f.get(key)
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            };
            let label = text("label")
                .or_else(|| text("title"))
                .or_else(|| text("perstag"))
                .unwrap_or_else(|| f["id"].as_str().unwrap_or_default().to_owned());
            json!({ "id": f["id"], "label": label })
        })
        .collect())
}

/// `syncContact`: the contact id.
pub async fn sync_contact(
    http: &dyn Http,
    auth: &Value,
    email: &str,
    phone: Option<&str>,
    field_values: &[(String, String)],
) -> Outcome<String> {
    let mut contact = json!({ "email": email });
    if let Some(phone) = phone {
        contact["phone"] = json!(phone);
    }
    if !field_values.is_empty() {
        contact["fieldValues"] = field_values
            .iter()
            .map(|(field, value)| json!({ "field": field, "value": value }))
            .collect();
    }
    let payload = request(
        http,
        auth,
        "POST",
        "contact/sync",
        Some(json!({ "contact": contact })),
    )
    .await?;
    payload
        .as_ref()
        .and_then(|p| p.get("contact"))
        .and_then(|c| c.get("id"))
        .and_then(id_of)
        .ok_or_else(|| super::invalid_response("ActiveCampaign"))
}

/// `addContactToList` (status "1").
pub async fn add_contact_to_list(
    http: &dyn Http,
    auth: &Value,
    contact_id: &str,
    list_id: &str,
) -> Outcome<()> {
    let body = json!({ "contactList": { "contact": contact_id, "list": list_id, "status": "1" } });
    request(http, auth, "POST", "contactLists", Some(body)).await?;
    Ok(())
}

/// `addTagToContact`.
pub async fn add_tag_to_contact(
    http: &dyn Http,
    auth: &Value,
    contact_id: &str,
    tag_id: &str,
) -> Outcome<()> {
    let body = json!({ "contactTag": { "contact": contact_id, "tag": tag_id } });
    request(http, auth, "POST", "contactTags", Some(body)).await?;
    Ok(())
}

/// `addContactToAutomation`: skipped when the contact is already in it;
/// a 422 "Could not create SubscriberSeries" counts as done.
pub async fn add_contact_to_automation(
    http: &dyn Http,
    auth: &Value,
    contact_id: &str,
    automation_id: &str,
) -> Outcome<()> {
    let path = format!("contacts/{contact_id}/contactAutomations");
    let existing = request(http, auth, "GET", &path, None).await?;
    let exists = existing
        .as_ref()
        .and_then(|p| p.get("contactAutomations"))
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                ["automation", "seriesid"]
                    .iter()
                    .any(|key| item.get(*key).and_then(id_of).as_deref() == Some(automation_id))
            })
        });
    if exists {
        return Ok(());
    }
    let body =
        json!({ "contactAutomation": { "contact": contact_id, "automation": automation_id } });
    match request(http, auth, "POST", "contactAutomations", Some(body)).await {
        Err(err)
            if err.status == Some(422) && err.message.contains(SUBSCRIBER_SERIES_ERROR_MESSAGE) =>
        {
            Ok(())
        }
        other => other.map(|_| ()),
    }
}

/// `syncActiveCampaignContact`: `Err` = the step's error message.
pub async fn sync_contact_step(
    db: &Db,
    http: &dyn Http,
    workspace_id: i64,
    contact_id: i64,
    step: &Value,
) -> Outcome<()> {
    let auth = load_auth(db, "activeCampaign", workspace_id).await?;
    let fields = contact_field_map(db, workspace_id, contact_id).await?;
    let email = field(&fields, step_str(step, "emailField")).map(|e| e.to_lowercase());
    let email =
        email.ok_or_else(|| ProviderError::local("ActiveCampaign contact email is empty"))?;
    let phone = field(&fields, step_str(step, "phoneField"));
    let field_values: Vec<(String, String)> =
        super::mappings(step, "contactFieldId", "activeCampaignFieldId")
            .into_iter()
            .filter_map(|(from, to)| Some((to.to_owned(), field(&fields, Some(from))?)))
            .collect();
    let api_url = normalize_api_url(auth_str(&auth, "apiUrl"));
    let key = format!(
        "active-campaign:sync-contact:{}:{}",
        sha256(&api_url),
        sha256(&email)
    );
    run_exclusive(&key, LOCK_TIMEOUT_SECONDS, async || {
        if step_str(step, "operation") == Some("addContactToAutomation") {
            let automation = step_str(step, "automationId")
                .filter(|a| !a.is_empty())
                .ok_or_else(|| ProviderError::new("ActiveCampaign automation is not configured"))?;
            let id = sync_contact(http, &auth, &email, None, &[]).await?;
            return add_contact_to_automation(http, &auth, &id, automation).await;
        }
        let id = sync_contact(http, &auth, &email, phone.as_deref(), &field_values).await?;
        for list_id in step_strings(step, "listIds") {
            add_contact_to_list(http, &auth, &id, &list_id).await?;
        }
        for tag_id in step_strings(step, "tagIds") {
            add_tag_to_contact(http, &auth, &id, &tag_id).await?;
        }
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::super::test_http::FakeHttp;
    use super::*;

    fn auth() -> Value {
        json!({ "authType": "custom", "apiUrl": "https://acct.api-us1.com", "apiKey": "k" })
    }

    #[test]
    fn normalizes_urls() {
        assert_eq!(
            normalize_api_url(" https://a.api-us1.com/api/3/ "),
            "https://a.api-us1.com"
        );
        assert_eq!(
            normalize_api_url("https://a.api-us1.com//"),
            "https://a.api-us1.com"
        );
    }

    #[tokio::test]
    async fn sync_sends_contact_and_reads_numeric_id() {
        let http = FakeHttp::with(vec![(201, r#"{"contact":{"id":42}}"#)]);
        let id = sync_contact(
            &http,
            &auth(),
            "a@b.co",
            Some("+1"),
            &[("7".into(), "x".into())],
        )
        .await
        .expect("id");
        assert_eq!(id, "42");
        let request = &http.requests()[0];
        assert_eq!(request.url, "https://acct.api-us1.com/api/3/contact/sync");
        assert_eq!(
            request.body,
            Some(
                json!({ "contact": { "email": "a@b.co", "phone": "+1", "fieldValues": [{ "field": "7", "value": "x" }] } })
            )
        );
        assert!(request.headers.contains(&("Api-Token", "k".into())));
    }

    #[tokio::test]
    async fn automation_skips_existing_and_conflicts() {
        let http = FakeHttp::with(vec![(200, r#"{"contactAutomations":[{"seriesid":"9"}]}"#)]);
        add_contact_to_automation(&http, &auth(), "1", "9")
            .await
            .expect("skip");
        assert_eq!(http.requests().len(), 1);
        let http = FakeHttp::with(vec![
            (200, r#"{"contactAutomations":[]}"#),
            (
                422,
                r#"{"errors":[{"title":"Could not create SubscriberSeries"}]}"#,
            ),
        ]);
        add_contact_to_automation(&http, &auth(), "1", "9")
            .await
            .expect("conflict ok");
        let http = FakeHttp::with(vec![(200, "{}"), (500, r#"{"message":"boom"}"#)]);
        let err = add_contact_to_automation(&http, &auth(), "1", "9")
            .await
            .unwrap_err();
        assert_eq!(err.message, "boom");
    }

    #[tokio::test]
    async fn custom_field_labels_fall_back() {
        let http = FakeHttp::with(vec![(
            200,
            r#"{"fields":[{"id":"1","perstag":"P"},{"id":2}]}"#,
        )]);
        let fields = list_custom_fields(&http, &auth()).await.expect("fields");
        assert_eq!(
            fields,
            vec![json!({"id":"1","label":"P"}), json!({"id":"2","label":"2"})]
        );
    }
}
