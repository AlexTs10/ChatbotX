//! `integrations/sendgrid/src/{client,integration}.ts` and
//! `sendgrid-handler.ts` (`addSendGridContact`, with the import-job poll).

use std::time::Duration;

use chatbotx_db::Db;
use serde_json::{Map, Value, json};

use super::{
    Http, Outcome, ProviderError, Request, auth_str, contact_field_map, field, is_email, json_call,
    load_auth, sha256, step_str, text_at,
};

const BASE_URL: &str = "https://api.sendgrid.com/v3/";
const POLL_INTERVAL: Duration = Duration::from_secs(3);
const POLL_MAX_ATTEMPTS: u32 = 15;

/// `sendGridRequest`.
async fn request(
    http: &dyn Http,
    auth: &Value,
    method: &'static str,
    path: &str,
    body: Option<Value>,
) -> Outcome<Option<Value>> {
    let mut req = Request::new(method, format!("{BASE_URL}{path}")).header(
        "Authorization",
        format!("Bearer {}", auth_str(auth, "apiKey").trim()),
    );
    if let Some(body) = body {
        req = req.json(body);
    }
    json_call(http, req, |payload, status| {
        text_at(payload, &["errors", "0", "message"])
            .map_or_else(|| format!("SendGrid API returned {status}"), str::to_owned)
    })
    .await
}

fn trimmed<'a>(item: &'a Value, key: &str) -> Option<&'a str> {
    item.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// `listLists`: `{ data, nextPageToken, count }`.
