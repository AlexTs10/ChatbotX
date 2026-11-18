//! `integrations/moosend/src/{client,integration}.ts`, `moosend-handler.ts`
//! (`addOrUpdateMoosendContact`) and its subscribe quota
//! (`chatbotx_kv::moosend`, one process-wide store).

use std::sync::{Arc, OnceLock};

use chatbotx_db::Db;
use chatbotx_kv::{MemKv, SystemClock};
use serde_json::{Value, json};

use super::{
    Http, Outcome, ProviderError, Request, auth_str, contact_field_map, is_email, load_auth,
    retry_after, run_exclusive, sha256, step_str,
};

const BASE_URL: &str = "https://api.moosend.com/v3/";
pub const LOCK_TTL_SECONDS: u64 = 20;

/// `MoosendApiError` (`messages[kind]`).
fn moosend_error(kind: &'static str, status: u16, retry: Option<u64>) -> ProviderError {
    let message = match kind {
        "invalid_credentials" => "Moosend credentials are invalid",
        "invalid_response" => "Moosend returned an invalid response",
        "rate_limited" => "Moosend rate limit exceeded",
        "transport" => "Moosend request failed",
        "user_not_enabled" => "Moosend account is not enabled for API access",
        _ => "Moosend rejected the request",
    };
    ProviderError {
        message: message.into(),
        status: Some(status),
        kind: Some(kind),
        retry_after_seconds: retry,
    }
}

fn error_kind(status: u16, error: Option<&str>, message: Option<&str>) -> &'static str {
    if error == Some("USER_NOT_ENABLED") || message == Some("USER_NOT_ENABLED") {
        "user_not_enabled"
    } else if matches!(status, 401 | 403) {
        "invalid_credentials"
    } else if status == 429 {
        "rate_limited"
    } else {
        "provider"
    }
}

