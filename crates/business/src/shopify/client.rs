//! `integrations/shopify/src/{client,constants,error,apis/*,lib/gid}.ts`:
//! the Admin GraphQL API calls the sync and the connection make. Bodies are
//! read as JSON values; the mappers pick the fields.
//!
//! The store is always `<name>.myshopify.com` (checked on connect), so the
//! calls need no SSRF guard.

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{Value, json};

pub const API_VERSION: &str = "2026-07";
const PRODUCT_PAGE_SIZE: i64 = 25;
const ORDER_PAGE_SIZE: i64 = 50;
const DISCOUNT_PAGE_SIZE: i64 = 50;
/// `read_orders` only reaches the last 60 days of orders.
pub const ORDER_SYNC_DAYS: i64 = 60;

/// `ShopifyCredentials`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub shop_domain: String,
    pub access_token: String,
}

/// `ShopifyApiError` / `ShopifyUserError` (`user_error`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub message: String,
    pub status: u16,
    pub retry_after_secs: Option<u64>,
    pub user_error: bool,
}

impl ApiError {
    fn new(message: impl Into<String>, status: u16) -> ApiError {
        ApiError {
            message: message.into(),
            status,
            retry_after_secs: None,
            user_error: false,
        }
    }

    /// The token or secret no longer works: mark the connection invalid.
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

/// `toShopifyGid`.
pub fn gid(kind: &str, id: impl std::fmt::Display) -> String {
    format!("gid://shopify/{kind}/{id}")
}

/// `shopifyAdminUrl`.
pub fn admin_url(shop_domain: &str) -> String {
    format!("https://{shop_domain}/admin")
}

fn http() -> ApiResult<&'static reqwest::Client> {
    static CLIENT: OnceLock<std::result::Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| ApiError::new(e.clone(), 503))
}

fn retry_after(response: &reqwest::Response) -> Option<u64> {
    let seconds: f64 = response
        .headers()
        .get("retry-after")?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| seconds.ceil() as u64)
}

