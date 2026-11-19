//! `webshopService.checkoutOnExternalStore`, `shopify/checkout.ts` and
//! `woocommerce/checkout.ts`: a shop mirroring Shopify or WooCommerce
//! checks out on that store.
//! - `webCheckout` answers with a link to the store's own page (Shopify's
//!   cart permalink, or the payment page of a pending WooCommerce order);
//!   the cart stays until the store reports the order.
//! - `inApp` places a cash-on-delivery order on the store now, mirrors it
//!   here and takes what was ordered out of the cart.

use chatbotx_db::repo::external_store as external_repo;
use chatbotx_db::repo::models::{IntegrationShopify, IntegrationWoocommerce, Order};
use chatbotx_db::repo::{self, store_integration};
use serde_json::{Value, json};

use super::{
    CheckoutForm, Shopper, cart_lines, find_cart, subtract_ordered_lines, trimmed, upsert_lines,
};
use crate::errors::{BusinessError, Result};
use crate::external_store::{
    EXTERNAL_CONTACT_ATTRIBUTE, sign_contact_reference, split_customer_name,
};
use crate::order::{self, Deps, PlaceItem};
use crate::{shopify, tx, woocommerce};

struct Customer {
    name: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    address: Option<Value>,
    note: Option<String>,
}

/// The discount the cart applied, as the store takes it.
struct Discount {
    code: String,
    discount_type: String,
    amount: f64,
    /// For a percentage code: its percent.
    percent: Option<f64>,
}

struct Input<'a> {
    shopper: Shopper,
    lines: &'a [PlaceItem],
    customer: Customer,
    discount: Option<Discount>,
    settings: &'a Value,
    delivery: bool,
}

impl Input<'_> {
    fn reference(&self) -> Result<String> {
        let s = &self.shopper;
        sign_contact_reference(
            s.workspace_id,
            s.contact_id,
            s.contact_inbox_id,
            s.conversation_id,
        )
    }

    fn delivery_cost(&self) -> f64 {
        self.settings["shipping"]["deliveryCost"]
            .as_f64()
            .unwrap_or(0.0)
    }

    /// The note: the customer's, then the extra lines, `None` when empty.
    fn note(&self, extra: &[Option<String>]) -> Option<String> {
        let parts: Vec<&str> = std::iter::once(&self.customer.note)
            .chain(extra)
            .filter_map(|p| p.as_deref().filter(|p| !p.is_empty()))
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n"))
    }

    fn pick_up_note(&self) -> Option<String> {
        (!self.delivery).then(|| "Pick up in the store".to_string())
    }
}

fn internal(error: impl std::fmt::Display) -> BusinessError {
    BusinessError::Repo(repo::Error::msg(error.to_string()))
}