pub async fn list_lists(
    http: &dyn Http,
    auth: &Value,
    page_size: u32,
    page_token: Option<&str>,
) -> Outcome<Value> {
    let mut path = format!("marketing/lists?page_size={page_size}");
    if let Some(token) = page_token {
        path.push_str(&format!("&page_token={}", super::drip::encode(token)));
    }
    let payload = request(http, auth, "GET", &path, None).await?;
    let metadata = payload.as_ref().and_then(|p| p.get("_metadata")).cloned();
    let invalid = || super::invalid_response("SendGrid");
    let data = super::items(payload, "result", "SendGrid")?
        .iter()
        .map(|l| {
            let mut out = json!({ "id": trimmed(l, "id")?, "name": trimmed(l, "name")? });
            if let Some(count) = l.get("contact_count").and_then(Value::as_u64) {
                out["contactCount"] = json!(count);
            }
            Some(out)
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(invalid)?;
    let next = metadata
        .as_ref()
        .and_then(|m| m.get("next"))
        .and_then(Value::as_str);
    let base_origin = url::Url::parse(BASE_URL).expect("url").origin();
    let next_page_token = next
        .and_then(|n| url::Url::parse(n).ok())
        .filter(|n| n.origin() == base_origin)
        .and_then(|n| {
            n.query_pairs()
                .find(|(k, _)| k == "page_token")
                .map(|(_, v)| v.trim().to_owned())
        })
        .filter(|t| !t.is_empty());
    let mut out = json!({ "data": data });
    if let Some(token) = next_page_token {
        out["nextPageToken"] = json!(token);
    }
    if let Some(count) = metadata
        .as_ref()
        .and_then(|m| m.get("count"))
        .and_then(Value::as_u64)
    {
        out["count"] = json!(count);
    }
    Ok(out)
}

/// `listCustomFields`.
pub async fn list_custom_fields(http: &dyn Http, auth: &Value) -> Outcome<Vec<Value>> {
    let payload = request(http, auth, "GET", "marketing/field_definitions", None).await?;
    super::items(payload, "custom_fields", "SendGrid")?
        .iter()
        .map(|f| {
            let kind =
                trimmed(f, "field_type").filter(|k| ["Text", "Number", "Date"].contains(k))?;
            Some(json!({ "id": trimmed(f, "id")?, "name": trimmed(f, "name")?, "fieldType": kind }))
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| super::invalid_response("SendGrid"))
}

/// `addOrUpdateContact`: the import job id.
pub async fn add_or_update_contact(
    http: &dyn Http,
    auth: &Value,
    payload: Value,
) -> Outcome<String> {
    let accepted = request(http, auth, "PUT", "marketing/contacts", Some(payload)).await?;
    text_at(accepted.as_ref(), &["job_id"])
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| super::invalid_response("SendGrid"))
}

/// `checkImportJob`.
pub async fn check_import_job(http: &dyn Http, auth: &Value, job_id: &str) -> Outcome<Value> {
    let job = request(
        http,
        auth,
        "GET",
        &format!("marketing/contacts/imports/{job_id}"),
        None,
    )
    .await?;
    let job = job.ok_or_else(|| super::invalid_response("SendGrid"))?;
    let status = job
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !["pending", "completed", "failed", "errored", "done"].contains(&status) {
        return Err(super::invalid_response("SendGrid"));
    }
    Ok(job)
}

/// `normalizeSendGridDate`: RFC 3339 / M/D/YYYY kept; ISO and day-first
/// dates become MM/DD/YYYY.
pub fn normalize_date(value: &str) -> String {
    static PATTERNS: std::sync::OnceLock<[regex::Regex; 4]> = std::sync::OnceLock::new();
    let [rfc3339, mdy, iso, dmy] = PATTERNS.get_or_init(|| {
        [
            r"^\d{4}-\d{2}-\d{2}T",
            r"^\d{1,2}/\d{1,2}/\d{4}$",
            r"^(\d{4})-(\d{2})-(\d{2})$",
            r"^(\d{1,2})[-/.](\d{1,2})[-/.](\d{4})$",
        ]
        .map(|p| regex::Regex::new(p).expect("pattern"))
    });
    if rfc3339.is_match(value) || mdy.is_match(value) {
        return value.to_owned();
    }
    if let Some(c) = iso.captures(value) {
        return format!("{}/{}/{}", &c[2], &c[3], &c[1]);
    }
    if let Some(c) = dmy.captures(value) {
        return format!("{:0>2}/{:0>2}/{}", &c[2], &c[1], &c[3]);
    }
    value.to_owned()
}

/// The `PUT marketing/contacts` body of the step.
pub fn contact_payload(fields: &super::Fields, step: &Value) -> Outcome<Value> {
    let email = field(fields, step_str(step, "emailField"))
        .map(|e| e.to_lowercase())
        .ok_or_else(|| ProviderError::local("SendGrid contact email is empty"))?;
    if !is_email(&email) {
        return Err(ProviderError::local(format!(
            "SendGrid contact email is invalid: {email}"
        )));
    }
    let (first_name, last_name) = super::drip::names(fields);
    let mut contact = json!({ "email": email });
    let optional = [
        ("first_name", first_name),
        ("last_name", last_name),
        ("phone_number", field(fields, step_str(step, "phoneField"))),
    ];
    for (key, value) in optional {
        if let Some(value) = value {
            contact[key] = json!(value);
        }
    }
    let mut custom = Map::new();
    for (from, to) in super::mappings(step, "contactFieldId", "sendGridField") {
        if let Some(raw) = field(fields, Some(from))
            && !custom
                .get(to)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        {
            let value = if to.ends_with("_D") {
                normalize_date(&raw)
            } else {
                raw
            };
            custom.insert(to.to_owned(), json!(value));
        }
    }
    if !custom.is_empty() {
        contact["custom_fields"] = Value::Object(custom);
    }
    let mut payload = json!({ "contacts": [contact] });
    if let Some(list) = step_str(step, "listId").filter(|l| !l.is_empty()) {
        payload["list_ids"] = json!([list]);
    }
    Ok(payload)
}

/// The poll verdict of one job state: `Some(Ok)` / `Some(Err)` = done.
fn job_verdict(job: &Value) -> Option<Outcome<()>> {
    let count = |key: &str| {
        job.pointer(&format!("/results/{key}"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let errored = count("errored_count");
    let errored_result = || {
        if errored > 0 {
            Err(ProviderError::local(format!(
                "SendGrid import job completed with {errored} errored record(s)"
            )))
        } else {
            Ok(())
        }
    };
    match job.get("status").and_then(Value::as_str) {
        Some("failed" | "errored") => Some(Err(ProviderError::local("SendGrid import job failed"))),
        Some("completed" | "done") => Some(errored_result()),
        _ => {
            let requested = count("requested_count");
            let processed = count("created_count") + count("updated_count") + errored;
            (requested > 0 && processed >= requested).then(errored_result)
        }
    }
}

/// `addSendGridContact`.
pub async fn add_contact_step(
    db: &Db,
    http: &dyn Http,
    workspace_id: i64,
    contact_id: i64,
    step: &Value,
) -> Outcome<()> {
    add_contact_step_with(db, http, workspace_id, contact_id, step, POLL_INTERVAL).await
}

pub(crate) async fn add_contact_step_with(
    db: &Db,
    http: &dyn Http,
    workspace_id: i64,
    contact_id: i64,
    step: &Value,
    interval: Duration,
) -> Outcome<()> {
    let auth = load_auth(db, "sendGrid", workspace_id).await?;
    let fields = contact_field_map(db, workspace_id, contact_id).await?;
    let payload = contact_payload(&fields, step)?;
    let job_id = add_or_update_contact(http, &auth, payload).await?;
    tracing::info!(workspace_id, job_id_fingerprint = %sha256(&job_id), "SendGrid accepted contact update, polling job status");
    for attempt in 0..POLL_MAX_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(interval).await;
        }
        let job = check_import_job(http, &auth, &job_id).await?;
        if let Some(verdict) = job_verdict(&job) {
            return verdict;
        }
    }
    tracing::warn!(
        workspace_id,
        "SendGrid import job still pending after max attempts, treating as success"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_http::{FakeHttp, fields};
    use super::*;

    #[test]
    fn dates_and_payload() {
        assert_eq!(normalize_date("2024-03-07"), "03/07/2024");
        assert_eq!(normalize_date("7.3.2024"), "03/07/2024");
        assert_eq!(normalize_date("3/7/2024"), "3/7/2024");
        assert_eq!(
            normalize_date("2024-03-07T10:00:00Z"),
            "2024-03-07T10:00:00Z"
        );
        let step = json!({ "emailField": "email", "listId": "L", "mergeFields": [{ "contactFieldId": "4", "sendGridField": "e1_D" }] });
        let payload = contact_payload(
            &fields(&[
                ("email", "a@b.co"),
                ("first_name", "Ada"),
                ("4", "2024-01-02"),
            ]),
            &step,
        )
        .unwrap();
        assert_eq!(
            payload,
            json!({ "list_ids": ["L"], "contacts": [{ "email": "a@b.co", "first_name": "Ada", "custom_fields": { "e1_D": "01/02/2024" } }] })
        );
        let err = contact_payload(&fields(&[("email", "nope")]), &step).unwrap_err();
        assert_eq!(err.message, "SendGrid contact email is invalid: nope");
    }

    #[test]
    fn job_verdicts() {
        assert!(job_verdict(&json!({ "status": "pending" })).is_none());
        assert!(matches!(
            job_verdict(&json!({ "status": "done" })),
            Some(Ok(()))
        ));
        assert!(matches!(
            job_verdict(
                &json!({ "status": "pending", "results": { "requested_count": 1, "created_count": 1 } })
            ),
            Some(Ok(()))
        ));
        let err = job_verdict(&json!({ "status": "completed", "results": { "errored_count": 2 } }))
            .unwrap()
            .unwrap_err();
        assert_eq!(
            err.message,
            "SendGrid import job completed with 2 errored record(s)"
        );
        assert_eq!(
            job_verdict(&json!({ "status": "failed" }))
                .unwrap()
                .unwrap_err()
                .message,
            "SendGrid import job failed"
        );
    }

    #[tokio::test]
    async fn lists_and_errors() {
        let auth = json!({ "apiKey": "k" });
        let http = FakeHttp::with(vec![
            (
                200,
                r#"{"result":[{"id":"1","name":"N","contact_count":3}],"_metadata":{"count":1,"next":"https://api.sendgrid.com/v3/marketing/lists?page_token=t2"}}"#,
            ),
            (400, r#"{"errors":[{"field":"email","message":"bad"}]}"#),
        ]);
        let page = list_lists(&http, &auth, 100, None).await.unwrap();
        assert_eq!(
            page,
            json!({ "data": [{ "id": "1", "name": "N", "contactCount": 3 }], "nextPageToken": "t2", "count": 1 })
        );
        assert_eq!(
            add_or_update_contact(&http, &auth, json!({}))
                .await
                .unwrap_err()
                .message,
            "bad"
        );
    }
}
