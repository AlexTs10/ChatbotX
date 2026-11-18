//! `integrations/mailer-lite/src/{client,integration}.ts` and
//! `mailer-lite-handler.ts` (`addMailerLiteSubscriber`).

use chatbotx_db::Db;
use serde_json::{Map, Value, json};

use super::{
    Fields, Http, Outcome, ProviderError, Request, auth_str, contact_field_map, field, is_email,
    load_auth, redact_emails, run_exclusive, sha256, split_full_name, step_str, text_at,
};

const BASE_URL: &str = "https://connect.mailerlite.com/api/";
pub const LOCK_TIMEOUT_SECONDS: u64 = 30;

/// `mailerLiteRequest`: a 2xx outside `accepted` is an error too.
async fn request(
    http: &dyn Http,
    auth: &Value,
    req: Request,
    accepted: &[u16],
) -> Outcome<Option<Value>> {
    let req = req.header(
        "Authorization",
        format!("Bearer {}", auth_str(auth, "apiKey").trim()),
    );
    let reply = super::call(http, req).await?;
    let payload = reply.payload();
    if !reply.ok() || !accepted.contains(&reply.status) {
        let message = text_at(payload.as_ref(), &["message"]).map_or_else(
            || format!("MailerLite API returned {}", reply.status),
            redact_emails,
        );
        return Err(ProviderError::http(message, reply.status, &reply));
    }
    Ok(payload)
}

/// `listGroups` / `listFields`: `{ data, meta }` in camelCase.
async fn page(
    http: &dyn Http,
    auth: &Value,
    path: &str,
    keys: &[&str],
    page: u32,
    limit: u32,
) -> Outcome<Value> {
    let url = format!("{BASE_URL}{path}?page={page}&limit={limit}");
    let payload = request(http, auth, Request::new("GET", url), &[200]).await?;
    let invalid = || super::invalid_response("MailerLite");
    let payload = payload.ok_or_else(invalid)?;
    let data = payload
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let data = data
        .iter()
        .map(|item| {
            let mut out = Map::new();
            for key in keys {
                let value = item
                    .get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(invalid)?;
                out.insert((*key).into(), json!(value));
            }
            if out.contains_key("type")
                && !["text", "number", "date"].contains(&out["type"].as_str().unwrap_or(""))
            {
                return Err(invalid());
            }
            Ok(Value::Object(out))
        })
        .collect::<Outcome<Vec<_>>>()?;
    let meta = payload.get("meta").ok_or_else(invalid)?;
    let num = |key: &str| meta.get(key).and_then(Value::as_u64).ok_or_else(invalid);
    Ok(json!({
        "data": data,
        "meta": {
            "currentPage": num("current_page")?,
            "lastPage": num("last_page")?,
            "perPage": num("per_page")?,
            "total": num("total")?,
        },
    }))
}

pub async fn list_groups(
    http: &dyn Http,
    auth: &Value,
    page_no: u32,
    limit: u32,
) -> Outcome<Value> {
    page(http, auth, "groups", &["id", "name"], page_no, limit).await
}

pub async fn list_fields(
    http: &dyn Http,
    auth: &Value,
    page_no: u32,
    limit: u32,
) -> Outcome<Value> {
    page(
        http,
        auth,
        "fields",
        &["id", "name", "key", "type"],
        page_no,
        limit,
    )
    .await
}

/// `createOrUpdateSubscriber`.
pub async fn create_or_update_subscriber(
    http: &dyn Http,
    auth: &Value,
    subscriber: Value,
) -> Outcome<Value> {
    let req = Request::new("POST", format!("{BASE_URL}subscribers")).json(subscriber);
    request(http, auth, req, &[200, 201])
        .await?
        .and_then(|p| p.get("data").cloned())
        .ok_or_else(|| super::invalid_response("MailerLite"))
}