/// `checkoutOnExternalStore`: `{ orderId, total, currency, paymentUrl }`.
pub(super) async fn checkout(
    deps: Deps<'_>,
    shopper: Shopper,
    source: &str,
    settings: &Value,
    lines: &[PlaceItem],
    discount_code: Option<String>,
    form: CheckoutForm,
) -> Result<Value> {
    let ws = shopper.workspace_id;
    let offers_codes = settings["checkout"]["externalAutoApplyDiscounts"] == true;
    let quote = order::quote(
        deps.db,
        ws,
        lines.to_vec(),
        form.shipping_type.clone(),
        Some(shopper.contact_id),
        discount_code.filter(|_| offers_codes),
        offers_codes,
    )
    .await?;
    let discount = quote
        .draft
        .discount
        .filter(|_| offers_codes)
        .map(|d| Discount {
            percent: (d.discount_type == "percentage").then_some(d.discount_value),
            code: d.code,
            discount_type: d.discount_type,
            amount: d.amount,
        });
    let input = Input {
        shopper,
        lines,
        customer: Customer {
            name: trimmed(form.customer_name.as_ref()),
            email: trimmed(form.customer_email.as_ref()),
            phone: trimmed(form.customer_phone.as_ref()),
            address: form.shipping_address.clone(),
            note: trimmed(form.note.as_ref()),
        },
        discount,
        settings,
        delivery: quote.draft.shipping_type.as_deref() == Some("delivery"),
    };
    let shopify = source == shopify::SOURCE;
    if settings["checkout"]["externalCheckoutMethod"] == "webCheckout" {
        let url = if shopify {
            shopify_web_checkout_url(deps, &input).await?
        } else {
            woocommerce_web_checkout_url(deps, &input).await?
        };
        return Ok(json!({ "orderId": null, "total": 0, "currency": "", "paymentUrl": url }));
    }
    let placed = if shopify {
        shopify_in_app_order(deps, &input).await?
    } else {
        woocommerce_in_app_order(deps, &input).await?
    };
    let ordered = lines.to_vec();
    tx::write(deps.db, move |tx| {
        let conn: &chatbotx_db::rusqlite::Connection = tx;
        let current = cart_lines(find_cart(conn, ws, shopper.contact_id)?.as_ref());
        match subtract_ordered_lines(&current, &ordered) {
            Some(left) => upsert_lines(conn, &shopper, &left),
            None => Ok(()),
        }
    })
    .await?;
    Ok(json!({
        "orderId": placed.id.to_string(),
        "total": order::major(placed.total),
        "currency": placed.currency,
        "paymentUrl": null,
    }))
}

/// `findCheckoutVariantIds` for the cart's lines.
async fn variant_ids(
    deps: Deps<'_>,
    ws: i64,
    source: &'static str,
    lines: &[PlaceItem],
) -> Result<external_repo::CheckoutVariantIds> {
    let products: Vec<i64> = lines.iter().map(|l| l.product_id).collect();
    let variants: Vec<i64> = lines.iter().filter_map(|l| l.variant_id).collect();
    tx::read(deps.db, move |conn| {
        Ok(external_repo::find_checkout_variant_ids(
            conn, ws, source, &products, &variants,
        )?)
    })
    .await
}

fn lookup(map: &[(i64, Option<String>)], id: i64) -> Option<&str> {
    map.iter()
        .find(|(k, _)| *k == id)
        .and_then(|(_, v)| v.as_deref())
}

// ---- Shopify -----------------------------------------------------------------

fn shopify_rejected(detail: &str) -> BusinessError {
    BusinessError::exception(
        format!("Shopify could not place the order: {detail}"),
        "shopifyOrderRejected",
        422,
    )
    .with_data(json!({ "detail": detail }))
}

fn shopify_variant_missing() -> BusinessError {
    BusinessError::exception(
        "A product in the cart is no longer sold on Shopify. Remove it and try again.",
        "shopifyVariantMissing",
        409,
    )
}

async fn shopify_row(deps: Deps<'_>, ws: i64) -> Result<IntegrationShopify> {
    tx::read(deps.db, move |conn| {
        Ok(store_integration::find_shopify(conn, ws)?)
    })
    .await?
    .ok_or_else(shopify::not_connected)
}

/// `resolveVariantLines`: Shopify variant GIDs and quantities.
async fn shopify_lines(deps: Deps<'_>, input: &Input<'_>) -> Result<Vec<(String, i64)>> {
    let ids = variant_ids(
        deps,
        input.shopper.workspace_id,
        shopify::SOURCE,
        input.lines,
    )
    .await?;
    input
        .lines
        .iter()
        .map(|line| {
            let variant = match line.variant_id {
                Some(id) => lookup(&ids.by_variant, id),
                None => lookup(&ids.by_product, line.product_id),
            };
            variant
                .map(|v| (v.to_string(), line.quantity))
                .ok_or_else(shopify_variant_missing)
        })
        .collect()
}

/// `toShopifyAddress`.
struct ShopifyAddress {
    first_name: Option<String>,
    last_name: String,
    address1: Option<String>,
    address2: Option<String>,
    city: Option<String>,
    province: Option<String>,
    zip: Option<String>,
    country_code: Option<String>,
    phone: Option<String>,
}

