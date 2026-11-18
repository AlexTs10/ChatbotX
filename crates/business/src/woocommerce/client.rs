//! `integrations/woocommerce/src/{client,constants,error,apis/*}.ts`: the
//! store's REST API (`/wp-json/wc/v3`). Bodies are read as JSON values.
//!
//! The merchant types the address, so every call checks it again with the
//! SSRF guard and pins the connection to the addresses it resolved to (no
//! DNS rebinding between the check and the call). Redirects are not
//! followed: the keys only go to the address the merchant entered.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use base64::Engine as _;
use serde_json::Value;

use crate::net::ssrf_guard::{HttpDoh, SsrfCheck, check_ssrf_safety_with};

const PAGE_SIZE: i64 = 100;
/// How far back a full sync reads orders.
pub const ORDER_SYNC_DAYS: i64 = 60;

/// `woocommerceAuthModes`: Basic auth, or the keys in the query string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Header,
    Query,
}

/// `WooCommerceCredentials`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub site_url: String,
    pub consumer_key: String,
    pub consumer_secret: String,
    pub auth_mode: AuthMode,
}

/// `WooCommerceApiError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub message: String,
    pub status: u16,
    pub code: Option<String>,
}

impl ApiError {
    fn new(message: impl Into<String>, status: u16, code: Option<&str>) -> ApiError {
        ApiError {
            message: message.into(),
            status,
            code: code.map(str::to_string),
        }
    }

