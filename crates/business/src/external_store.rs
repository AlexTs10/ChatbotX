//! `external-store/orders.ts`, `orderService.syncExternal` and the store
//! source helpers of `ecommerce-settings/store-source.ts` that Shopify and
//! WooCommerce share: an order mirrored from the other store, the contact
//! behind it, and the connection's sync columns.

use chatbotx_core::encryption::{ShopCustomerClaims, ShopTokenPayload};
use chatbotx_db::repo::models::{Cart, Order};
use chatbotx_db::repo::order as order_repo;
use chatbotx_db::repo::patch::Patch;
use chatbotx_db::repo::{self, store_integration};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::ecommerce_settings::currency_symbol_for;
use crate::errors::{BusinessError, Result};
use crate::order::{self, Actor, ContactRef, Deps, scaled};
use crate::tx;

/// The order attribute (Shopify note attribute, WooCommerce order meta) our
/// web shop checkout stamps with the customer: a signed shop token.
pub const EXTERNAL_CONTACT_ATTRIBUTE: &str = "cbx_ref";

/// How long that reference stays valid.
const CONTACT_REFERENCE_TTL_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// `signContactReference`.
pub fn sign_contact_reference(
    workspace_id: i64,
    contact_id: i64,
    contact_inbox_id: Option<i64>,
    conversation_id: Option<i64>,
) -> Result<String> {
    crate::meta_catalog::process_encryption()
        .sign_shop_token(
            &ShopCustomerClaims {
                workspace_id: workspace_id.to_string(),
                contact_id: contact_id.to_string(),
                contact_inbox_id: contact_inbox_id.map(|id| id.to_string()),
                conversation_id: conversation_id.map(|id| id.to_string()),
            },
            CONTACT_REFERENCE_TTL_MS,
        )
        .map_err(|e| BusinessError::Repo(repo::Error::msg(e.to_string())))
}

/// `ExternalOrderContact`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalOrderContact {
    pub contact_id: i64,
    pub contact_inbox_id: Option<i64>,
    pub conversation_id: Option<i64>,
    /// Checked out from our shop link (the signed reference matched).
    pub from_shop_link: bool,
}

fn parse_id(text: Option<&str>) -> Option<i64> {
    text.and_then(|t| t.parse().ok())
}

/// `resolveExternalOrderContact`: the signed reference, else exactly one of
/// the identifiers (`email:…`, `phone:…`, email first).
pub async fn resolve_external_order_contact(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    reference: Option<&str>,
    identifiers: &[String],
) -> Option<ExternalOrderContact> {
    if let Some(reference) = reference
        // A forged or expired reference falls back to email and phone.
        && let Ok(ShopTokenPayload::Customer {
            workspace_id: ws,
            contact_id,
            contact_inbox_id,
            conversation_id,
            ..
        }) = crate::meta_catalog::process_encryption().verify_shop_token(reference)
        && ws == workspace_id.to_string()
        && let Some(contact_id) = parse_id(Some(&contact_id))
    {
        return Some(ExternalOrderContact {
            contact_id,
            contact_inbox_id: parse_id(contact_inbox_id.as_deref()),
            conversation_id: parse_id(conversation_id.as_deref()),
            from_shop_link: true,
        });
    }
    for identifier in identifiers {
        if let Ok(contact_id) =
            crate::contact::service::resolve_id_by_identifier(db, workspace_id, identifier).await
        {
            return Some(ExternalOrderContact {
                contact_id,
                contact_inbox_id: None,
                conversation_id: None,
                from_shop_link: false,
            });
        }
    }
    None
}