fn shopify_address(customer: &Customer) -> ShopifyAddress {
    let field = |key: &str| {
        customer
            .address
            .as_ref()
            .and_then(|a| a[key].as_str())
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let (first_name, last_name) = split_customer_name(customer.name.as_deref());
    let (line1, line2) = (field("line1"), field("line2"));
    ShopifyAddress {
        first_name,
        last_name,
        address2: line1.as_ref().and(line2.clone()),
        address1: line1.or(line2),
        city: field("city"),
        province: field("state"),
        zip: field("zip"),
        country_code: field("country"),
        phone: customer.phone.clone(),
    }
}

/// `fromShopifyGid`: the number at the end of a GID.
fn from_gid(gid: &str) -> &str {
    let tail = gid.rsplit('/').next().unwrap_or(gid);
    let number = tail.split('?').next().unwrap_or(tail);
    if gid.contains('/') && !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) {
        number
    } else {
        gid
    }
}

/// `buildShopifyCartPermalink`.
fn cart_permalink(
    shop_domain: &str,
    lines: &[(String, i64)],
    email: Option<&str>,
    address: &ShopifyAddress,
    discount_code: Option<&str>,
    reference: &str,
) -> Result<String> {
    let path = lines
        .iter()
        .filter(|(_, quantity)| *quantity > 0)
        .map(|(gid, quantity)| format!("{}:{quantity}", from_gid(gid)))
        .collect::<Vec<_>>()
        .join(",");
    let mut url =
        url::Url::parse(&format!("https://{shop_domain}/cart/{path}")).map_err(internal)?;
    {
        let mut params = url.query_pairs_mut();
        if let Some(email) = email {
            params.append_pair("checkout[email]", email);
        }
        for (field, value) in [
            ("first_name", &address.first_name),
            ("last_name", &Some(address.last_name.clone())),
            ("address1", &address.address1),
            ("address2", &address.address2),
            ("city", &address.city),
            ("province", &address.province),
            ("zip", &address.zip),
            ("country", &address.country_code),
            ("phone", &address.phone),
        ] {
            if let Some(value) = value.as_deref().filter(|v| !v.is_empty()) {
                params.append_pair(&format!("checkout[shipping_address][{field}]"), value);
            }
        }
        if let Some(code) = discount_code {
            params.append_pair("discount", code);
        }
        params.append_pair(
            &format!("attributes[{EXTERNAL_CONTACT_ATTRIBUTE}]"),
            reference,
        );
    }
    Ok(url.to_string())
}

/// `buildShopifyWebCheckoutUrl`: the order comes back through the
/// `orders/create` webhook, tied to the contact by the signed reference.
async fn shopify_web_checkout_url(deps: Deps<'_>, input: &Input<'_>) -> Result<String> {
    let row = shopify_row(deps, input.shopper.workspace_id).await?;
    let lines = shopify_lines(deps, input).await?;
    cart_permalink(
        &row.shop_domain,
        &lines,
        input.customer.email.as_deref(),
        &shopify_address(&input.customer),
        input.discount.as_ref().map(|d| d.code.as_str()),
        &input.reference()?,
    )
}

/// `toShopifyDiscount` + `discountCodeInput`.
fn shopify_discount(discount: &Discount, currency: &str) -> Value {
    let money =
        |amount: f64| json!({ "shopMoney": { "amount": amount, "currencyCode": currency } });
    match (discount.discount_type.as_str(), discount.percent) {
        ("percentage", Some(percent)) => json!({
            "itemPercentageDiscountCode": { "code": discount.code, "percentage": percent }
        }),
        ("freeShipping", _) => json!({
            "freeShippingDiscountCode": { "code": discount.code, "amountSet": money(discount.amount) }
        }),
        _ => json!({
            "itemFixedDiscountCode": { "code": discount.code, "amountSet": money(discount.amount) }
        }),
    }
}

