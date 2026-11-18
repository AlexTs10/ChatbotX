//! `integrations/drip/src/{client,integration}.ts` and `drip-handler.ts`
//! (`subscribeDripSubscriber`).

use base64::Engine as _;
use chatbotx_db::Db;
use serde_json::{Map, Value, json};

use super::{
    Fields, Http, Outcome, ProviderError, Request, auth_str, contact_field_map, field, items,
    json_call, load_auth, run_exclusive, sha256, split_full_name, step_str, step_strings, text_at,
};

const BASE_URL: &str = "https://api.getdrip.com/v2/";
pub const LOCK_TIMEOUT_SECONDS: u64 = 30;

/// `encodeURIComponent`.
pub(crate) fn encode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

/// `dripRequest`.
pub async fn request(
    http: &dyn Http,
    auth: &Value,
    method: &'static str,
    path: &str,
    body: Option<Value>,
) -> Outcome<Option<Value>> {
    let basic = base64::engine::general_purpose::STANDARD
        .encode(format!("{}:", auth_str(auth, "apiToken").trim()));
    let mut req = Request::new(method, format!("{BASE_URL}{path}"))
        .header("Authorization", format!("Basic {basic}"));
    if let Some(body) = body {
        req = req.json(body);
    }
    json_call(http, req, |payload, status| {
        text_at(payload, &["errors", "0", "message"])
            .map_or_else(|| format!("Drip API returned {status}"), str::to_owned)
    })
    .await
}

/// `listAccounts`: `{ id: String(id), name }`.
pub async fn list_accounts(http: &dyn Http, auth: &Value) -> Outcome<Vec<Value>> {
    let accounts = items(
        request(http, auth, "GET", "accounts", None).await?,
        "accounts",
        "Drip",
    )?;
    accounts
        .into_iter()
        .map(|account| {
            let id = match &account["id"] {
                Value::String(s)
                    if !s.trim().is_empty()
                        && !s.trim().starts_with('0')
                        && s.trim().bytes().all(|b| b.is_ascii_digit()) =>
                {
                    s.trim().to_owned()
                }
                Value::Number(n) if n.as_u64().is_some_and(|n| n > 0) => n.to_string(),
                _ => return Err(super::invalid_response("Drip")),
            };
            let mut out = json!({ "id": id });
            if let Some(name) = account.get("name").filter(|n| n.is_string()) {
                out["name"] = name.clone();
            }
            Ok(out)
        })
        .collect()
}

/// `listTags`.
pub async fn list_tags(http: &dyn Http, auth: &Value, account_id: &str) -> Outcome<Vec<Value>> {
    let path = format!("{}/tags", encode(account_id));
    items(
        request(http, auth, "GET", &path, None).await?,
        "tags",
        "Drip",
    )
}

/// `listCustomFields`: `{ identifier, label: identifier }`.
pub async fn list_custom_fields(
    http: &dyn Http,
    auth: &Value,
    account_id: &str,
) -> Outcome<Vec<Value>> {
    let path = format!("{}/custom_field_identifiers", encode(account_id));
    let ids = items(
        request(http, auth, "GET", &path, None).await?,
        "custom_field_identifiers",
        "Drip",
    )?;
    Ok(ids
        .into_iter()
        .filter_map(|id| {
            id.as_str()
                .map(|id| json!({ "identifier": id, "label": id }))
        })
        .collect())
}

/// The `syncSubscriber` props.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Subscriber {
    pub email: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub phone: Option<String>,
    pub tags: Vec<String>,
    pub custom_fields: Map<String, Value>,
}

/// `syncSubscriber`.
pub async fn sync_subscriber(
    http: &dyn Http,
    auth: &Value,
    account_id: &str,
    s: &Subscriber,
) -> Outcome<()> {
    let mut subscriber = json!({ "email": s.email });
    for (key, value) in [
        ("first_name", &s.first_name),
        ("last_name", &s.last_name),
        ("phone", &s.phone),
    ] {
        if let Some(value) = value {
            subscriber[key] = json!(value);
        }
    }
    if !s.tags.is_empty() {
        subscriber["tags"] = json!(s.tags);
    }
    if !s.custom_fields.is_empty() {
        subscriber["custom_fields"] = Value::Object(s.custom_fields.clone());
    }
    let path = format!("{}/subscribers", encode(account_id));
    request(
        http,
        auth,
        "POST",
        &path,
        Some(json!({ "subscribers": [subscriber] })),
    )
    .await?;
    Ok(())
}