fn id_key(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// `removeOrderedFromCart`: what a shop-link customer ordered on the other
/// store leaves their cart. Best effort.
pub async fn remove_ordered_from_cart(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    contact: &ExternalOrderContact,
    items: &[MirrorItem],
) {
    let ordered: Vec<String> = items
        .iter()
        .map(|i| {
            format!(
                "{}\0{}",
                i.product_id.map(|p| p.to_string()).unwrap_or_default(),
                i.variant_id.map(|v| v.to_string()).unwrap_or_default()
            )
        })
        .collect();
    let contact_id = contact.contact_id;
    let contact_inbox_id = contact.contact_inbox_id;
    let result = tx::write(db, move |tx| {
        let conn: &Connection = tx;
        let Some(cart) = repo::find_one::<Cart>(
            conn,
            "\"workspaceId\" = ?1 AND \"contactId\" = ?2",
            &[&workspace_id, &contact_id],
        )?
        else {
            return Ok(());
        };
        let lines = cart.items.0.as_array().cloned().unwrap_or_default();
        let left: Vec<Value> = lines
            .iter()
            .filter(|line| {
                let key = format!(
                    "{}\0{}",
                    id_key(&line["productId"]),
                    id_key(&line["variantId"])
                );
                !ordered.contains(&key)
            })
            .cloned()
            .collect();
        if left.len() == lines.len() {
            return Ok(());
        }
        conn.execute(
            r#"UPDATE "Cart" SET "items" = ?3, "contactInboxId" = ?4, "updatedAt" = ?5, "lastActivityAt" = ?5
               WHERE "workspaceId" = ?1 AND "contactId" = ?2"#,
            params![
                workspace_id,
                contact_id,
                Value::Array(left).to_string(),
                contact_inbox_id,
                repo::now_ms()
            ],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(())
    })
    .await;
    if let Err(error) = result {
        tracing::warn!(%error, workspace_id, contact_id, "[external-store] could not empty the cart of a mirrored order");
    }
}

fn status_rank(status: &str) -> u8 {
    match status {
        "pending" => 0,
        "accepted" => 1,
        "shipped" => 2,
        _ => 3,
    }
}

/// `nextMirroredStatus`: a finished order stays finished, and never steps back.
pub fn next_mirrored_status<'a>(current: &'a str, incoming: &'a str) -> &'a str {
    if current == "concluded" || current == "cancelled" {
        return current;
    }
    if incoming == "cancelled" {
        return incoming;
    }
    if status_rank(incoming) > status_rank(current) {
        incoming
    } else {
        current
    }
}

/// `variantLabelOf`: "Size: M, Color: Black".
pub fn variant_label_of(combination: Option<&Value>) -> Option<String> {
    let map = combination?.as_object()?;
    Some(
        map.iter()
            .map(|(k, v)| format!("{k}: {}", v.as_str().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// `splitCustomerName`: "Nguyen Van A" -> last "Nguyen", first "Van A".
pub fn split_customer_name(name: Option<&str>) -> (Option<String>, String) {
    let parts: Vec<&str> = name.unwrap_or_default().split_whitespace().collect();
    match parts.as_slice() {
        [] => (None, "Customer".to_string()),
        [one] => (None, (*one).to_string()),
        [last, rest @ ..] => (Some(rest.join(" ")), (*last).to_string()),
    }
}

/// `syncErrorMessage`: at most 500 characters.
pub fn sync_error_message(error: &str) -> String {
    error.chars().take(500).collect()
}

// ---- store source -----------------------------------------------------------

/// `adoptExternalStoreCurrency`: a store selling from `source` follows its
/// currency. `symbol` `None` = the usual symbol of the currency.
pub async fn adopt_external_store_currency(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    source: &'static str,
    currency: &str,
    symbol: Option<Option<String>>,
    position: Option<String>,
) -> Result<()> {
    let currency = currency.to_uppercase();
    let symbol = symbol.unwrap_or_else(|| currency_symbol_for(&currency).map(str::to_string));
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        conn.execute(
            r#"UPDATE "EcommerceSettings" SET "currency" = ?3, "currencySymbol" = ?4,
                 "currencyPosition" = COALESCE(?5, "currencyPosition"), "updatedAt" = ?6
               WHERE "workspaceId" = ?1 AND "storeSource" = ?2"#,
            params![
                workspace_id,
                source,
                currency,
                symbol,
                position,
                repo::now_ms()
            ],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(())
    })
    .await
}

/// `switchStoreBackFrom`: a disconnected store stops being the source.
pub async fn switch_store_back_from(
    db: &chatbotx_db::Db,
    workspace_id: i64,
    source: &'static str,
) -> Result<()> {
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        conn.execute(
            r#"UPDATE "EcommerceSettings" SET "storeSource" = 'chatbotx', "updatedAt" = ?3
               WHERE "workspaceId" = ?1 AND "storeSource" = ?2"#,
            params![workspace_id, source, repo::now_ms()],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(())
    })
    .await
}

/// The store the workspace sells from.
pub async fn store_source(db: &chatbotx_db::Db, workspace_id: i64) -> Result<String> {
    tx::read(db, move |conn| {
        crate::store_source::resolve_store_source(conn, workspace_id)
    })
    .await
}

/// `updateIntegration` on `IntegrationShopify` / `IntegrationWoocommerce`.
pub async fn update_connection(
    db: &chatbotx_db::Db,
    source: &'static str,
    workspace_id: i64,
    values: Patch,
) -> Result<()> {
    tx::write(db, move |tx| {
        let conn: &Connection = tx;
        if source == "shopify" {
            store_integration::update_shopify(conn, workspace_id, &values)?;
        } else {
            store_integration::update_woocommerce(conn, workspace_id, &values)?;
        }
        Ok(())
    })
    .await
}

/// The common sync columns of a connection.
pub fn text(value: Option<&str>) -> Sql {
    value.map_or(Sql::Null, |v| Sql::Text(v.to_string()))
}

pub fn int(value: Option<i64>) -> Sql {
    value.map_or(Sql::Null, Sql::Integer)
}

/// `resolveExternalStoreLinks`: the connected store's links when the store
/// sells from it, else `null`.
pub async fn store_links(db: &chatbotx_db::Db, app_url: &str, workspace_id: i64) -> Result<Value> {
    let source = store_source(db, workspace_id).await?;
    if source == crate::shopify::SOURCE {
        let status = crate::shopify::get_status(db, app_url, workspace_id).await?;
        if status["connected"] == true {
            return Ok(serde_json::json!({
                "source": "shopify",
                "storeName": "Shopify",
                "shopLabel": status["shopDomain"],
                "adminUrl": status["adminUrl"],
            }));
        }
    }
    Ok(Value::Null)
}

// ---- orderService.syncExternal ----------------------------------------------

/// `OrderItemInsertValues` of a mirrored order line (money in major units).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MirrorItem {
    pub product_id: Option<i64>,
    pub variant_id: Option<i64>,
    pub vendor_id: Option<i64>,
    pub category_id: Option<i64>,
    pub name: String,
    pub variant_label: Option<String>,
    pub combination: Option<Value>,
    pub sku: Option<String>,
    pub image_url: Option<String>,
    pub unit_price: f64,
    pub quantity: i64,
    pub discount_percent: f64,
    pub discount_total: f64,
    pub tax_total: f64,
    pub line_total: f64,
}

impl MirrorItem {
    fn patch(&self) -> Patch {
        vec![
            ("productId", int(self.product_id)),
            ("variantId", int(self.variant_id)),
            ("vendorId", int(self.vendor_id)),
            ("categoryId", int(self.category_id)),
            ("name", Sql::Text(self.name.clone())),
            ("variantLabel", text(self.variant_label.as_deref())),
            (
                "combination",
                self.combination
                    .as_ref()
                    .map_or(Sql::Null, |c| Sql::Text(c.to_string())),
            ),
            ("sku", text(self.sku.as_deref())),
            ("retailerId", Sql::Null),
            ("imageUrl", text(self.image_url.as_deref())),
            ("vendorName", Sql::Null),
            ("unitPrice", Sql::Integer(scaled(self.unit_price))),
            ("quantity", Sql::Integer(self.quantity)),
            ("stockTaken", Sql::Integer(0)),
            (
                "discountPercent",
                Sql::Integer(scaled(self.discount_percent)),
            ),
            ("taxPercent", Sql::Integer(0)),
            ("discountTotal", Sql::Integer(scaled(self.discount_total))),
            ("taxTotal", Sql::Integer(scaled(self.tax_total))),
            ("lineTotal", Sql::Integer(scaled(self.line_total))),
        ]
    }
}

/// The mirrored order's columns (money in major units).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MirrorOrder {
    pub external_id: String,
    pub external_name: Option<String>,
    pub status: String,
    pub contact: Option<ExternalOrderContact>,
    pub payment_method: String,
    pub payment_status: String,
    pub paid_at: Option<i64>,
    pub fulfillment_status: Option<String>,
    pub tracking_company: Option<String>,
    pub tracking_number: Option<String>,
    pub tracking_url: Option<String>,
    pub currency: String,
    pub subtotal: f64,
    pub discount_total: f64,
    pub shipping_total: f64,
    pub tax_total: f64,
    pub total: f64,
    pub discount_code: Option<String>,
    pub customer_name: Option<String>,
    pub customer_email: Option<String>,
    pub customer_phone: Option<String>,
    pub shipping_type: Option<String>,
    pub shipping_address: Option<Value>,
    pub note: Option<String>,
    pub cancel_reason: Option<String>,
    /// When it was placed on the other store; kept on create only.
    pub created_at: Option<i64>,
}

fn status_column(status: &str) -> Option<&'static str> {
    match status {
        "accepted" => Some("acceptedAt"),
        "shipped" => Some("shippedAt"),
        "concluded" => Some("concludedAt"),
        "cancelled" => Some("cancelledAt"),
        _ => None,
    }
}

struct Written {
    order: Order,
    from: Option<String>,
    created: bool,
    was_paid: bool,
}

/// `orderService.syncExternal`: created the first time (matched by
/// `externalId`), updated after; statuses only move forward. With
/// `announce`, a new order or a move is announced like one made here.
pub async fn sync_external(
    deps: Deps<'_>,
    workspace_id: i64,
    source: &'static str,
    input: MirrorOrder,
    items: Vec<MirrorItem>,
    announce: bool,
) -> Result<(Order, bool)> {
    let contact = input.contact.as_ref().map(|c| ContactRef {
        contact_id: Some(c.contact_id),
        contact_inbox_id: c.contact_inbox_id,
        inbox_id: None,
        conversation_id: c.conversation_id,
        channel: None,
    });
    let conversation = match contact {
        Some(c) => order::resolve_conversation(deps.db, workspace_id, Some(c)).await?,
        None => None,
    }
    .unwrap_or_default();
    // ponytail: one SQLite writer, so the checkout and the orders/create
    // webhook cannot both insert; the original's unique-violation retry is not needed.
    let written = tx::write(deps.db, move |tx| {
        let conn: &Connection = tx;
        let current =
            order_repo::find_by_external_id_for_update(conn, workspace_id, &input.external_id)?;
        let now = repo::now_ms();
        let address = input
            .shipping_address
            .as_ref()
            .map_or(Sql::Null, |a| Sql::Text(a.to_string()));
        let mut values: Patch = vec![
            ("externalName", text(input.external_name.as_deref())),
            ("contactId", int(conversation.contact_id)),
            ("contactInboxId", int(conversation.contact_inbox_id)),
            ("conversationId", int(conversation.conversation_id)),
            ("inboxId", int(conversation.inbox_id)),
            ("channel", text(conversation.channel.as_deref())),
            ("paymentMethod", Sql::Text(input.payment_method.clone())),
            ("paymentStatus", Sql::Text(input.payment_status.clone())),
            ("paidAt", int(input.paid_at)),
            (
                "fulfillmentStatus",
                text(input.fulfillment_status.as_deref()),
            ),
            ("trackingCompany", text(input.tracking_company.as_deref())),
            ("trackingNumber", text(input.tracking_number.as_deref())),
            ("trackingUrl", text(input.tracking_url.as_deref())),
            ("currency", Sql::Text(input.currency.clone())),
            ("subtotal", Sql::Integer(scaled(input.subtotal))),
            ("discountTotal", Sql::Integer(scaled(input.discount_total))),
            ("couponDiscount", Sql::Integer(0)),
            ("shippingTotal", Sql::Integer(scaled(input.shipping_total))),
            ("serviceFeeTotal", Sql::Integer(0)),
            ("taxTotal", Sql::Integer(scaled(input.tax_total))),
            ("total", Sql::Integer(scaled(input.total))),
            ("discountCodeId", Sql::Null),
            ("discountCode", text(input.discount_code.as_deref())),
            ("customerName", text(input.customer_name.as_deref())),
            ("customerEmail", text(input.customer_email.as_deref())),
            ("customerPhone", text(input.customer_phone.as_deref())),
            ("shippingType", text(input.shipping_type.as_deref())),
            ("shippingAddress", address),
            ("note", text(input.note.as_deref())),
            ("cancelReason", text(input.cancel_reason.as_deref())),
        ];
        let rows: Vec<Patch> = items.iter().map(MirrorItem::patch).collect();
        let system = || Actor {
            kind: "system",
            user_id: None,
        };
        let Some(current) = current else {
            let number = order::next_order_number(conn, workspace_id)?;
            values.extend([
                ("workspaceId", Sql::Integer(workspace_id)),
                ("number", Sql::Integer(number)),
                ("source", Sql::Text(source.to_string())),
                ("externalId", Sql::Text(input.external_id.clone())),
                ("status", Sql::Text(input.status.clone())),
            ]);
            if let Some(column) = status_column(&input.status) {
                values.push((column, Sql::Integer(now)));
            }
            if let Some(at) = input.created_at {
                values.push(("createdAt", Sql::Integer(at)));
                values.push(("updatedAt", Sql::Integer(now)));
            }
            let created = order_repo::insert(conn, &values)?.ok_or_else(|| {
                BusinessError::Repo(repo::Error::msg("Failed to save the mirrored order"))
            })?;
            order_repo::insert_items(conn, workspace_id, created.id, &rows)?;
            order::insert_history(
                conn,
                workspace_id,
                created.id,
                None,
                &created.status,
                system(),
                None,
            )?;
            return Ok(Written {
                order: created,
                from: None,
                created: true,
                was_paid: false,
            });
        };
        let next = next_mirrored_status(&current.status, &input.status).to_string();
        let moved = next != current.status;
        // The contact found the first time is kept; paid the first time too.
        let keep = |key: &str, existing: Option<i64>| -> (&'static str, Sql) {
            let key: &'static str = match key {
                "contactId" => "contactId",
                "contactInboxId" => "contactInboxId",
                "inboxId" => "inboxId",
                "conversationId" => "conversationId",
                _ => "paidAt",
            };
            let incoming = values
                .iter()
                .find(|(k, _)| *k == key)
                .map_or(Sql::Null, |(_, v)| v.clone());
            (key, existing.map_or(incoming, Sql::Integer))
        };
        let kept = [
            keep("contactId", current.contact_id),
            keep("contactInboxId", current.contact_inbox_id),
            keep("inboxId", current.inbox_id),
            keep("conversationId", current.conversation_id),
            keep("paidAt", current.paid_at),
        ];
        let channel = current
            .channel
            .clone()
            .map(Sql::Text)
            .unwrap_or_else(|| text(conversation.channel.as_deref()));
        values.retain(|(k, _)| {
            !matches!(
                *k,
                "contactId"
                    | "contactInboxId"
                    | "inboxId"
                    | "conversationId"
                    | "paidAt"
                    | "channel"
            )
        });
        values.extend(kept);
        values.push(("channel", channel));
        values.push(("status", Sql::Text(next.clone())));
        if moved && let Some(column) = status_column(&next) {
            values.push((column, Sql::Integer(now)));
        }
        let updated =
            order_repo::update(conn, workspace_id, current.id, &values)?.ok_or_else(|| {
                BusinessError::Repo(repo::Error::msg("Failed to update the mirrored order"))
            })?;
        order_repo::delete_items(conn, workspace_id, current.id)?;
        order_repo::insert_items(conn, workspace_id, current.id, &rows)?;
        if moved {
            order::insert_history(
                conn,
                workspace_id,
                current.id,
                Some(&current.status),
                &next,
                system(),
                None,
            )?;
        }
        Ok(Written {
            order: updated,
            from: moved.then(|| current.status.clone()),
            created: false,
            was_paid: current.payment_status == "paid",
        })
    })
    .await?;

    let Written {
        order,
        from,
        created,
        was_paid,
    } = written;
    if !announce {
        return Ok((order, created));
    }
    if created {
        let mut placed = order.clone();
        placed.status = "pending".into();
        order::after_status_change(deps, &placed, None, None, false, false).await;
        if order.status != "pending" {
            order::after_status_change(deps, &order, Some("pending"), None, false, false).await;
        }
    } else if let Some(from) = &from {
        order::after_status_change(deps, &order, Some(from), None, false, false).await;
    }
    if !was_paid && order.payment_status == "paid" && order.contact_id.is_some() {
        order::announce_events(deps, &order, "orderPaid", false).await;
    }
    Ok((order, created))
}