/// `createShopifyOrder`'s `OrderCreateOrderInput`.
fn shopify_order_input(
    input: &Input<'_>,
    currency: &str,
    lines: &[(String, i64)],
    reference: &str,
) -> Value {
    let address = shopify_address(&input.customer);
    // Shopify takes a province code; a free-text state goes in the note.
    let long_province = address.province.as_ref().filter(|p| p.chars().count() > 3);
    let mut order = json!({
        "currency": currency,
        "financialStatus": "PENDING",
        "lineItems": lines.iter().map(|(variant_id, quantity)| json!({
            "variantId": variant_id,
            "quantity": quantity,
        })).collect::<Vec<_>>(),
        "customAttributes": [{ "key": EXTERNAL_CONTACT_ATTRIBUTE, "value": reference }],
    });
    let set = |order: &mut Value, key: &str, value: Value| {
        order[key] = value;
    };
    if let Some(email) = &input.customer.email {
        set(&mut order, "email", json!(email));
    }
    if let Some(phone) = &input.customer.phone {
        set(&mut order, "phone", json!(phone));
    }
    if input.delivery {
        let province_code = address
            .province
            .as_ref()
            .filter(|p| p.chars().count() <= 3)
            .map(|p| p.to_uppercase());
        let shipping: serde_json::Map<String, Value> = [
            ("firstName", address.first_name.clone()),
            ("lastName", Some(address.last_name.clone())),
            ("address1", address.address1.clone()),
            ("address2", address.address2.clone()),
            ("city", address.city.clone()),
            ("provinceCode", province_code),
            ("zip", address.zip.clone()),
            ("countryCode", address.country_code.clone()),
            ("phone", address.phone.clone()),
        ]
        .into_iter()
        .filter_map(|(k, v)| {
            v.filter(|v| !v.is_empty())
                .map(|v| (k.to_string(), json!(v)))
        })
        .collect();
        set(&mut order, "shippingAddress", Value::Object(shipping));
        set(
            &mut order,
            "shippingLines",
            json!([{
                "title": "Delivery",
                "priceSet": { "shopMoney": { "amount": input.delivery_cost(), "currencyCode": currency } },
            }]),
        );
    }
    if let Some(discount) = &input.discount {
        set(
            &mut order,
            "discountCode",
            shopify_discount(discount, currency),
        );
    }
    let note = input.note(&[
        long_province.map(|p| format!("State / province: {p}")),
        input.pick_up_note(),
    ]);
    if let Some(note) = note {
        set(&mut order, "note", json!(note));
    }
    order
}

/// `placeShopifyInAppOrder`: the order is placed on Shopify (cash on
/// delivery, stock taken by Shopify) and mirrored here at once; the
/// `orders/create` webhook that follows only updates it.
async fn shopify_in_app_order(deps: Deps<'_>, input: &Input<'_>) -> Result<Order> {
    let row = shopify_row(deps, input.shopper.workspace_id).await?;
    let lines = shopify_lines(deps, input).await?;
    let order = shopify_order_input(input, &row.currency, &lines, &input.reference()?);
    let credentials = shopify::resolve_credentials(deps.db, &row)
        .await
        .map_err(|e| match e {
            shopify::sync::SyncError::Business(e) => e,
            shopify::sync::SyncError::Api(e) => internal(e.message),
        })?;
    let query = format!(
        "mutation OrderCreate($order: OrderCreateOrderInput!, $options: OrderCreateOptionsInput) {{
          orderCreate(order: $order, options: $options) {{
            order {{ {} }}
            userErrors {{ field message }}
          }}
        }}",
        shopify::client::ORDER_FIELDS
    );
    let rejected = |e: shopify::client::ApiError| {
        if e.user_error {
            shopify_rejected(&e.message)
        } else {
            internal(e.message)
        }
    };
    let data = shopify::client::graphql(
        &credentials,
        &query,
        json!({ "order": order, "options": { "inventoryBehaviour": "DECREMENT_OBEYING_POLICY" } }),
    )
    .await
    .map_err(rejected)?;
    shopify::client::assert_no_user_errors(&data["orderCreate"]["userErrors"]).map_err(rejected)?;
    let created = &data["orderCreate"]["order"];
    if !created.is_object() {
        return Err(internal("Shopify created no order."));
    }
    let (saved, _) = shopify::sync::mirror_order(deps, &row, created, true).await?;
    Ok(saved)
}

