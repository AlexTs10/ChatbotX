//! `integrations/get-response/src/{client,integration}.ts` and
//! `get-response-handler.ts` (`addGetResponseContact`).

use chatbotx_db::Db;
use serde_json::{Value, json};

use super::{
    Fields, Http, Outcome, ProviderError, Reply, Request, auth_str, call, contact_field_map, field,
    load_auth, non_empty, redact_emails, run_exclusive, sha256, step_str, step_strings, text_at,
};

const BASE_URL: &str = "https://api.getresponse.com/v3/";
pub const LOCK_TIMEOUT_SECONDS: u64 = 30;

/// `getResponseRequest`: the answer and its `X-Total-Count`.
async fn request(
    http: &dyn Http,
    auth: &Value,
    req: Request,
    accepted: u16,
) -> Outcome<(Option<Value>, Reply)> {
    let req = req.header(
        "X-Auth-Token",
        format!("api-key {}", auth_str(auth, "apiKey").trim()),
    );
    let reply = call(http, req).await?;
    let payload = reply.payload();
    if reply.status != accepted {
        let message = text_at(payload.as_ref(), &["message"]).map_or_else(
            || format!("GetResponse API returned {}", reply.status),
            redact_emails,
        );
        return Err(ProviderError::http(message, reply.status, &reply));
    }
    Ok((payload, reply))
}

/// `listCampaigns` / `listTags`: `{ data, meta }` of one page.
async fn page(
    http: &dyn Http,
    auth: &Value,
    path: &str,
    id_key: &str,
    page: u32,
    per_page: u32,
) -> Outcome<Value> {
    let url = format!("{BASE_URL}{path}?page={page}&perPage={per_page}");
    let (payload, reply) = request(http, auth, Request::new("GET", url), 200).await?;
    let Some(Value::Array(items)) = payload else {
        return Err(super::invalid_response("GetResponse"));
    };
    let valid = |item: &Value, key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty())
    };
    if !items.iter().all(|i| valid(i, id_key) && valid(i, "name")) {
        return Err(super::invalid_response("GetResponse"));
    }
    let total = reply
        .header("x-total-count")
        .and_then(|t| t.parse::<u64>().ok())
        .unwrap_or(items.len() as u64);
    let last_page = total.div_ceil(u64::from(per_page.max(1))).max(1);
    Ok(json!({
        "data": items,
        "meta": { "currentPage": page, "lastPage": last_page, "perPage": per_page, "total": total },
    }))
}

pub async fn list_campaigns(
    http: &dyn Http,
    auth: &Value,
    page_no: u32,
    per_page: u32,
) -> Outcome<Value> {
    page(http, auth, "campaigns", "campaignId", page_no, per_page).await
}

pub async fn list_tags(
    http: &dyn Http,
    auth: &Value,
    page_no: u32,
    per_page: u32,
) -> Outcome<Value> {
    page(http, auth, "tags", "tagId", page_no, per_page).await
}

/// `createOrUpdateContact`: 202 = queued; 409 (blacklisted /
/// unsubscribed) is ignored.
pub async fn create_or_update_contact(
    http: &dyn Http,
    auth: &Value,
    contact: Value,
) -> Outcome<()> {
    let req = Request::new("POST", format!("{BASE_URL}contacts")).json(contact);
    match request(http, auth, req, 202).await {
        Err(err) if err.status == Some(409) => Ok(()),
        other => other.map(|_| ()),
    }
}

