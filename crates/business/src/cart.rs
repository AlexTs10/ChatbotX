//! `webshopService.getCart`: a contact's web shop cart, priced as an order
//! would be now (`orderService.quote`), with the codes it can offer.

use chatbotx_db::repo::find_one;
use chatbotx_db::repo::models::Cart;
use serde_json::{Value, json};

use crate::discount_code;
use crate::errors::Result;
use crate::order::{self, Line, PlaceItem, Quote, goods_after_discounts};
use crate::tx;

/// The cart's stored items (`[{productId, variantId, quantity}]`).
pub fn items_of(items: &Value) -> Vec<PlaceItem> {
    let id = |v: &Value| match v {
        Value::String(s) => s.parse::<i64>().ok(),
        other => other.as_i64(),
    };
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            // ponytail: a non-numeric id names no product: left out of the quote.
            Some(PlaceItem {
                product_id: id(&item["productId"])?,
                variant_id: id(&item["variantId"]),
                quantity: item["quantity"].as_i64().unwrap_or(1),
            })
        })
        .collect()
}

fn id_text(id: Option<i64>) -> Value {
    id.map_or(Value::Null, |id| Value::String(id.to_string()))
}

/// A priced line without `gross` / `stockProductId`.
pub fn line_json(line: &Line) -> Value {
    json!({
        "productId": line.product_id.to_string(),
        "variantId": id_text(line.variant_id),
        "vendorId": id_text(line.vendor_id),
        "categoryId": id_text(line.category_id),
        "name": line.name,
        "variantLabel": line.variant_label,
        "combination": line.combination,
        "sku": line.sku,
        "retailerId": null,
        "imageUrl": line.image_url,
        "vendorName": line.vendor_name,
        "unitPrice": line.unit_price,
        "quantity": line.quantity,
        "discountPercent": line.discount_percent,
        "taxPercent": line.tax_percent,
        "discountTotal": line.discount_total,
        "taxTotal": line.tax_total,
        "lineTotal": line.line_total,
    })
}

/// `orderService.quote`'s response.
pub fn quote_json(quote: &Quote, typed_code: bool, available: Vec<Value>) -> Value {
    let draft = &quote.draft;
    let discount = draft.discount.as_ref().map(|d| {
        json!({
            "code": d.code,
            "amount": d.amount,
            "discountType": d.discount_type,
            "discountValue": d.discount_value,
            "automatic": !typed_code || quote.discount_error.is_some(),
        })
    });
    let discount_error = quote
        .discount_error
        .as_ref()
        .map(|(code, data)| match data {
            Some(data) => json!({ "code": code, "data": data }),
            None => json!({ "code": code }),
        });
    let minimum = draft.settings["checkout"]["minimumOrderValue"]
        .as_f64()
        .unwrap_or(0.0);
    let below = minimum > 0.0
        && goods_after_discounts(&draft.totals, draft.discount.as_ref(), &draft.currency) < minimum;
    json!({
        "currency": draft.currency,
        "shippingType": draft.shipping_type,
        "lines": draft.lines.iter().map(line_json).collect::<Vec<_>>(),
        "totals": draft.totals.json(),
        "discount": discount,
        "discountError": discount_error,
        "availableDiscounts": available,
        "unavailable": quote.unavailable.iter().map(|(item, name)| json!({
            "productId": item.product_id.to_string(),
            "variantId": id_text(item.variant_id),
            "quantity": item.quantity,
            "name": name,
        })).collect::<Vec<_>>(),
        "belowMinimum": if below { json!(minimum) } else { Value::Null },
    })
}

/// `getCart`.
pub async fn get_cart(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    contact_id: i64,
    shipping_type: Option<String>,
    list_available: bool,
) -> Result<Value> {
    let cart: Option<Cart> = tx::read(db, move |conn| {
        Ok(find_one(
            conn,
            r#""workspaceId" = ?1 AND "contactId" = ?2"#,
            &[&workspace_id, &contact_id],
        )?)
    })
    .await?;
    let stored = cart
        .as_ref()
        .map_or_else(|| json!([]), |c| c.items.0.clone());
    let code = cart.as_ref().and_then(|c| c.discount_code.clone());
    price(
        db,
        workspace_id,
        stored,
        code,
        Some(contact_id),
        shipping_type,
        list_available,
    )
    .await
}

/// A cart's stored `items` and code priced as an order would be now; a
/// preview cart (`getPreviewCart`) is priced for no contact.
pub async fn price(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    stored: Value,
    code: Option<String>,
    contact_id: Option<i64>,
    shipping_type: Option<String>,
    list_available: bool,
) -> Result<Value> {
    let quote = order::quote(
        db,
        workspace_id,
        items_of(&stored),
        shipping_type,
        contact_id,
        code.clone(),
        true,
    )
    .await?;
    let available = if list_available && !quote.draft.lines.is_empty() {
        let context = quote.draft.coupon_context.clone();
        tx::read(db, move |conn| {
            discount_code::list_available(conn, &context, chatbotx_db::repo::now_ms())
        })
        .await?
    } else {
        Vec::new()
    };
    let mut cart = quote_json(&quote, code.is_some(), available);
    cart["items"] = stored;
    cart["enteredDiscountCode"] = json!(code);
    Ok(cart)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_stored_items() {
        let items = items_of(&json!([
            { "productId": "12", "variantId": null, "quantity": 2 },
            { "productId": "x", "quantity": 1 },
        ]));
        assert_eq!(items.len(), 1);
        assert_eq!(
            (items[0].product_id, items[0].variant_id, items[0].quantity),
            (12, None, 2)
        );
    }
}