/// The contact's first / last name, from `full_name` when empty.
pub(crate) fn names(fields: &Fields) -> (Option<String>, Option<String>) {
    let (first, last) = split_full_name(fields.get("full_name").map_or("", String::as_str));
    let first = field(fields, Some("first_name")).unwrap_or(first);
    let last = field(fields, Some("last_name")).unwrap_or(last);
    (
        (!first.is_empty()).then_some(first),
        (!last.is_empty()).then_some(last),
    )
}

/// `subscribeDripSubscriber`.
pub async fn subscribe_step(
    db: &Db,
    http: &dyn Http,
    workspace_id: i64,
    contact_id: i64,
    step: &Value,
) -> Outcome<()> {
    let auth = load_auth(db, "drip", workspace_id).await?;
    let fields = contact_field_map(db, workspace_id, contact_id).await?;
    let email = field(&fields, step_str(step, "emailField")).map(|e| e.to_lowercase());
    let email = email.ok_or_else(|| ProviderError::local("Drip subscriber email is empty"))?;
    let (first_name, last_name) = names(&fields);
    let mut custom_fields = Map::new();
    for (from, to) in super::mappings(step, "contactFieldId", "dripField") {
        if let Some(value) = field(&fields, Some(from))
            && !custom_fields
                .get(to)
                .is_some_and(|v: &Value| v.as_str().is_some_and(|s| !s.is_empty()))
        {
            custom_fields.insert(to.to_owned(), json!(value));
        }
    }
    let subscriber = Subscriber {
        phone: field(&fields, step_str(step, "phoneField")),
        email,
        first_name,
        last_name,
        tags: step_strings(step, "tags"),
        custom_fields,
    };
    let account_id = step_str(step, "accountId").unwrap_or_default();
    let key = format!(
        "drip:sync-subscriber:{}:{}",
        sha256(account_id),
        sha256(&subscriber.email)
    );
    run_exclusive(&key, LOCK_TIMEOUT_SECONDS, async || {
        sync_subscriber(http, &auth, account_id, &subscriber).await
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::super::test_http::{FakeHttp, fields};
    use super::*;

    #[tokio::test]
    async fn sync_posts_only_present_fields() {
        let http = FakeHttp::with(vec![(200, "{}")]);
        let auth = json!({ "authType": "custom", "apiToken": "tok" });
        let subscriber = Subscriber {
            email: "a@b.co".into(),
            first_name: Some("Ada".into()),
            ..Subscriber::default()
        };
        sync_subscriber(&http, &auth, "12 3", &subscriber)
            .await
            .expect("ok");
        let request = &http.requests()[0];
        assert_eq!(request.url, "https://api.getdrip.com/v2/12%203/subscribers");
        assert_eq!(
            request.body,
            Some(json!({ "subscribers": [{ "email": "a@b.co", "first_name": "Ada" }] }))
        );
        assert!(
            request
                .headers
                .contains(&("Authorization", "Basic dG9rOg==".into()))
        );
    }

    #[tokio::test]
    async fn errors_and_accounts() {
        let auth = json!({ "apiToken": "t" });
        let http = FakeHttp::with(vec![(401, r#"{"errors":[{"message":"nope"}]}"#)]);
        assert_eq!(
            list_accounts(&http, &auth).await.unwrap_err().message,
            "nope"
        );
        let http = FakeHttp::with(vec![(200, r#"{"accounts":[{"id":7,"name":"A"}]}"#)]);
        assert_eq!(
            list_accounts(&http, &auth).await.unwrap(),
            vec![json!({"id":"7","name":"A"})]
        );
    }

    #[test]
    fn names_fall_back_to_full_name() {
        assert_eq!(
            names(&fields(&[("full_name", "Ada King Lovelace")])),
            (Some("Ada".into()), Some("King Lovelace".into()))
        );
        assert_eq!(
            names(&fields(&[("first_name", "A"), ("full_name", "X Y")])),
            (Some("A".into()), Some("Y".into()))
        );
    }
}
