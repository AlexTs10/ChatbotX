//! `features/webshop/api/public.ts`: `webshop.getLink` (`createWebshopLink`).
//! `carts.get` is not ported (the cart lives with the public shop pages).

use chatbotx_business::contact::service::resolve_id_by_identifier;
use chatbotx_business::{BusinessError, tx, vendor};
use chatbotx_core::encryption::{SHOP_PREVIEW_TOKEN_TTL_MS, SHOP_TOKEN_TTL_MS, ShopCustomerClaims};
use chatbotx_core::utils::web_url::join_app_url;
use chatbotx_db::repo::{product, product_category};
use serde_json::json;

use super::input::{Input, Str, key};
use super::{ApiState, Call, Principal, Reply, business_error, json};
use crate::orpc::OrpcError;

/// The routes of this module.
pub const ROUTES: &[&str] = &["webshop.getLink"];

/// `buildWebshopUrl`: `token` may be `{{shop_token}}`, kept as is.
pub fn build_url(
    app_url: &str,
    workspace_id: i64,
    token: &str,
    page: Option<&str>,
    product_id: Option<i64>,
    vendor_id: Option<i64>,
    category_id: Option<i64>,
) -> String {
    let page = match (product_id, page) {
        (Some(id), _) => format!("/products/{id}"),
        (None, Some(page)) => format!("/{page}"),
        (None, None) => String::new(),
    };
    let mut query = vec![format!("token={token}")];
    if let Some(id) = vendor_id {
        query.push(format!("vendor={id}"));
    }
    if let Some(id) = category_id {
        query.push(format!("category={id}"));
    }
    join_app_url(
        app_url,
        &format!("/shop/{workspace_id}{page}?{}", query.join("&")),
    )
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, call: Call<'_>, principal: &Principal) -> Reply {
    let ws = call.workspace_id;
    let mut input = Input::from_query(call.params, call.query);
    let contact = input.opt_string(
        "contact",
        Str {
            min: Some(1),
            ..Str::default()
        },
    );
    let page = input
        .get("page")
        .cloned()
        .and_then(|v| input.enum_at(&key("page"), Some(&v), &["cart", "categories"], true));
    let mut opt_id = |name: &str| {
        if input.has(name) {
            input.id(name)
        } else {
            None
        }
    };
    let product_id = opt_id("productId");
    let vendor_id = opt_id("vendorId");
    let category_id = opt_id("categoryId");
    input.finish()?;
    if contact.is_some() && principal.read_only {
        return Err(OrpcError::new(
            "FORBIDDEN",
            None,
            Some("Read-only token cannot create a shop link for a contact"),
        ));
    }
    let contact_id = match contact {
        Some(identifier) => Some(
            resolve_id_by_identifier(&state.db, ws, &identifier)
                .await
                .map_err(|e| business_error(e, super::FINDING))?,
        ),
        None => None,
    };
    tx::read(&state.db, move |conn| {
        if let Some(id) = vendor_id {
            vendor::assert_belongs_to_workspace(conn, ws, id)?;
        }
        if let Some(id) = category_id
            && product_category::find(conn, ws, id)?.is_none()
        {
            return Err(BusinessError::not_found("Product category does not exist."));
        }
        if let Some(id) = product_id
            && product::find(conn, ws, id)?.is_none()
        {
            return Err(BusinessError::not_found("Product does not exist."));
        }
        Ok(())
    })
    .await
    .map_err(|e| business_error(e, super::FINDING))?;
    let enc = state
        .encryption
        .as_deref()
        .ok_or_else(|| super::internal_error("ENCRYPTION_KEY is not set"))?;
    let preview = contact_id.is_none();
    let token = match contact_id {
        Some(contact_id) => enc.sign_shop_token(
            &ShopCustomerClaims {
                workspace_id: ws.to_string(),
                contact_id: contact_id.to_string(),
                contact_inbox_id: None,
                conversation_id: None,
            },
            SHOP_TOKEN_TTL_MS,
        ),
        None => enc.sign_shop_preview_token(&ws.to_string()),
    }
    .map_err(|e| super::internal_error(&e.to_string()))?;
    // ponytail: the app URL is the platform's; white-label domains are not ported.
    let url = |token: &str| {
        build_url(
            &state.app_url,
            ws,
            token,
            page.as_deref(),
            product_id,
            vendor_id,
            category_id,
        )
    };
    let ttl = if preview {
        SHOP_PREVIEW_TOKEN_TTL_MS
    } else {
        SHOP_TOKEN_TTL_MS
    };
    Ok(json(
        200,
        json!({
            "url": url(&token),
            "buttonUrl": url("{{shop_token}}"),
            "preview": preview,
            "expiresInSeconds": ttl / 1000,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::build_url;

    #[test]
    fn shop_urls() {
        assert_eq!(
            build_url("https://app.test", 7, "t", None, None, None, None),
            "https://app.test/shop/7?token=t"
        );
        assert_eq!(
            build_url(
                "https://app.test",
                7,
                "{{shop_token}}",
                Some("cart"),
                None,
                Some(3),
                Some(4)
            ),
            "https://app.test/shop/7/cart?token={{shop_token}}&vendor=3&category=4"
        );
        assert_eq!(
            build_url(
                "https://app.test",
                7,
                "t",
                Some("cart"),
                Some(9),
                None,
                None
            ),
            "https://app.test/shop/7/products/9?token=t"
        );
    }
}
