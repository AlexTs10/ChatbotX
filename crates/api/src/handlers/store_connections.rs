//! `features/{shopify,woocommerce}/api/public.ts`: get, connect, sync and
//! disconnect the store source. Every route is a workspace token with the `ecommerce` scope.

use chatbotx_business::shopify::{self, ConnectAuth};
use chatbotx_business::woocommerce;
use serde_json::Value;

use super::input::{Input, Str, key};
use super::{ApiState, Call, ErrorMap, FINDING, Reply, business_error, json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "shopify.getStatus",
    "shopify.connect",
    "shopify.sync",
    "shopify.disconnect",
    "woocommerce.getStatus",
    "woocommerce.connect",
    "woocommerce.sync",
    "woocommerce.disconnect",
];

/// `possibleErrorsOnShopify`.
const SHOPIFY: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("validationError", 400),
    ("shopifyNotConnected", 400),
    ("shopifyCredentialsInvalid", 400),
    ("shopifySyncInProgress", 409),
];

/// `possibleErrorsOnWooCommerce`.
const WOOCOMMERCE: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("validationError", 400),
    ("woocommerceNotConnected", 400),
    ("woocommerceCredentialsInvalid", 400),
    ("woocommerceSyncInProgress", 409),
];

const SCOPES: &[&str] = &["all", "products", "orders"];

fn shopify_ctx(state: &ApiState) -> shopify::Ctx<'_> {
    shopify::Ctx {
        db: &state.db,
        jobs: state.jobs.as_deref(),
        app_url: &state.app_url,
        audit: &state.audit,
    }
}

/// `syncShopifyRequest`: `scope` defaults to `all`.
fn scope(input: &mut Input) -> String {
    let value = input.get("scope").cloned();
    match value {
        None => "all".into(),
        Some(v) => input
            .enum_at(&key("scope"), Some(&v), SCOPES, true)
            .unwrap_or_default(),
    }
}

/// `connectShopifyRequest` (discriminated by `authKind`).
fn connect_input(input: &mut Input) -> Option<(String, ConnectAuth)> {
    let kind = input.get("authKind").cloned();
    let kind = input.enum_at(
        &key("authKind"),
        kind.as_ref(),
        &["token", "clientCredentials"],
        true,
    );
    let domain = input.string("shopDomain", Str::trimmed(1, 255));
    let secret = Str::trimmed(1, 512);
    let auth = match kind.as_deref() {
        Some("token") => {
            let access_token = input.string("accessToken", secret);
            let api_secret = input.string("apiSecret", secret);
            ConnectAuth::Token {
                access_token: access_token?,
                api_secret: api_secret?,
            }
        }
        Some(_) => {
            let client_id = input.string("clientId", secret);
            let client_secret = input.string("clientSecret", secret);
            ConnectAuth::ClientCredentials {
                client_id: client_id?,
                client_secret: client_secret?,
            }
        }
        None => return None,
    };
    Some((domain?, auth))
}

fn invalid_input() -> crate::orpc::OrpcError {
    crate::orpc::OrpcError::new(
        "invalidRequestData",
        Some(422),
        Some("Input validation failed"),
    )
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    if let Some(action) = route_id.strip_prefix("woocommerce.") {
        return handle_woocommerce(state, action, call).await;
    }
    let ws = call.workspace_id;
    let ctx = shopify_ctx(state);
    let view: Value = match route_id {
        "shopify.getStatus" => shopify::get_status(&state.db, &state.app_url, ws)
            .await
            .map_err(|e| business_error(e, FINDING))?,
        "shopify.connect" => {
            let mut input = Input::from_body(call.params, call.body);
            let parsed = connect_input(&mut input);
            input.finish()?;
            let (domain, auth) = parsed.ok_or_else(invalid_input)?;
            shopify::connect(ctx, ws, &domain, auth)
                .await
                .map_err(|e| business_error(e, SHOPIFY))?
        }
        "shopify.sync" => {
            let mut input = Input::from_body(call.params, call.body);
            let scope = scope(&mut input);
            input.finish()?;
            shopify::request_sync(ctx, ws, &scope)
                .await
                .map_err(|e| business_error(e, SHOPIFY))?
        }
        _ => {
            shopify::disconnect(ctx, ws)
                .await
                .map_err(|e| business_error(e, SHOPIFY))?;
            return Ok(no_content());
        }
    };
    Ok(json(200, view))
}

async fn handle_woocommerce(state: &ApiState, action: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    let ctx = shopify_ctx(state);
    let view: Value = match action {
        "getStatus" => woocommerce::get_status(&state.db, &state.app_url, ws)
            .await
            .map_err(|e| business_error(e, FINDING))?,
        "connect" => {
            // `connectWooCommerceRequest`.
            let mut input = Input::from_body(call.params, call.body);
            let site_url = input.string("siteUrl", Str::trimmed(1, 2048));
            let key = input.string("consumerKey", Str::trimmed(1, 512));
            let secret = input.string("consumerSecret", Str::trimmed(1, 512));
            input.finish()?;
            let (Some(site_url), Some(key), Some(secret)) = (site_url, key, secret) else {
                return Err(invalid_input());
            };
            woocommerce::connect(ctx, ws, &site_url, &key, &secret)
                .await
                .map_err(|e| business_error(e, WOOCOMMERCE))?
        }
        "sync" => {
            let mut input = Input::from_body(call.params, call.body);
            let scope = scope(&mut input);
            input.finish()?;
            woocommerce::request_sync(ctx, ws, &scope)
                .await
                .map_err(|e| business_error(e, WOOCOMMERCE))?
        }
        _ => {
            woocommerce::disconnect(ctx, ws)
                .await
                .map_err(|e| business_error(e, WOOCOMMERCE))?;
            return Ok(no_content());
        }
    };
    Ok(json(200, view))
}