    /// The keys no longer work: mark the connection invalid.
    pub fn unauthorized(&self) -> bool {
        self.status == 401
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;

/// `woocommerceAdminUrl`.
pub fn admin_url(site_url: &str) -> String {
    format!("{site_url}/wp-admin")
}

/// `plainText`: WooCommerce's messages may carry HTML.
fn plain_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut in_tag = false;
    for c in value.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// A client pinned to the addresses `url`'s host resolved to, or an error
/// when the address is not a public website.
async fn pinned_client(url: &str) -> ApiResult<reqwest::Client> {
    let unsafe_address = || {
        ApiError::new(
            "the store address no longer points at a public website",
            400,
            Some("unsafe_address"),
        )
    };
    let SsrfCheck::Safe { resolved_ips } = check_ssrf_safety_with(url, &HttpDoh).await else {
        return Err(unsafe_address());
    };
    let parsed = url::Url::parse(url).map_err(|_| unsafe_address())?;
    let host = parsed.host_str().unwrap_or_default().to_string();
    let port = parsed.port_or_known_default().unwrap_or(443);
    let addrs: Vec<SocketAddr> = resolved_ips
        .iter()
        .filter_map(|ip| ip.parse::<IpAddr>().ok())
        .map(|ip| SocketAddr::new(ip, port))
        .collect();
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none());
    if !addrs.is_empty() {
        builder = builder.resolve_to_addrs(&host, &addrs);
    }
    builder
        .build()
        .map_err(|e| ApiError::new(e.to_string(), 503, None))
}

fn query_string(credentials: &Credentials, query: &[(&str, String)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in query {
        serializer.append_pair(key, value);
    }
    if credentials.auth_mode == AuthMode::Query {
        serializer.append_pair("consumer_key", &credentials.consumer_key);
        serializer.append_pair("consumer_secret", &credentials.consumer_secret);
    }
    serializer.finish()
}

/// `woocommerceRequest`: the body and `X-WP-TotalPages` (1 when absent).
pub async fn request(
    credentials: &Credentials,
    method: reqwest::Method,
    path: &str,
    query: &[(&str, String)],
    body: Option<&Value>,
) -> ApiResult<(Value, i64)> {
    let base = format!("{}/wp-json/wc/v3{path}", credentials.site_url);
    let qs = query_string(credentials, query);
    let url = if qs.is_empty() {
        base
    } else {
        format!("{base}?{qs}")
    };
    let client = pinned_client(&url).await?;
    let mut req = client
        .request(method, &url)
        .header("Accept", "application/json");
    if credentials.auth_mode == AuthMode::Header {
        let basic = base64::engine::general_purpose::STANDARD.encode(format!(
            "{}:{}",
            credentials.consumer_key, credentials.consumer_secret
        ));
        req = req.header("Authorization", format!("Basic {basic}"));
    }
    if let Some(body) = body {
        req = req
            .header("Content-Type", "application/json")
            .body(body.to_string());
    }
    let response = req
        .send()
        .await
        .map_err(|e| ApiError::new(format!("The store could not be reached: {e}"), 503, None))?;
    let status = response.status().as_u16();
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    if (300..400).contains(&status) {
        let message = header("location").map_or_else(
            || "The store answered with a redirect.".to_string(),
            |l| {
                format!(
                    "The store redirected to {}; connect with that address instead.",
                    l.split('?').next().unwrap_or_default()
                )
            },
        );
        return Err(ApiError::new(message, 400, Some("redirect")));
    }
    let total_pages = header("x-wp-totalpages")
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|p| *p > 0)
        .unwrap_or(1);
    let payload: Option<Value> = response
        .bytes()
        .await
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    if !(200..300).contains(&status) {
        let payload = payload.unwrap_or(Value::Null);
        return Err(ApiError {
            message: payload["message"].as_str().map_or_else(
                || format!("WooCommerce REST API returned {status}"),
                plain_text,
            ),
            status,
            code: payload["code"].as_str().map(str::to_string),
        });
    }
    let Some(payload) = payload else {
        // A WordPress page instead of JSON: the REST API is not where we called.
        return Err(ApiError::new(
            "The store did not answer as a WooCommerce REST API. Check the address, and that WordPress permalinks are not set to Plain.",
            404,
            Some("not_rest_api"),
        ));
    };
    Ok((payload, total_pages))
}

async fn get(credentials: &Credentials, path: &str, query: &[(&str, String)]) -> ApiResult<Value> {
    Ok(
        request(credentials, reqwest::Method::GET, path, query, None)
            .await?
            .0,
    )
}

/// A 404 (deleted, or never there) reads as nothing.
async fn get_or_none(credentials: &Credentials, path: &str) -> ApiResult<Option<Value>> {
    match get(credentials, path, &[]).await {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.status == 404 => Ok(None),
        Err(e) => Err(e),
    }
}

/// `woocommerceListAll`: every page, one after the other.
pub async fn list_all(
    credentials: &Credentials,
    path: &str,
    query: &[(&str, String)],
) -> ApiResult<Vec<Value>> {
    let mut items = Vec::new();
    let mut page = 1;
    let mut total = 1;
    while page <= total {
        let mut q = query.to_vec();
        q.push(("page", page.to_string()));
        q.push(("per_page", PAGE_SIZE.to_string()));
        let (data, pages) = request(credentials, reqwest::Method::GET, path, &q, None).await?;
        items.extend(data.as_array().cloned().unwrap_or_default());
        total = pages;
        page += 1;
    }
    Ok(items)
}

fn segment(id: &str) -> String {
    url::form_urlencoded::byte_serialize(id.as_bytes()).collect()
}

pub async fn list_products(credentials: &Credentials) -> ApiResult<Vec<Value>> {
    let q = [
        ("status", "any".to_string()),
        ("orderby", "id".to_string()),
        ("order", "asc".to_string()),
    ];
    list_all(credentials, "/products", &q).await
}

pub async fn get_product(credentials: &Credentials, id: &str) -> ApiResult<Option<Value>> {
    get_or_none(credentials, &format!("/products/{}", segment(id))).await
}

pub async fn list_variations(credentials: &Credentials, product_id: i64) -> ApiResult<Vec<Value>> {
    list_all(
        credentials,
        &format!("/products/{product_id}/variations"),
        &[],
    )
    .await
}

pub async fn list_coupons(credentials: &Credentials) -> ApiResult<Vec<Value>> {
    let q = [("orderby", "id".to_string()), ("order", "asc".to_string())];
    list_all(credentials, "/coupons", &q).await
}

pub async fn get_coupon(credentials: &Credentials, id: &str) -> ApiResult<Option<Value>> {
    get_or_none(credentials, &format!("/coupons/{}", segment(id))).await
}

/// `listWooCommerceOrders`: created after `created_after` (ISO), oldest first.
pub async fn list_orders(credentials: &Credentials, created_after: &str) -> ApiResult<Vec<Value>> {
    let q = [
        ("after", created_after.to_string()),
        ("orderby", "date".to_string()),
        ("order", "asc".to_string()),
    ];
    list_all(credentials, "/orders", &q).await
}

pub async fn get_order(credentials: &Credentials, id: &str) -> ApiResult<Option<Value>> {
    get_or_none(credentials, &format!("/orders/{}", segment(id))).await
}

/// `WooCommerceStoreInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreInfo {
    pub name: Option<String>,
    /// ISO 4217, upper case.
    pub currency: String,
    pub currency_symbol: Option<String>,
    /// `None` when the key may not read the store's settings.
    pub currency_position: Option<String>,
}