fn rest_error_message(payload: &Value) -> Option<String> {
    match payload.get("errors")? {
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

async fn read(response: reqwest::Response) -> (u16, Option<u64>, Value) {
    let status = response.status().as_u16();
    let retry = retry_after(&response);
    let body = response
        .bytes()
        .await
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    (status, retry, body)
}

/// `shopifyGraphql`: `data` of one operation.
pub async fn graphql(credentials: &Credentials, query: &str, variables: Value) -> ApiResult<Value> {
    let url = format!(
        "https://{}/admin/api/{API_VERSION}/graphql.json",
        credentials.shop_domain
    );
    let response = http()?
        .post(url)
        .header("Accept", "application/json")
        .header("X-Shopify-Access-Token", &credentials.access_token)
        .header("Content-Type", "application/json")
        .body(json!({ "query": query, "variables": variables }).to_string())
        .send()
        .await
        .map_err(|e| ApiError::new(format!("Shopify could not be reached: {e}"), 503))?;
    let (status, retry, payload) = read(response).await;
    if !(200..300).contains(&status) {
        return Err(ApiError {
            message: rest_error_message(&payload)
                .unwrap_or_else(|| format!("Shopify Admin API returned {status}")),
            status,
            retry_after_secs: retry,
            user_error: false,
        });
    }
    if let Some(errors) = payload["errors"].as_array().filter(|e| !e.is_empty()) {
        let has = |code: &str| errors.iter().any(|e| e["extensions"]["code"] == code);
        let throttled = has("THROTTLED");
        let status = if throttled {
            429
        } else if has("ACCESS_DENIED") {
            403
        } else {
            400
        };
        return Err(ApiError {
            message: errors
                .iter()
                .map(|e| e["message"].as_str().unwrap_or_default())
                .collect::<Vec<_>>()
                .join("; "),
            status,
            retry_after_secs: throttled.then_some(2),
            user_error: false,
        });
    }
    Ok(payload["data"].clone())
}

/// `assertNoUserErrors`.
fn assert_no_user_errors(errors: &Value) -> ApiResult<()> {
    let Some(list) = errors.as_array().filter(|l| !l.is_empty()) else {
        return Ok(());
    };
    Err(ApiError {
        message: list
            .iter()
            .map(|e| e["message"].as_str().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("; "),
        status: 400,
        retry_after_secs: None,
        user_error: true,
    })
}

/// `requestShopifyAccessToken`: the client credentials grant.
/// Returns the token and its lifetime in seconds.
pub async fn request_access_token(
    shop_domain: &str,
    client_id: &str,
    client_secret: &str,
) -> ApiResult<(String, i64)> {
    let response = http()?
        .post(format!("https://{shop_domain}/admin/oauth/access_token"))
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(
            url::form_urlencoded::Serializer::new(String::new())
                .append_pair("grant_type", "client_credentials")
                .append_pair("client_id", client_id)
                .append_pair("client_secret", client_secret)
                .finish(),
        )
        .send()
        .await
        .map_err(|e| ApiError::new(format!("Shopify could not be reached: {e}"), 503))?;
    let (status, _, payload) = read(response).await;
    if !(200..300).contains(&status) {
        return Err(ApiError::new(
            rest_error_message(&payload)
                .unwrap_or_else(|| format!("Shopify refused the client credentials ({status})")),
            // A bad ID or secret answers 400; treat it as an auth failure.
            if status == 400 { 401 } else { status },
        ));
    }
    let token = payload["access_token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ApiError::new("Shopify issued no access token.", 502))?;
    Ok((
        token.to_string(),
        payload["expires_in"].as_i64().unwrap_or(86_400),
    ))
}

const PRODUCT_FIELDS: &str = "
  id title handle description descriptionHtml status productType vendor tags
  onlineStoreUrl tracksInventory totalInventory
  options { name position optionValues { name } }
  media(first: 10) { nodes { ... on MediaImage { image { url } } } }
  variants(first: 100) {
    nodes {
      id title sku price inventoryQuantity availableForSale
      selectedOptions { name value }
      inventoryItem { id tracked }
    }
  }";

/// `getShopifyShop`: `{ name, currencyCode, myshopifyDomain }`.
pub async fn get_shop(credentials: &Credentials) -> ApiResult<Value> {
    let data = graphql(
        credentials,
        "query Shop { shop { name currencyCode myshopifyDomain } }",
        json!({}),
    )
    .await?;
    let shop = data["shop"].clone();
    if !(shop["name"].is_string() && shop["currencyCode"].is_string()) {
        return Err(ApiError::new("Shopify answered without the shop.", 502));
    }
    Ok(shop)
}

/// A page of `{ pageInfo, nodes }`: the nodes and the next cursor.
fn page(connection: &Value) -> (Vec<Value>, Option<String>) {
    let nodes = connection["nodes"].as_array().cloned().unwrap_or_default();
    let next = (connection["pageInfo"]["hasNextPage"] == true)
        .then(|| {
            connection["pageInfo"]["endCursor"]
                .as_str()
                .map(str::to_string)
        })
        .flatten();
    (nodes, next)
}

/// `listShopifyProducts`.
pub async fn list_products(
    credentials: &Credentials,
    after: Option<&str>,
) -> ApiResult<(Vec<Value>, Option<String>)> {
    let query = format!(
        "query Products($first: Int!, $after: String) {{
          products(first: $first, after: $after, sortKey: ID) {{
            pageInfo {{ hasNextPage endCursor }}
            nodes {{ {PRODUCT_FIELDS} }}
          }}
        }}"
    );
    let data = graphql(
        credentials,
        &query,
        json!({ "first": PRODUCT_PAGE_SIZE, "after": after }),
    )
    .await?;
    Ok(page(&data["products"]))
}

/// `getShopifyProduct`: null once it was deleted.
pub async fn get_product(credentials: &Credentials, id: &str) -> ApiResult<Option<Value>> {
    let query = format!("query Product($id: ID!) {{ product(id: $id) {{ {PRODUCT_FIELDS} }} }}");
    let data = graphql(credentials, &query, json!({ "id": id })).await?;
    Ok(Some(data["product"].clone()).filter(|p| p.is_object()))
}

/// `getShopifyInventoryItemProductId`.
pub async fn get_inventory_item_product_id(
    credentials: &Credentials,
    id: &str,
) -> ApiResult<Option<String>> {
    let data = graphql(
        credentials,
        "query InventoryItem($id: ID!) { inventoryItem(id: $id) { variant { product { id } } } }",
        json!({ "id": id }),
    )
    .await?;
    Ok(data["inventoryItem"]["variant"]["product"]["id"]
        .as_str()
        .map(str::to_string))
}

const CODE_DISCOUNT_FIELDS: &str = "
  __typename
  ... on DiscountCodeBasic {
    title status startsAt endsAt usageLimit asyncUsageCount appliesOncePerCustomer
    codes(first: 1) { nodes { code } }
    minimumRequirement {
      __typename
      ... on DiscountMinimumSubtotal { greaterThanOrEqualToSubtotal { amount } }
      ... on DiscountMinimumQuantity { greaterThanOrEqualToQuantity }
    }
    customerGets {
      value {
        __typename
        ... on DiscountPercentage { percentage }
        ... on DiscountAmount { amount { amount } }
      }
      items {
        __typename
        ... on AllDiscountItems { allItems }
        ... on DiscountProducts { products(first: 100) { nodes { id } } }
      }
    }
  }
  ... on DiscountCodeFreeShipping {
    title status startsAt endsAt usageLimit asyncUsageCount appliesOncePerCustomer
    codes(first: 1) { nodes { code } }
    minimumRequirement {
      __typename
      ... on DiscountMinimumSubtotal { greaterThanOrEqualToSubtotal { amount } }
      ... on DiscountMinimumQuantity { greaterThanOrEqualToQuantity }
    }
  }";

/// `listShopifyCodeDiscounts`.
pub async fn list_code_discounts(
    credentials: &Credentials,
    after: Option<&str>,
) -> ApiResult<(Vec<Value>, Option<String>)> {
    let query = format!(
        "query CodeDiscounts($first: Int!, $after: String) {{
          codeDiscountNodes(first: $first, after: $after) {{
            pageInfo {{ hasNextPage endCursor }}
            nodes {{ id codeDiscount {{ {CODE_DISCOUNT_FIELDS} }} }}
          }}
        }}"
    );
    let data = graphql(
        credentials,
        &query,
        json!({ "first": DISCOUNT_PAGE_SIZE, "after": after }),
    )
    .await?;
    Ok(page(&data["codeDiscountNodes"]))
}