// ---- WooCommerce -------------------------------------------------------------

fn woocommerce_rejected(detail: &str) -> BusinessError {
    BusinessError::exception(
        format!("WooCommerce could not place the order: {detail}"),
        "woocommerceOrderRejected",
        422,
    )
    .with_data(json!({ "detail": detail }))
}

fn woocommerce_product_missing() -> BusinessError {
    BusinessError::exception(
        "A product in the cart is no longer sold on WooCommerce. Remove it and try again.",
        "woocommerceProductMissing",
        409,
    )
}

/// `resolveOrderLines`: the store's product and variation ids.
async fn woocommerce_lines(deps: Deps<'_>, input: &Input<'_>) -> Result<Vec<Value>> {
    let ids = variant_ids(
        deps,
        input.shopper.workspace_id,
        woocommerce::SOURCE,
        input.lines,
    )
    .await?;
    input
        .lines
        .iter()
        .map(|line| {
            let product_id = lookup(&ids.product_external_ids, line.product_id)
                .and_then(|id| id.parse::<i64>().ok())
                .filter(|id| *id != 0);
            let variation = line.variant_id.and_then(|id| lookup(&ids.by_variant, id));
            let (Some(product_id), true) =
                (product_id, line.variant_id.is_none() || variation.is_some())
            else {
                return Err(woocommerce_product_missing());
            };
            let mut item = json!({ "product_id": product_id, "quantity": line.quantity });
            if let Some(variation) = variation {
                if let Ok(id) = woocommerce::mappers::variation_id_of(variation).parse::<i64>()
                    && id != 0
                {
                    item["variation_id"] = json!(id);
                }
                let attributes = woocommerce::mappers::any_attributes_of(variation);
                if !attributes.is_empty() {
                    item["meta_data"] = attributes
                        .into_iter()
                        .map(|(key, value)| json!({ "key": key, "value": value }))
                        .collect();
                }
            }
            Ok(item)
        })
        .collect()
}

/// `toAddress`.
fn woocommerce_address(customer: &Customer, with_email: bool) -> Value {
    let field = |key: &str| {
        customer
            .address
            .as_ref()
            .and_then(|a| a[key].as_str())
            .unwrap_or_default()
            .to_string()
    };
    let (first_name, last_name) = split_customer_name(customer.name.as_deref());
    let mut address = json!({
        "first_name": first_name.unwrap_or_default(),
        "last_name": last_name,
        "address_1": field("line1"),
        "address_2": field("line2"),
        "city": field("city"),
        "state": field("state"),
        "postcode": field("zip"),
        "country": field("country"),
        "phone": customer.phone.clone().unwrap_or_default(),
    });
    if with_email {
        address["email"] = json!(customer.email.clone().unwrap_or_default());
    }
    address
}