/// `getContactName`: "first last" or the full name, 3+ characters.
fn contact_name(fields: &Fields) -> Option<String> {
    let composed = [
        field(fields, Some("first_name")),
        field(fields, Some("last_name")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    let name = non_empty(Some(&composed)).or_else(|| field(fields, Some("full_name")))?;
    (name.chars().count() >= 3).then_some(name)
}

/// `buildGetResponseContactProps`.
pub fn contact_props(fields: &Fields, step: &Value) -> Outcome<Value> {
    let email = field(fields, step_str(step, "emailField"))
        .map(|e| e.to_lowercase())
        .ok_or_else(|| ProviderError::new("GetResponse contact email is empty"))?;
    let campaign_id = non_empty(step_str(step, "campaignId").map(str::to_owned).as_ref())
        .ok_or_else(|| ProviderError::new("GetResponse list is not configured"))?;
    let mut contact = json!({ "email": email, "campaign": { "campaignId": campaign_id } });
    if let Some(name) = contact_name(fields) {
        contact["name"] = json!(name);
    }
    let tags = step_strings(step, "tags");
    if !tags.is_empty() {
        contact["tags"] = tags.iter().map(|t| json!({ "tagId": t })).collect();
    }
    // `Number.parseInt`: leading digits.
    if let Some(day) = step_str(step, "dayOfCycle").and_then(|d| {
        let digits: String = d.trim().chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<u64>().ok()
    }) {
        contact["dayOfCycle"] = json!(day);
    }
    Ok(contact)
}

/// `addGetResponseContact`.
pub async fn add_contact_step(
    db: &Db,
    http: &dyn Http,
    workspace_id: i64,
    contact_id: i64,
    step: &Value,
) -> Outcome<()> {
    let auth = load_auth(db, "getResponse", workspace_id).await?;
    let fields = contact_field_map(db, workspace_id, contact_id).await?;
    let contact = contact_props(&fields, step)?;
    let email = contact["email"].as_str().unwrap_or_default();
    let key = format!(
        "get-response:sync-contact:{}:{}",
        sha256(&workspace_id.to_string()),
        sha256(email)
    );
    run_exclusive(&key, LOCK_TIMEOUT_SECONDS, async || {
        create_or_update_contact(http, &auth, contact.clone()).await
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::super::test_http::{FakeHttp, fields};
    use super::*;

    #[test]
    fn builds_contact_props() {
        let step = json!({ "emailField": "email", "campaignId": " c1 ", "tags": ["t"], "dayOfCycle": "3" });
        let props = contact_props(
            &fields(&[("email", " A@B.co "), ("first_name", "Al")]),
            &step,
        )
        .unwrap();
        assert_eq!(
            props,
            json!({ "email": "a@b.co", "campaign": { "campaignId": "c1" }, "tags": [{ "tagId": "t" }], "dayOfCycle": 3 })
        );
        let props = contact_props(
            &fields(&[
                ("email", "a@b.co"),
                ("first_name", "Al"),
                ("last_name", "B"),
            ]),
            &step,
        )
        .unwrap();
        assert_eq!(props["name"], "Al B");
        let err = contact_props(&fields(&[]), &step).unwrap_err();
        assert_eq!(err.message, "GetResponse contact email is empty");
        let err = contact_props(
            &fields(&[("email", "a@b.co")]),
            &json!({ "emailField": "email" }),
        )
        .unwrap_err();
        assert_eq!(err.message, "GetResponse list is not configured");
    }

    #[tokio::test]
    async fn contact_accepts_202_and_409_and_redacts() {
        let auth = json!({ "apiKey": "k" });
        let http = FakeHttp::with(vec![
            (202, ""),
            (409, "{}"),
            (400, r#"{"message":"bad a@b.co"}"#),
        ]);
        create_or_update_contact(&http, &auth, json!({}))
            .await
            .expect("202");
        create_or_update_contact(&http, &auth, json!({}))
            .await
            .expect("409");
        let err = create_or_update_contact(&http, &auth, json!({}))
            .await
            .unwrap_err();
        assert_eq!(err.message, "bad [redacted]");
        assert!(
            http.requests()[0]
                .headers
                .contains(&("X-Auth-Token", "api-key k".into()))
        );
    }

    #[tokio::test]
    async fn pages_campaigns() {
        let http = FakeHttp::with(vec![(200, r#"[{"campaignId":"c","name":"N"}]"#)]);
        let page = list_campaigns(&http, &json!({ "apiKey": "k" }), 1, 100)
            .await
            .unwrap();
        assert_eq!(
            page["meta"],
            json!({ "currentPage": 1, "lastPage": 1, "perPage": 100, "total": 1 })
        );
        assert_eq!(
            http.requests()[0].url,
            "https://api.getresponse.com/v3/campaigns?page=1&perPage=100"
        );
    }
}