/// `buildMailerLiteSubscriberProps`.
pub fn subscriber_props(fields: &Fields, step: &Value) -> Outcome<Value> {
    let email = field(fields, step_str(step, "emailField"))
        .map(|e| e.to_lowercase())
        .ok_or_else(|| ProviderError::new("MailerLite subscriber email is empty"))?;
    if !is_email(&email) {
        return Err(ProviderError::new("Invalid email"));
    }
    let (first, last) = split_full_name(fields.get("full_name").map_or("", String::as_str));
    let mut subscriber_fields = Map::new();
    let reserved = [
        ("name", field(fields, Some("first_name")).or(Some(first))),
        ("last_name", field(fields, Some("last_name")).or(Some(last))),
        ("phone", field(fields, Some("phone"))),
    ];
    for (key, value) in reserved {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            subscriber_fields.insert(key.into(), json!(value));
        }
    }
    for (from, to) in super::mappings(step, "contactFieldId", "mailerLiteField") {
        if let Some(value) = field(fields, Some(from)) {
            subscriber_fields.insert(to.to_owned(), json!(value));
        }
    }
    let status = step_str(step, "status").unwrap_or("active");
    let mut subscriber = json!({ "email": email, "status": status });
    if let Some(group) = step_str(step, "groupId").filter(|g| !g.is_empty()) {
        subscriber["groups"] = json!([group]);
    }
    if !subscriber_fields.is_empty() {
        subscriber["fields"] = Value::Object(subscriber_fields);
    }
    Ok(subscriber)
}

/// `addMailerLiteSubscriber`.
pub async fn add_subscriber_step(
    db: &Db,
    http: &dyn Http,
    workspace_id: i64,
    contact_id: i64,
    step: &Value,
) -> Outcome<()> {
    let auth = load_auth(db, "mailerLite", workspace_id).await?;
    let fields = contact_field_map(db, workspace_id, contact_id).await?;
    let subscriber = subscriber_props(&fields, step)?;
    let email = subscriber["email"].as_str().unwrap_or_default().to_owned();
    let key = format!(
        "mailer-lite:sync-subscriber:{}:{}",
        sha256(&workspace_id.to_string()),
        sha256(&email)
    );
    run_exclusive(&key, LOCK_TIMEOUT_SECONDS, async || {
        create_or_update_subscriber(http, &auth, subscriber.clone())
            .await
            .map(|_| ())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::super::test_http::{FakeHttp, fields};
    use super::*;

    #[test]
    fn builds_subscriber_props() {
        let step = json!({ "emailField": "email", "status": "unconfirmed", "groupId": "g", "mergeFields": [{ "contactFieldId": "5", "mailerLiteField": "city" }] });
        let props = subscriber_props(
            &fields(&[
                ("email", "A@b.co"),
                ("full_name", "Ada King"),
                ("5", "Rome"),
            ]),
            &step,
        )
        .unwrap();
        assert_eq!(
            props,
            json!({ "email": "a@b.co", "status": "unconfirmed", "groups": ["g"], "fields": { "name": "Ada", "last_name": "King", "city": "Rome" } })
        );
        assert_eq!(
            subscriber_props(&fields(&[]), &step).unwrap_err().message,
            "MailerLite subscriber email is empty"
        );
    }

    #[tokio::test]
    async fn requests_and_pages() {
        let auth = json!({ "apiKey": "k" });
        let http = FakeHttp::with(vec![
            (
                201,
                r#"{"data":{"id":"1","email":"a@b.co","status":"active","fields":{}}}"#,
            ),
            (
                200,
                r#"{"data":[{"id":"1","name":"G"}],"meta":{"current_page":1,"last_page":2,"per_page":1,"total":2}}"#,
            ),
            (422, r#"{"message":"a@b.co is bad"}"#),
        ]);
        create_or_update_subscriber(&http, &auth, json!({}))
            .await
            .unwrap();
        let page = list_groups(&http, &auth, 1, 1).await.unwrap();
        assert_eq!(
            page["meta"],
            json!({ "currentPage": 1, "lastPage": 2, "perPage": 1, "total": 2 })
        );
        let err = create_or_update_subscriber(&http, &auth, json!({}))
            .await
            .unwrap_err();
        assert_eq!(
            (err.message.as_str(), err.status),
            ("[redacted] is bad", Some(422))
        );
        assert!(
            http.requests()[0]
                .headers
                .contains(&("Authorization", "Bearer k".into()))
        );
    }
}