/// `placeOnStore`: a `pending` (web checkout) or `processing` (cash on
/// delivery) order on the store; a 4xx refusal becomes ours.
async fn woocommerce_place(
    deps: Deps<'_>,
    input: &Input<'_>,
    status: &str,
    meta: &[(&str, &str)],
) -> Result<(IntegrationWoocommerce, Value)> {
    let ws = input.shopper.workspace_id;
    let row = woocommerce::find(deps.db, ws)
        .await?
        .ok_or_else(woocommerce::not_connected)?;
    let line_items = woocommerce_lines(deps, input).await?;
    let cash_on_delivery = status == "processing";
    let mut meta_data =
        vec![json!({ "key": EXTERNAL_CONTACT_ATTRIBUTE, "value": input.reference()? })];
    meta_data.extend(
        meta.iter()
            .map(|(key, value)| json!({ "key": key, "value": value })),
    );
    let mut body = json!({
        "status": status,
        "set_paid": false,
        "billing": woocommerce_address(&input.customer, true),
        "line_items": line_items,
        "shipping_lines": if input.delivery {
            json!([{
                "method_id": "flat_rate",
                "method_title": "Delivery",
                "total": format!("{:.2}", input.delivery_cost()),
            }])
        } else {
            json!([])
        },
        "coupon_lines": input.discount.as_ref().map_or_else(|| json!([]), |d| json!([{ "code": d.code }])),
        "customer_note": input.note(&[input.pick_up_note()]).unwrap_or_default(),
        "meta_data": meta_data,
    });
    if cash_on_delivery {
        body["payment_method"] = json!("cod");
        body["payment_method_title"] = json!("Cash on delivery");
    }
    if input.delivery {
        body["shipping"] = woocommerce_address(&input.customer, false);
    }
    let credentials = woocommerce::resolve_credentials(&row)?;
    let (order, _) = woocommerce::client::request(
        &credentials,
        reqwest::Method::POST,
        "/orders",
        &[],
        Some(&body),
    )
    .await
    .map_err(|e| {
        if (400..500).contains(&e.status) && !e.unauthorized() {
            woocommerce_rejected(&e.message)
        } else {
            internal(e.message)
        }
    })?;
    Ok((row, order))
}

/// `paymentUrlOf`: the customer's payment page of an unpaid order.
fn payment_url_of(site_url: &str, order: &Value) -> String {
    if let Some(url) = order["payment_url"].as_str() {
        return url.to_string();
    }
    let id = match &order["id"] {
        Value::Number(n) => n.to_string(),
        other => other.as_str().unwrap_or_default().to_string(),
    };
    // ponytail: form encoding with `%20` for spaces stands in for
    // `encodeURIComponent`; an order key is `wc_order_` + alphanumerics.
    let key: String = url::form_urlencoded::byte_serialize(
        order["order_key"].as_str().unwrap_or_default().as_bytes(),
    )
    .collect::<String>()
    .replace('+', "%20");
    format!("{site_url}/checkout/order-pay/{id}/?pay_for_order=true&key={key}")
}

/// `buildWooCommerceWebCheckoutUrl`: an unpaid order with the cart; the
/// customer pays on the store, and the order is mirrored once paid.
async fn woocommerce_web_checkout_url(deps: Deps<'_>, input: &Input<'_>) -> Result<String> {
    let meta = [(
        woocommerce::sync::CHECKOUT_META,
        woocommerce::sync::WEB_CHECKOUT,
    )];
    let (row, order) = woocommerce_place(deps, input, "pending", &meta).await?;
    Ok(payment_url_of(&row.site_url, &order))
}

