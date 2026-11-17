//! `integrations/klaviyo/src/{client,integration}.ts` and
//! `klaviyo-handler.ts` (`syncKlaviyoProfile`).

use chatbotx_db::Db;
use serde_json::{Map, Value, json};

use super::{
    Fields, Http, Outcome, ProviderError, Request, auth_str, contact_field_map, field, is_email,
    load_auth, redact_emails, run_exclusive, sha256, split_full_name, step_str,
};

const BASE_URL: &str = "https://a.klaviyo.com/api/";
const REVISION: &str = "2026-04-15";
pub const LIST_PAGE_SIZE: u32 = 10;
pub const LOCK_TIMEOUT_SECONDS: u64 = 30;

/// `klaviyoRequest`: a 2xx outside `accepted` is an error too.
async fn request(
    http: &dyn Http,
    auth: &Value,
    req: Request,
    accepted: &[u16],
) -> Outcome<Option<Value>> {
    let req = req
        .header(
            "Authorization",
            format!("Klaviyo-API-Key {}", auth_str(auth, "apiKey").trim()),
        )
        .header("revision", REVISION);
    let message = |payload: Option<&Value>, status: u16| {
        let first = payload
            .and_then(|p| p.get("errors"))
            .and_then(Value::as_array)
            .and_then(|e| e.first());
        first
            .and_then(|f| f.get("detail").and_then(Value::as_str))
            .or_else(|| first.and_then(|f| f.get("title").and_then(Value::as_str)))
            .map_or_else(|| format!("Klaviyo API returned {status}"), redact_emails)
    };
    let reply = super::call(http, req).await?;
    let payload = reply.payload();
    if !reply.ok() || !accepted.contains(&reply.status) {
        return Err(ProviderError::http(
            message(payload.as_ref(), reply.status),
            reply.status,
            &reply,
        ));
    }
    Ok(payload)
}

/// `listLists`: `{ data: [{ id, name }], nextCursor }`.
pub async fn list_lists(
    http: &dyn Http,
    auth: &Value,
    cursor: Option<&str>,
    size: u32,
) -> Outcome<Value> {
    let mut url = url::Url::parse(&format!("{BASE_URL}lists")).expect("url");
    url.query_pairs_mut()
        .append_pair("page[size]", &size.to_string());
    if let Some(cursor) = cursor {
        url.query_pairs_mut().append_pair("page[cursor]", cursor);
    }
    let payload = request(http, auth, Request::new("GET", url.as_str()), &[200]).await?;
    let invalid = || super::invalid_response("Klaviyo");
    let data = payload
        .as_ref()
        .and_then(|p| p.get("data"))
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let lists = data
        .iter()
        .map(|item| {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty());
            let name = item
                .pointer("/attributes/name")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty());
            match (item.get("type").and_then(Value::as_str), id, name) {
                (Some("list"), Some(id), Some(name)) => {
                    Ok(json!({ "id": id.trim(), "name": name.trim() }))
                }
                _ => Err(invalid()),
            }
        })
        .collect::<Outcome<Vec<_>>>()?;
    let next_cursor = payload
        .as_ref()
        .and_then(|p| p.pointer("/links/next"))
        .and_then(Value::as_str)
        .and_then(|next| url::Url::parse(next).ok())
        .and_then(|next| {
            next.query_pairs()
                .find(|(k, _)| k == "page[cursor]")
                .map(|(_, v)| v.into_owned())
        });
    Ok(json!({ "data": lists, "nextCursor": next_cursor }))
}

/// `syncProfile`: profile import, then the list (if any). The profile id.
pub async fn sync_profile(
    http: &dyn Http,
    auth: &Value,
    attributes: Map<String, Value>,
    list_id: Option<&str>,
) -> Outcome<String> {
    let body = json!({ "data": { "type": "profile", "attributes": attributes } });
    let req = Request::new("POST", format!("{BASE_URL}profile-import")).json(body);
    let profile = request(http, auth, req, &[200, 201]).await?;
    let id = profile
        .as_ref()
        .and_then(|p| p.pointer("/data/id"))
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| super::invalid_response("Klaviyo"))?
        .to_owned();
    if let Some(list_id) = list_id {
        let path = format!(
            "{BASE_URL}lists/{}/relationships/profiles",
            super::drip::encode(list_id)
        );
        let req =
            Request::new("POST", path).json(json!({ "data": [{ "type": "profile", "id": id }] }));
        request(http, auth, req, &[204]).await?;
    }
    Ok(id)
}

/// `normalizePhone`: E.164 (`00` → `+`) or nothing.
fn normalize_phone(value: Option<String>) -> Option<String> {
    let phone: String = value?.chars().filter(|c| !c.is_whitespace()).collect();
    let candidate = phone
        .strip_prefix("00")
        .map_or(phone.clone(), |rest| format!("+{rest}"));
    let digits = candidate.strip_prefix('+')?;
    let valid = (8..=15).contains(&digits.len())
        && digits.bytes().all(|b| b.is_ascii_digit())
        && !digits.starts_with('0');
    valid.then_some(candidate)
}