/// `moosendRequest`: the envelope's `Context`.
async fn request(
    http: &dyn Http,
    auth: &Value,
    method: &'static str,
    path: &str,
    body: Option<Value>,
) -> Outcome<Value> {
    let mut url = url::Url::parse(&format!("{BASE_URL}{path}"))
        .map_err(|e| ProviderError::new(e.to_string()))?;
    url.query_pairs_mut()
        .append_pair("apikey", auth_str(auth, "apiKey").trim());
    let mut req = Request::new(method, url.as_str());
    if let Some(body) = body {
        req = req.json(body);
    }
    let reply = http
        .send(req)
        .await
        .map_err(|_| moosend_error("transport", 503, None))?;
    let retry = retry_after(reply.header("retry-after"));
    let payload: Option<Value> = serde_json::from_str(&reply.body).ok();
    let code = payload
        .as_ref()
        .and_then(|p| p.get("Code"))
        .and_then(Value::as_i64);
    let text = |key: &str| {
        payload
            .as_ref()
            .and_then(|p| p.get(key))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    let Some(code) = code else {
        let kind = if reply.ok() {
            "invalid_response"
        } else {
            error_kind(reply.status, None, None)
        };
        return Err(moosend_error(kind, reply.status, retry));
    };
    let failed = code != 0 || text("Error").is_some() || text("ErrorMessage").is_some();
    if !(reply.ok() && reply.status == 200) || failed {
        let kind = error_kind(reply.status, text("Error"), text("ErrorMessage"));
        return Err(moosend_error(kind, reply.status, retry));
    }
    Ok(payload
        .and_then(|mut p| p.get_mut("Context").map(Value::take))
        .unwrap_or(Value::Null))
}

/// `listMailingLists`.
pub async fn list_mailing_lists(
    http: &dyn Http,
    auth: &Value,
    page: u32,
    page_size: u32,
) -> Outcome<Value> {
    let context = request(
        http,
        auth,
        "GET",
        &format!("lists/{page}/{page_size}.json"),
        None,
    )
    .await?;
    let invalid = || moosend_error("invalid_response", 200, None);
    let lists = context
        .get("MailingLists")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let data = lists
        .iter()
        .map(|l| {
            let text = |k: &str| {
                l.get(k)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            };
            Some(json!({ "id": text("ID")?, "name": text("Name")? }))
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(invalid)?;
    let paging = context.get("Paging").ok_or_else(invalid)?;
    let num = |k: &str| paging.get(k).and_then(Value::as_u64).ok_or_else(invalid);
    Ok(json!({
        "data": data,
        "meta": {
            "pageSize": num("PageSize")?,
            "currentPage": num("CurrentPage")?,
            "totalResults": num("TotalResults")?,
            "totalPageCount": num("TotalPageCount")?,
        },
    }))
}

/// `createOrUpdateContact`.
pub async fn create_or_update_contact(
    http: &dyn Http,
    auth: &Value,
    list_id: &str,
    email: &str,
) -> Outcome<Value> {
    let path = format!(
        "subscribers/{}/subscribe.json",
        super::drip::encode(list_id)
    );
    let context = request(http, auth, "POST", &path, Some(json!({ "Email": email }))).await?;
    let id = context
        .get("ID")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty());
    match (
        id,
        context.get("Email").and_then(Value::as_str),
        context.get("SubscribeType").and_then(Value::as_i64),
    ) {
        (Some(id), Some(email), Some(kind)) => {
            Ok(json!({ "id": id, "email": email, "subscribeType": kind }))
        }
        _ => Err(moosend_error("invalid_response", 200, None)),
    }
}

fn quota_store() -> &'static MemKv {
    static STORE: OnceLock<MemKv> = OnceLock::new();
    STORE.get_or_init(|| MemKv::new(Arc::new(SystemClock)))
}

/// `buildMoosendContactLockKey`.
pub fn lock_key(workspace_id: i64, list_id: &str, email: &str) -> String {
    format!(
        "moosend:subscribe:{}:{}:{}",
        sha256(&workspace_id.to_string()),
        sha256(list_id),
        sha256(email)
    )
}

/// `addOrUpdateMoosendContact`.
pub async fn create_contact_step(
    db: &Db,
    http: &dyn Http,
    workspace_id: i64,
    contact_id: i64,
    step: &Value,
) -> Outcome<()> {
    let auth = load_auth(db, "moosend", workspace_id).await?;
    let fields = contact_field_map(db, workspace_id, contact_id).await?;
    let list_id = step_str(step, "listId")
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    let email = step_str(step, "emailField")
        .and_then(|f| fields.get(f))
        .map(|e| e.trim().to_lowercase())
        .unwrap_or_default();
    if list_id.is_empty() || !is_email(&email) {
        return Err(ProviderError::new("Moosend contact data is invalid"));
    }
    chatbotx_kv::moosend::acquire_moosend_subscribe_permit(
        quota_store(),
        auth_str(&auth, "apiKey").trim(),
    )
    .map_err(|e| ProviderError {
        kind: Some("local_rate_limited"),
        retry_after_seconds: Some(e.retry_after_seconds),
        ..ProviderError::new(e.to_string())
    })?;
    run_exclusive(
        &lock_key(workspace_id, &list_id, &email),
        LOCK_TTL_SECONDS,
        async || {
            create_or_update_contact(http, &auth, &list_id, &email)
                .await
                .map(|_| ())
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::super::test_http::FakeHttp;
    use super::*;

    #[tokio::test]
    async fn subscribe_reads_envelope() {
        let auth = json!({ "apiKey": "k y" });
        let http = FakeHttp::with(vec![
            (
                200,
                r#"{"Code":0,"Error":null,"Context":{"ID":"s","Email":"a@b.co","SubscribeType":1}}"#,
            ),
            (200, r#"{"Code":0,"Error":"USER_NOT_ENABLED"}"#),
            (401, "nope"),
            (200, "nope"),
        ]);
        let out = create_or_update_contact(&http, &auth, "L", "a@b.co")
            .await
            .unwrap();
        assert_eq!(
            out,
            json!({ "id": "s", "email": "a@b.co", "subscribeType": 1 })
        );
        assert_eq!(
            http.requests()[0].url,
            "https://api.moosend.com/v3/subscribers/L/subscribe.json?apikey=k+y"
        );
        let err = create_or_update_contact(&http, &auth, "L", "a@b.co")
            .await
            .unwrap_err();
        assert_eq!(
            (err.kind, err.message.as_str()),
            (
                Some("user_not_enabled"),
                "Moosend account is not enabled for API access"
            )
        );
        assert_eq!(
            create_or_update_contact(&http, &auth, "L", "a")
                .await
                .unwrap_err()
                .kind,
            Some("invalid_credentials")
        );
        assert_eq!(
            create_or_update_contact(&http, &auth, "L", "a")
                .await
                .unwrap_err()
                .kind,
            Some("invalid_response")
        );
    }

    #[tokio::test]
    async fn lists_page() {
        let http = FakeHttp::with(vec![(
            200,
            r#"{"Code":0,"Context":{"Paging":{"PageSize":20,"CurrentPage":1,"TotalResults":1,"TotalPageCount":1},"MailingLists":[{"ID":"1","Name":"N"}]}}"#,
        )]);
        let page = list_mailing_lists(&http, &json!({ "apiKey": "k" }), 1, 20)
            .await
            .unwrap();
        assert_eq!(page["data"], json!([{ "id": "1", "name": "N" }]));
        assert_eq!(page["meta"]["totalPageCount"], 1);
    }
}