/// `placeWooCommerceInAppOrder`: a cash-on-delivery order on the store,
/// mirrored here at once; the `order.created` webhook only updates it.
async fn woocommerce_in_app_order(deps: Deps<'_>, input: &Input<'_>) -> Result<Order> {
    let (row, order) = woocommerce_place(deps, input, "processing", &[]).await?;
    let (saved, _) = woocommerce::sync::mirror_order(deps, &row, &order, true)
        .await?
        .ok_or_else(|| internal("The WooCommerce order could not be mirrored"))?;
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn customer(state: &str) -> Customer {
        Customer {
            name: Some("Nguyen Van A".into()),
            email: Some("a@example.com".into()),
            phone: None,
            address: Some(json!({
                "line1": "123 Nguyen Hue, Quan 1",
                "line2": "",
                "city": "Ho Chi Minh",
                "state": state,
                "country": "VN",
            })),
            note: Some("Ring twice".into()),
        }
    }

    /// Port of: integrations/shopify/__tests__/lib.test.ts "buildShopifyCartPermalink > puts lines, customer, discount and attributes in the URL"
    #[test]
    fn the_permalink_carries_lines_customer_discount_and_reference() {
        let url = cart_permalink(
            "botxshop.myshopify.com",
            &[
                ("gid://shopify/ProductVariant/11".into(), 2),
                ("gid://shopify/ProductVariant/12".into(), 0),
                ("13".into(), 1),
            ],
            Some("a@example.com"),
            &shopify_address(&customer("")),
            Some("FASHION10"),
            "abc.def",
        )
        .unwrap();
        let url = url::Url::parse(&url).unwrap();
        assert_eq!(
            url.origin().ascii_serialization(),
            "https://botxshop.myshopify.com"
        );
        assert_eq!(url.path(), "/cart/11:2,13:1");
        let get = |key: &str| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
        };
        assert_eq!(get("checkout[email]").as_deref(), Some("a@example.com"));
        assert_eq!(
            get("checkout[shipping_address][address1]").as_deref(),
            Some("123 Nguyen Hue, Quan 1")
        );
        assert_eq!(
            get("checkout[shipping_address][first_name]").as_deref(),
            Some("Van A")
        );
        assert_eq!(
            get("checkout[shipping_address][last_name]").as_deref(),
            Some("Nguyen")
        );
        assert_eq!(get("checkout[shipping_address][zip]"), None);
        assert_eq!(get("checkout[shipping_address][address2]"), None);
        assert_eq!(get("discount").as_deref(), Some("FASHION10"));
        assert_eq!(get("attributes[cbx_ref]").as_deref(), Some("abc.def"));
    }

    #[test]
    fn a_long_state_goes_in_the_note_and_pick_up_is_noted() {
        let settings = json!({ "shipping": { "deliveryCost": 2.5 } });
        let lines = [("gid://shopify/ProductVariant/11".to_string(), 1)];
        let input = |state: &str, delivery: bool| Input {
            shopper: Shopper {
                workspace_id: 1,
                contact_id: 2,
                contact_inbox_id: None,
                conversation_id: None,
            },
            lines: &[],
            customer: customer(state),
            discount: Some(Discount {
                code: "TEN".into(),
                discount_type: "percentage".into(),
                amount: 1.0,
                percent: Some(10.0),
            }),
            settings: &settings,
            delivery,
        };
        let order = shopify_order_input(&input("Ho Chi Minh City", true), "VND", &lines, "ref");
        assert_eq!(order["shippingAddress"].get("provinceCode"), None);
        assert_eq!(
            order["note"],
            "Ring twice\nState / province: Ho Chi Minh City"
        );
        assert_eq!(
            order["shippingLines"][0]["priceSet"]["shopMoney"]["amount"],
            2.5
        );
        assert_eq!(
            order["discountCode"]["itemPercentageDiscountCode"]["percentage"],
            10.0
        );
        assert_eq!(
            order["customAttributes"][0]["key"],
            EXTERNAL_CONTACT_ATTRIBUTE
        );

        let order = shopify_order_input(&input("ca", false), "VND", &lines, "ref");
        assert_eq!(order.get("shippingAddress"), None);
        assert_eq!(order["note"], "Ring twice\nPick up in the store");
        let order = shopify_order_input(&input("ca", true), "VND", &lines, "ref");
        assert_eq!(order["shippingAddress"]["provinceCode"], "CA");
        assert_eq!(order["note"], "Ring twice");
    }

    #[test]
    fn the_woocommerce_payment_page_falls_back_to_the_order_key() {
        assert_eq!(
            payment_url_of(
                "https://shop.test",
                &json!({ "id": 7, "order_key": "wc_order_a b" })
            ),
            "https://shop.test/checkout/order-pay/7/?pay_for_order=true&key=wc_order_a%20b"
        );
        assert_eq!(
            payment_url_of(
                "https://shop.test",
                &json!({ "id": 7, "payment_url": "https://pay" })
            ),
            "https://pay"
        );
    }
}