/// `buildKlaviyoProfileProps`: the attributes and the list id.
pub fn profile_props(
    fields: &Fields,
    step: &Value,
) -> Outcome<(Map<String, Value>, Option<String>)> {
    let email = field(fields, step_str(step, "emailField"))
        .map(|e| e.to_lowercase())
        .ok_or_else(|| ProviderError::new("Klaviyo profile email is empty"))?;
    if !is_email(&email) {
        return Err(ProviderError::new("Invalid email"));
    }
    let (first, last) = split_full_name(fields.get("full_name").map_or("", String::as_str));
    let mut attributes = Map::new();
    attributes.insert("email".into(), json!(email));
    let optional = [
        (
            "first_name",
            field(fields, Some("first_name")).or((!first.is_empty()).then_some(first)),
        ),
        (
            "last_name",
            field(fields, Some("last_name")).or((!last.is_empty()).then_some(last)),
        ),
        (
            "phone_number",
            normalize_phone(field(fields, Some("phone"))),
        ),
        ("title", field(fields, step_str(step, "titleField"))),
        ("organization", field(fields, step_str(step, "orgField"))),
    ];
    for (key, value) in optional {
        if let Some(value) = value {
            attributes.insert(key.into(), json!(value));
        }
    }
    let properties: Map<String, Value> = super::mappings(step, "contactFieldId", "klaviyoProperty")
        .into_iter()
        .filter(|(_, to)| !to.trim().is_empty())
        .filter_map(|(from, to)| Some((to.to_owned(), json!(field(fields, Some(from))?))))
        .collect();
    if !properties.is_empty() {
        attributes.insert("properties".into(), Value::Object(properties));
    }
    let list_id = step_str(step, "listId")
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_owned());
    Ok((attributes, list_id))
}

/// `syncKlaviyoProfile`.
pub async fn sync_profile_step(
    db: &Db,
    http: &dyn Http,
    workspace_id: i64,
    contact_id: i64,
    step: &Value,
) -> Outcome<()> {
    let auth = load_auth(db, "klaviyo", workspace_id).await?;
    let fields = contact_field_map(db, workspace_id, contact_id).await?;
    let (attributes, list_id) = profile_props(&fields, step)?;
    let email = attributes["email"].as_str().unwrap_or_default().to_owned();
    let key = format!(
        "klaviyo:sync-profile:{}:{}",
        sha256(&workspace_id.to_string()),
        sha256(&email)
    );
    run_exclusive(&key, LOCK_TIMEOUT_SECONDS, async || {
        let id = sync_profile(http, &auth, attributes.clone(), list_id.as_deref()).await?;
        tracing::info!(workspace_id, profile_id = %id, "Klaviyo profile sync completed");
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::super::test_http::{FakeHttp, fields};
    use super::*;

    #[test]
    fn builds_profile_props() {
        let step = json!({ "emailField": "email", "titleField": "9", "listId": "L", "mergeFields": [{ "contactFieldId": "8", "klaviyoProperty": "plan" }] });
        let (attrs, list) = profile_props(
            &fields(&[
                ("email", "A@B.co"),
                ("full_name", "Ada King"),
                ("phone", "0049 151 2345678"),
                ("9", "CTO"),
                ("8", "pro"),
            ]),
            &step,
        )
        .unwrap();
        assert_eq!(
            Value::Object(attrs),
            json!({ "email": "a@b.co", "first_name": "Ada", "last_name": "King", "phone_number": "+491512345678", "title": "CTO", "properties": { "plan": "pro" } })
        );
        assert_eq!(list.as_deref(), Some("L"));
        assert_eq!(normalize_phone(Some("12345".into())), None);
    }

    #[tokio::test]
    async fn sync_imports_then_subscribes() {
        let http = FakeHttp::with(vec![
            (
                201,
                r#"{"data":{"type":"profile","id":"P1","attributes":{"email":"a@b.co"}}}"#,
            ),
            (204, ""),
        ]);
        let mut attrs = Map::new();
        attrs.insert("email".into(), json!("a@b.co"));
        let id = sync_profile(&http, &json!({ "apiKey": "pk" }), attrs, Some("L 1"))
            .await
            .unwrap();
        assert_eq!(id, "P1");
        let requests = http.requests();
        assert_eq!(
            requests[1].url,
            "https://a.klaviyo.com/api/lists/L%201/relationships/profiles"
        );
        assert!(requests[0].headers.contains(&("revision", REVISION.into())));
        let http = FakeHttp::with(vec![(400, r#"{"errors":[{"detail":"bad a@b.co"}]}"#)]);
        let err = sync_profile(&http, &json!({}), Map::new(), None)
            .await
            .unwrap_err();
        assert_eq!(err.message, "bad [redacted]");
    }

    #[tokio::test]
    async fn lists_read_next_cursor() {
        let http = FakeHttp::with(vec![(
            200,
            r#"{"data":[{"type":"list","id":"L","attributes":{"name":"N"}}],"links":{"next":"https://a.klaviyo.com/api/lists?page%5Bcursor%5D=abc"}}"#,
        )]);
        let page = list_lists(&http, &json!({}), None, 10).await.unwrap();
        assert_eq!(
            page,
            json!({ "data": [{ "id": "L", "name": "N" }], "nextCursor": "abc" })
        );
    }
}