/// `getShopifyCodeDiscount`.
pub async fn get_code_discount(credentials: &Credentials, id: &str) -> ApiResult<Option<Value>> {
    let query = format!(
        "query CodeDiscount($id: ID!) {{
          codeDiscountNode(id: $id) {{ id codeDiscount {{ {CODE_DISCOUNT_FIELDS} }} }}
        }}"
    );
    let data = graphql(credentials, &query, json!({ "id": id })).await?;
    Ok(Some(data["codeDiscountNode"].clone()).filter(|p| p.is_object()))
}

pub(crate) const ORDER_FIELDS: &str = "
  id name createdAt processedAt cancelledAt cancelReason
  displayFinancialStatus displayFulfillmentStatus email phone note currencyCode
  customAttributes { key value }
  discountCodes
  subtotalPriceSet { shopMoney { amount currencyCode } }
  totalDiscountsSet { shopMoney { amount currencyCode } }
  totalShippingPriceSet { shopMoney { amount currencyCode } }
  totalTaxSet { shopMoney { amount currencyCode } }
  totalPriceSet { shopMoney { amount currencyCode } }
  customer { displayName email phone }
  shippingAddress { name firstName lastName address1 address2 city province zip countryCodeV2 phone }
  lineItems(first: 100) {
    nodes {
      id title variantTitle sku quantity
      originalUnitPriceSet { shopMoney { amount currencyCode } }
      totalDiscountSet { shopMoney { amount currencyCode } }
      variant { id }
      product { id }
      image { url }
    }
  }
  fulfillments(first: 10) {
    displayStatus
    trackingInfo(first: 1) { company number url }
  }";

/// `listShopifyOrders`: created since `created_after` (ISO), oldest first.
pub async fn list_orders(
    credentials: &Credentials,
    after: Option<&str>,
    created_after: &str,
) -> ApiResult<(Vec<Value>, Option<String>)> {
    let query = format!(
        "query Orders($first: Int!, $after: String, $query: String) {{
          orders(first: $first, after: $after, query: $query, sortKey: CREATED_AT) {{
            pageInfo {{ hasNextPage endCursor }}
            nodes {{ {ORDER_FIELDS} }}
          }}
        }}"
    );
    let data = graphql(
        credentials,
        &query,
        json!({
            "first": ORDER_PAGE_SIZE,
            "after": after,
            "query": format!("created_at:>={created_after}"),
        }),
    )
    .await?;
    Ok(page(&data["orders"]))
}