/// `decodeHtmlEntities`: `&#8363;` -> `₫`.
pub fn decode_html_entities(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let decoded = tail.find(';').and_then(|end| {
            let body = &tail[1..end];
            let ch = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X"))
            {
                u32::from_str_radix(hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .map(String::from)
            } else if let Some(dec) = body.strip_prefix('#') {
                dec.parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                    .map(String::from)
            } else {
                match body.to_lowercase().as_str() {
                    "amp" => Some("&".into()),
                    "euro" => Some("€".into()),
                    "pound" => Some("£".into()),
                    "yen" => Some("¥".into()),
                    "cent" => Some("¢".into()),
                    _ => None,
                }
            };
            ch.map(|c| (c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push_str(&c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `getWooCommerceStoreInfo`: the currency as the store shows prices (it also
/// proves the keys work) and the site's name (best effort).
pub async fn get_store_info(credentials: &Credentials) -> ApiResult<StoreInfo> {
    let data = get(credentials, "/data/currencies/current", &[]).await?;
    let code = data["code"]
        .as_str()
        .filter(|c| !c.is_empty())
        .ok_or_else(|| ApiError::new("The store sent no currency.", 502, None))?
        .to_uppercase();
    let position = match get(
        credentials,
        "/settings/general/woocommerce_currency_pos",
        &[],
    )
    .await
    {
        Ok(setting) => match setting["value"].as_str() {
            Some("left") => Some("left"),
            Some("right") => Some("right"),
            Some("left_space") => Some("leftSpace"),
            Some("right_space") => Some("rightSpace"),
            _ => None,
        }
        .map(str::to_string),
        // A key whose user may not manage the store's settings still sells.
        Err(e) if e.status < 500 => None,
        Err(e) => return Err(e),
    };
    let index = format!("{}/wp-json/?_fields=name", credentials.site_url);
    let name = match pinned_client(&index).await {
        Ok(client) => match client.get(&index).send().await {
            Ok(response) => response
                .bytes()
                .await
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| v["name"].as_str().map(|n| n.trim().to_string()))
                .filter(|n| !n.is_empty()),
            Err(_) => None,
        },
        Err(_) => None,
    };
    let symbol = data["symbol"]
        .as_str()
        .map(|s| decode_html_entities(s).trim().to_string())
        .filter(|s| !s.is_empty());
    Ok(StoreInfo {
        name,
        currency: code,
        currency_symbol: symbol,
        currency_position: position,
    })
}

/// `createWooCommerceWebhook`: its id.
pub async fn create_webhook(
    credentials: &Credentials,
    topic: &str,
    delivery_url: &str,
    secret: &str,
) -> ApiResult<String> {
    let body = serde_json::json!({
        "name": format!("ChatbotX {topic}"),
        "topic": topic,
        "delivery_url": delivery_url,
        "secret": secret,
        "status": "active",
    });
    let (data, _) = request(
        credentials,
        reqwest::Method::POST,
        "/webhooks",
        &[],
        Some(&body),
    )
    .await?;
    data["id"].as_i64().map(|id| id.to_string()).ok_or_else(|| {
        ApiError::new(
            format!("The store created no webhook for {topic}."),
            502,
            None,
        )
    })
}

/// `listWooCommerceWebhooks`: ours (delivering to `delivery_url`).
pub async fn list_webhooks(credentials: &Credentials, delivery_url: &str) -> ApiResult<Vec<Value>> {
    let data = get(
        credentials,
        "/webhooks",
        &[("per_page", PAGE_SIZE.to_string())],
    )
    .await?;
    Ok(data
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|w| w["delivery_url"] == delivery_url)
        .collect())
}

/// `activateWooCommerceWebhook`.
pub async fn activate_webhook(credentials: &Credentials, id: &str) -> ApiResult<()> {
    let body = serde_json::json!({ "status": "active" });
    request(
        credentials,
        reqwest::Method::PUT,
        &format!("/webhooks/{}", segment(id)),
        &[],
        Some(&body),
    )
    .await
    .map(|_| ())
}

/// `deleteWooCommerceWebhook`.
pub async fn delete_webhook(credentials: &Credentials, id: &str) -> ApiResult<()> {
    request(
        credentials,
        reqwest::Method::DELETE,
        &format!("/webhooks/{}", segment(id)),
        &[("force", "true".to_string())],
        None,
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_and_text() {
        assert_eq!(decode_html_entities("&#8363;"), "₫");
        assert_eq!(
            decode_html_entities("&#x20AC; &euro; &foo; a&b"),
            "€ € &foo; a&b"
        );
        assert_eq!(
            plain_text(" <a href=\"x\">Settings</a> off "),
            "Settings off"
        );
    }

    #[test]
    fn query_keys() {
        let mut credentials = Credentials {
            site_url: "https://shop.example".into(),
            consumer_key: "ck".into(),
            consumer_secret: "cs".into(),
            auth_mode: AuthMode::Header,
        };
        assert_eq!(
            query_string(&credentials, &[("page", "1".into())]),
            "page=1"
        );
        credentials.auth_mode = AuthMode::Query;
        assert_eq!(
            query_string(&credentials, &[]),
            "consumer_key=ck&consumer_secret=cs"
        );
    }
}
