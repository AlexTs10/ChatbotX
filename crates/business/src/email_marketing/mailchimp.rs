//! `integrations/mailchimp/src/{client,integration}.ts` and the input of
//! `mailchimp-handler.ts` (merge-field formatting).

use base64::Engine as _;
use md5::{Digest, Md5};
use serde_json::{Map, Value, json};

use super::{Http, Outcome, ProviderError, Request, auth_str, call};

pub const PAGE_SIZE: u32 = 1000;

/// `getSubscriberHash`: md5 of the trimmed, lower-case e-mail.
pub fn subscriber_hash(email: &str) -> String {
    let digest = Md5::digest(email.trim().to_lowercase().as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// `mailchimpRequest`: the parsed JSON answer (`None` for 204 / no JSON).
pub async fn request(
    http: &dyn Http,
    auth: &Value,
    method: &'static str,
    endpoint: &str,
    body: Option<Value>,
) -> Outcome<Option<Value>> {
    let base = format!(
        "https://{}.api.mailchimp.com/3.0/",
        auth_str(auth, "dataCenter")
    );
    let basic = base64::engine::general_purpose::STANDARD
        .encode(format!("anystring:{}", auth_str(auth, "apiKey")));
    let mut req = Request::new(method, format!("{base}{endpoint}"))
        .header("Authorization", format!("Basic {basic}"));
    if let Some(body) = body {
        req = req.json(body);
    }
    let reply = call(http, req).await?;
    let payload = reply.payload();
    if !reply.ok() {
        let errors: Vec<String> = payload
            .as_ref()
            .and_then(|p| p.get("errors"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|e| {
                        Some(format!(
                            "{}: {}",
                            e.get("field")?.as_str()?,
                            e.get("message")?.as_str()?
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let text = |key| {
            payload
                .as_ref()
                .and_then(|p| p.get(key))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let message = if errors.is_empty() {
            text("detail").or_else(|| text("title"))
        } else {
            Some(errors.join(", "))
        }
        .unwrap_or_else(|| format!("Mailchimp API returned {}", reply.status));
        return Err(ProviderError::http(message, reply.status, &reply));
    }
    Ok(payload)
}

fn list_of(payload: Option<Value>, key: &str) -> Outcome<Vec<Value>> {
    match payload.as_ref().and_then(|p| p.get(key)) {
        None | Some(Value::Null) if payload.as_ref().is_some_and(Value::is_object) => {
            Ok(Vec::new())
        }
        Some(Value::Array(items)) => Ok(items.clone()),
        _ => Err(super::invalid_response("Mailchimp")),
    }
}

/// `listAudiences`.
pub async fn list_audiences(http: &dyn Http, auth: &Value) -> Outcome<Vec<Value>> {
    let payload = request(
        http,
        auth,
        "GET",
        &format!("lists?count={PAGE_SIZE}&offset=0"),
        None,
    )
    .await?;
    Ok(list_of(payload, "lists")?
        .into_iter()
        .map(|l| json!({ "id": l["id"], "name": l["name"] }))
        .collect())
}

/// `listTags`.
pub async fn list_tags(http: &dyn Http, auth: &Value, list_id: &str) -> Outcome<Vec<Value>> {
    let endpoint = format!("lists/{list_id}/tag-search?count={PAGE_SIZE}");
    let payload = request(http, auth, "GET", &endpoint, None).await?;
    Ok(list_of(payload, "tags")?
        .into_iter()
        .map(|t| json!({ "id": t["id"], "name": t["name"] }))
        .collect())
}

/// `listMergeFields`.
pub async fn list_merge_fields(
    http: &dyn Http,
    auth: &Value,
    list_id: &str,
) -> Outcome<Vec<Value>> {
    let endpoint = format!("lists/{list_id}/merge-fields?count={PAGE_SIZE}");
    let payload = request(http, auth, "GET", &endpoint, None).await?;
    Ok(list_of(payload, "merge_fields")?
        .into_iter()
        .map(|f| {
            let mut out = json!({ "tag": f["tag"], "name": f["name"], "type": f["type"] });
            if let Some(required) = f.get("required").filter(|r| r.is_boolean()) {
                out["required"] = required.clone();
            }
            out
        })
        .collect())
}

/// `addMember`: upsert by subscriber hash, then the tags (if any).
pub async fn add_member(
    http: &dyn Http,
    auth: &Value,
    list_id: &str,
    email: &str,
    double_opt_in: bool,
    tags: &[String],
    merge_fields: Map<String, Value>,
) -> Outcome<Value> {
    let hash = subscriber_hash(email);
    let status = if double_opt_in {
        "pending"
    } else {
        "subscribed"
    };
    let member = request(
        http,
        auth,
        "PUT",
        &format!("lists/{list_id}/members/{hash}"),
        Some(json!({
            "email_address": email,
            "status_if_new": status,
            "status": status,
            "merge_fields": merge_fields,
        })),
    )
    .await?
    .ok_or_else(|| super::invalid_response("Mailchimp"))?;
    if !tags.is_empty() {
        let tags: Vec<Value> = tags
            .iter()
            .map(|name| json!({ "name": name, "status": "active" }))
            .collect();
        request(
            http,
            auth,
            "POST",
            &format!("lists/{list_id}/members/{hash}/tags"),
            Some(json!({ "tags": tags })),
        )
        .await?;
    }
    Ok(member)
}

/// `shouldRetry`: everything but a non-429 4xx.
pub fn should_retry(error: &ProviderError) -> bool {
    !matches!(error.status, Some(400..=499) if error.status != Some(429))
}

fn valid_date(year: i32, month: u32, day: u32) -> bool {
    chrono::NaiveDate::from_ymd_opt(year, month, day).is_some()
}

fn date_parts(value: &str) -> Option<(i32, u32, u32)> {
    static ISO: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static DMY: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let iso = ISO.get_or_init(|| regex::Regex::new(r"^(\d{4})-(\d{2})-(\d{2})").expect("pattern"));
    let dmy =
        DMY.get_or_init(|| regex::Regex::new(r"^(\d{1,2})-(\d{1,2})-(\d{4})$").expect("pattern"));
    let checked = |y: i32, m: u32, d: u32| valid_date(y, m, d).then_some((y, m, d));
    if let Some(c) = iso.captures(value) {
        return checked(c[1].parse().ok()?, c[2].parse().ok()?, c[3].parse().ok()?);
    }
    if let Some(c) = dmy.captures(value) {
        return checked(c[3].parse().ok()?, c[2].parse().ok()?, c[1].parse().ok()?);
    }
    // ponytail: `new Date(value)` accepts more shapes than RFC 3339 / 2822;
    // add formats when a flow needs them.
    use chrono::Datelike as _;
    let date = chrono::DateTime::parse_from_rfc3339(value)
        .or_else(|_| chrono::DateTime::parse_from_rfc2822(value))
        .ok()?
        .with_timezone(&chrono::Utc);
    Some((date.year(), date.month(), date.day()))
}

fn format_date(value: &str) -> String {
    date_parts(value).map_or_else(String::new, |(y, m, d)| format!("{y:04}-{m:02}-{d:02}"))
}

fn format_birthday(value: &str) -> String {
    static MD: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let md = MD.get_or_init(|| regex::Regex::new(r"^(\d{1,2})/(\d{1,2})$").expect("pattern"));
    if let Some(c) = md.captures(value) {
        let (m, d): (u32, u32) = (c[1].parse().unwrap_or(0), c[2].parse().unwrap_or(0));
        if valid_date(2000, m, d) {
            return format!("{m:02}/{d:02}");
        }
    }
    date_parts(value).map_or_else(String::new, |(_, m, d)| format!("{m:02}/{d:02}"))
}

/// `formatMergeFieldValue`: `None` = the field is not sent (address).
pub fn format_merge_field_value(tag: &str, field_type: Option<&str>, raw: &str) -> Option<Value> {
    let value = raw.trim();
    let kind = field_type.unwrap_or(tag).to_lowercase();
    match kind.as_str() {
        "address" => None,
        "birthday" => Some(json!(format_birthday(value))),
        "date" => Some(json!(format_date(value))),
        "number" => Some(
            value
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .map_or_else(|| json!(""), |n| json!(n)),
        ),
        _ => Some(json!(value)),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_http::FakeHttp;
    use super::*;

    fn auth() -> Value {
        json!({ "authType": "custom", "apiKey": "key-us1", "dataCenter": "us1" })
    }

    #[test]
    fn hashes_and_formats() {
        assert_eq!(
            subscriber_hash(" Test@Example.com "),
            subscriber_hash("test@example.com")
        );
        assert_eq!(
            subscriber_hash("test@example.com"),
            "55502f40dc8b7c769880b10874abc9d0"
        );
        assert_eq!(
            format_merge_field_value("BDAY", Some("birthday"), "3/7"),
            Some(json!("03/07"))
        );
        assert_eq!(
            format_merge_field_value("BDAY", Some("birthday"), "1990-03-07"),
            Some(json!("03/07"))
        );
        assert_eq!(
            format_merge_field_value("D", Some("date"), "07-03-1990"),
            Some(json!("1990-03-07"))
        );
        assert_eq!(
            format_merge_field_value("D", Some("date"), "31-02-1990"),
            Some(json!(""))
        );
        assert_eq!(
            format_merge_field_value("N", Some("number"), "4.5"),
            Some(json!(4.5))
        );
        assert_eq!(
            format_merge_field_value("N", Some("number"), "x"),
            Some(json!(""))
        );
        assert_eq!(format_merge_field_value("ADDRESS", None, "x"), None);
        assert_eq!(
            format_merge_field_value("FNAME", Some("text"), " Ada "),
            Some(json!("Ada"))
        );
    }

    #[tokio::test]
    async fn add_member_upserts_then_tags() {
        let http = FakeHttp::with(vec![
            (200, r#"{"id":"m","email_address":"a@b.co"}"#),
            (204, ""),
        ]);
        let mut merge = Map::new();
        merge.insert("FNAME".into(), json!("Ada"));
        add_member(&http, &auth(), "L1", "a@b.co", true, &["vip".into()], merge)
            .await
            .expect("member");
        let requests = http.requests();
        assert_eq!(requests[0].method, "PUT");
        assert_eq!(
            requests[0].url,
            format!(
                "https://us1.api.mailchimp.com/3.0/lists/L1/members/{}",
                subscriber_hash("a@b.co")
            )
        );
        assert_eq!(requests[0].body.as_ref().unwrap()["status"], "pending");
        assert_eq!(
            requests[1].body.as_ref().unwrap()["tags"][0]["status"],
            "active"
        );
        assert!(
            requests[0]
                .headers
                .iter()
                .any(|(k, v)| *k == "Authorization" && v.starts_with("Basic "))
        );
    }

    #[tokio::test]
    async fn errors_join_field_messages() {
        let http = FakeHttp::with(vec![(
            400,
            r#"{"errors":[{"field":"email","message":"bad"}]}"#,
        )]);
        let err = list_audiences(&http, &auth()).await.unwrap_err();
        assert_eq!(err.message, "email: bad");
        assert!(!should_retry(&err));
        let http = FakeHttp::with(vec![(503, "")]);
        let err = list_audiences(&http, &auth()).await.unwrap_err();
        assert_eq!(err.message, "Mailchimp API returned 503");
        assert!(should_retry(&err));
    }
}