/// `getShopifyOrder`.
pub async fn get_order(credentials: &Credentials, id: &str) -> ApiResult<Option<Value>> {
    let query = format!("query Order($id: ID!) {{ order(id: $id) {{ {ORDER_FIELDS} }} }}");
    let data = graphql(credentials, &query, json!({ "id": id })).await?;
    Ok(Some(data["order"].clone()).filter(|p| p.is_object()))
}

/// `createShopifyWebhookSubscription`: the subscription's GID.
pub async fn create_webhook_subscription(
    credentials: &Credentials,
    topic: &str,
    uri: &str,
) -> ApiResult<String> {
    let data = graphql(
        credentials,
        "mutation WebhookCreate($topic: WebhookSubscriptionTopic!, $webhookSubscription: WebhookSubscriptionInput!) {
          webhookSubscriptionCreate(topic: $topic, webhookSubscription: $webhookSubscription) {
            webhookSubscription { id }
            userErrors { field message }
          }
        }",
        json!({ "topic": topic, "webhookSubscription": { "uri": uri, "format": "JSON" } }),
    )
    .await?;
    let created = &data["webhookSubscriptionCreate"];
    assert_no_user_errors(&created["userErrors"])?;
    created["webhookSubscription"]["id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| ApiError::new(format!("Shopify created no subscription for {topic}."), 502))
}

/// `listShopifyWebhookSubscriptionIds`: ours delivering to `uri`.
pub async fn list_webhook_subscription_ids(
    credentials: &Credentials,
    uri: &str,
) -> ApiResult<Vec<String>> {
    let mut ids = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let data = graphql(
            credentials,
            "query WebhookSubscriptions($after: String) {
              webhookSubscriptions(first: 100, after: $after) {
                pageInfo { hasNextPage endCursor }
                nodes { id uri }
              }
            }",
            json!({ "after": after }),
        )
        .await?;
        let (nodes, next) = page(&data["webhookSubscriptions"]);
        ids.extend(
            nodes
                .iter()
                .filter(|n| n["uri"] == uri)
                .filter_map(|n| n["id"].as_str().map(str::to_string)),
        );
        match next {
            Some(cursor) => after = Some(cursor),
            None => return Ok(ids),
        }
    }
}

/// `deleteShopifyWebhookSubscription`: one already gone counts as removed.
pub async fn delete_webhook_subscription(credentials: &Credentials, id: &str) -> ApiResult<()> {
    let data = graphql(
        credentials,
        "mutation WebhookDelete($id: ID!) {
          webhookSubscriptionDelete(id: $id) {
            deletedWebhookSubscriptionId
            userErrors { field message }
          }
        }",
        json!({ "id": id }),
    )
    .await?;
    let errors = &data["webhookSubscriptionDelete"]["userErrors"];
    let gone = errors.as_array().is_none_or(|list| {
        list.iter().all(|e| {
            let m = e["message"].as_str().unwrap_or_default().to_lowercase();
            m.contains("not exist") || m.contains("not found")
        })
    });
    if gone {
        return Ok(());
    }
    assert_no_user_errors(errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gids_and_pages() {
        assert_eq!(gid("Product", 12), "gid://shopify/Product/12");
        let connection = json!({ "pageInfo": { "hasNextPage": true, "endCursor": "c1" }, "nodes": [{ "id": "a" }] });
        assert_eq!(page(&connection).1.as_deref(), Some("c1"));
        let last = json!({ "pageInfo": { "hasNextPage": false, "endCursor": "c2" }, "nodes": [] });
        assert_eq!(page(&last).1, None);
    }

    #[test]
    fn user_errors() {
        assert!(assert_no_user_errors(&json!([])).is_ok());
        let error =
            assert_no_user_errors(&json!([{ "message": "a" }, { "message": "b" }])).unwrap_err();
        assert_eq!(error.message, "a; b");
        assert!(error.user_error);
    }
}