/// The id of an `Order` already mirrored from `external_id`, if any.
pub fn find_mirrored(
    conn: &Connection,
    workspace_id: i64,
    external_id: &str,
) -> Result<Option<i64>> {
    conn.query_row(
        r#"SELECT "id" FROM "Order" WHERE "workspaceId" = ?1 AND "externalId" = ?2"#,
        params![workspace_id, external_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| BusinessError::Repo(e.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrored_statuses_only_move_forward() {
        assert_eq!(next_mirrored_status("pending", "shipped"), "shipped");
        assert_eq!(next_mirrored_status("shipped", "accepted"), "shipped");
        assert_eq!(next_mirrored_status("accepted", "cancelled"), "cancelled");
        assert_eq!(next_mirrored_status("concluded", "cancelled"), "concluded");
        assert_eq!(next_mirrored_status("cancelled", "shipped"), "cancelled");
    }

    #[test]
    fn names_and_labels() {
        assert_eq!(
            split_customer_name(Some("Nguyen Van A")),
            (Some("Van A".into()), "Nguyen".into())
        );
        assert_eq!(split_customer_name(Some(" Ann ")), (None, "Ann".into()));
        assert_eq!(split_customer_name(None), (None, "Customer".into()));
        let combination = serde_json::json!({ "Size": "M", "Color": "Black" });
        assert_eq!(
            variant_label_of(Some(&combination)).as_deref(),
            Some("Color: Black, Size: M")
        );
    }
}
