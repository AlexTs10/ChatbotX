//! `integration/handlers/inbound-commerce.ts` and
//! `handlers/reply-to-inbound-cart.ts`: a cart or a product question sent
//! from a channel's own catalog (WhatsApp).

use std::collections::HashMap;

use chatbotx_business::events::{Event, trigger};
use chatbotx_business::webshop::{Shopper, build_inbound_cart_reply};
use chatbotx_core::utils::id::create_id;
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use serde_json::{Map, Value, json};

use crate::channel::{InboundInbox, Received};
use crate::inbound::{Stored, add_job};
use crate::{Deps, fail, id_of, is_blocked_workspace};

pub const REPLY_TO_INBOUND_CART: &str = "replyToInboundCart";

/// `resolve` of a commerce event: `None` when a reference is missing, so
/// nothing half-resolved is stored.
fn resolve(event: &Value, references: &[Value]) -> Option<Value> {
    let by_retailer: HashMap<&str, &Value> = references
        .iter()
        .filter_map(|r| Some((r["catalogReference"]["retailerId"].as_str()?, r)))
        .collect();
    match event["kind"].as_str()? {
        "cartSent" => {
            let items = event["items"]
                .as_array()?
                .iter()
                .map(|item| {
                    let mut resolved = (*by_retailer.get(item["retailerId"].as_str()?)?).clone();
                    resolved["quantity"] = item["quantity"].clone();
                    resolved["itemPrice"] = item["itemPrice"].clone();
                    resolved["currency"] = item["currency"].clone();
                    Some(resolved)
                })
                .collect::<Option<Vec<_>>>()?;
            Some(json!({ "inboundOrder": { "note": event["note"], "items": items } }))
        }
        "productInquiry" => {
            let product = by_retailer.get(event["product"]["retailerId"].as_str()?)?;
            Some(json!({ "referredProduct": product }))
        }
        _ => None,
    }
}

fn catalog_and_retailers(event: &Value) -> Option<(String, Vec<String>)> {
    let text = |v: &Value| v.as_str().map(str::to_owned);
    if event["kind"] == "cartSent" {
        let ids = event["items"]
            .as_array()?
            .iter()
            .filter_map(|i| text(&i["retailerId"]))
            .collect();
        return Some((text(&event["catalogId"])?, ids));
    }
    let product = &event["product"];
    Some((
        text(&product["catalogId"])?,
        vec![text(&product["retailerId"])?],
    ))
}

/// `enrichInboundCommerce`: names what a contact's catalog interaction
/// refers to and stores it on the message before the save. Best effort: an
/// unresolved reference leaves the message unchanged. Never an echo.
pub async fn enrich(deps: &Deps, inbox: &InboundInbox, received: &mut Received) {
    let (Some(event), Some(message)) = (&received.commerce, received.message.as_mut()) else {
        return;
    };
    if message.message_type != "incoming" {
        return;
    }
    let Some((catalog_id, retailer_ids)) = catalog_and_retailers(event) else {
        return;
    };
    let references = chatbotx_business::product_message::service::resolve_inbound_products(
        &deps.db,
        inbox.workspace_id,
        &inbox.channel,
        &catalog_id,
        &retailer_ids,
    )
    .await;
    let Some(Value::Object(extra)) = resolve(event, &references) else {
        return;
    };
    let mut attributes = match message.content_attributes.take() {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    attributes.extend(extra);
    message.content_attributes = Some(Value::Object(attributes));
}

/// `getInboundOrder` of stored attributes: a note and at least one item.
fn stored_order(attributes: &Value) -> Option<&Value> {
    let order = attributes.get("inboundOrder")?;
    order["items"].as_array().filter(|i| !i.is_empty())?;
    Some(order)
}

/// `emitStoredCommerceEvent` and the `replyToInboundCart` enqueue: read back
/// from the SAVED message, once per message, only when this delivery's
/// channel reported a catalog interaction and automation may run.
pub async fn announce(
    deps: &Deps,
    stored: &Stored,
    message: &Value,
    message_id: i64,
) -> Result<(), JobError> {
    let Some(kind) = stored
        .received
        .commerce
        .as_ref()
        .and_then(|c| c["kind"].as_str())
    else {
        return Ok(());
    };
    let attributes = &message["contentAttributes"];
    let (ws, contact) = (
        stored.inbox.workspace_id.to_string(),
        stored.contact_inbox.contact_id.to_string(),
    );
    let inbox_id = stored.contact_inbox.id.to_string();
    let mut metadata = Map::new();
    metadata.insert("channel".into(), json!(stored.inbox.channel));
    metadata.insert(
        "conversationId".into(),
        json!(stored.conversation.id.to_string()),
    );
    metadata.insert("messageId".into(), json!(message_id.to_string()));
    let event = match kind {
        "cartSent" => stored_order(attributes).map(|order| {
            metadata.insert(
                "catalogId".into(),
                order["items"][0]["catalogReference"]["catalogId"].clone(),
            );
            metadata.insert("note".into(), order["note"].clone());
            metadata.insert("products".into(), order["items"].clone());
            trigger::shopping_cart_sent(&ws, &contact, metadata, Some(inbox_id.as_str()))
        }),
        _ => attributes
            .get("referredProduct")
            .filter(|p| p.is_object())
            .map(|product| {
                metadata.insert(
                    "catalogId".into(),
                    product["catalogReference"]["catalogId"].clone(),
                );
                metadata.insert("product".into(), product.clone());
                trigger::user_asked_about_product(&ws, &contact, metadata, Some(inbox_id.as_str()))
            }),
    };
    if let Some(event) = event {
        crate::order_payment::bus(deps)
            .publish(Event::Trigger(event))
            .await;
    }
    if kind == "cartSent"
        && let Some(cart) = stored_order(attributes)
    {
        add_job(
            deps,
            QueueName::Integration,
            REPLY_TO_INBOUND_CART,
            json!({ "type": REPLY_TO_INBOUND_CART, "data": {
                "workspaceId": ws,
                "conversationId": stored.conversation.id.to_string(),
                "contactId": contact,
                "contactInboxId": inbox_id,
                "messageId": message_id.to_string(),
                "cart": cart,
            }}),
            JobOptions {
                job_id: Some(format!("inbound-cart-reply-{message_id}")),
                ..JobOptions::default()
            },
        )
        .await?;
    }
    Ok(())
}

/// A text with one link button, as a flow's own sendText step carries.
fn checkout_step(reply: &Value) -> Value {
    json!({
        "id": create_id(),
        "stepType": "sendText",
        "text": reply["text"],
        "buttons": [{
            "id": create_id(),
            "label": reply["buttonLabel"],
            "buttonType": "openWebsite",
            "beforeStep": {
                "id": create_id(),
                "stepType": "openWebsite",
                "url": reply["buttonUrl"],
                "browserSize": 100,
            },
            "steps": [],
        }],
    })
}

/// `replyToInboundCart`: the cart becomes the contact's shop cart and the
/// bot answers with its summary and a CHECK OUT button, through the
/// flow-step sender (so `{{shop_token}}` is signed for this contact).
pub async fn reply_to_inbound_cart(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let (Some(ws), Some(contact_id), Some(message_id)) = (
        id_of(&data["workspaceId"]),
        id_of(&data["contactId"]),
        id_of(&data["messageId"]),
    ) else {
        return Err(JobError::unrecoverable("replyToInboundCart: bad data"));
    };
    if is_blocked_workspace(&deps.db, &ws.to_string()).await? {
        return Ok(());
    }
    let shopper = Shopper {
        workspace_id: ws,
        contact_id,
        contact_inbox_id: id_of(&data["contactInboxId"]),
        conversation_id: id_of(&data["conversationId"]),
    };
    let reply = build_inbound_cart_reply(&deps.db, &deps.app_url, shopper, &data["cart"])
        .await
        .map_err(|e| fail(e.to_string()))?;
    let Some(reply) = reply else {
        return Ok(());
    };
    add_job(
        deps,
        QueueName::Chat,
        crate::chat::SEND_FLOW_MESSAGE,
        json!({ "type": crate::chat::SEND_FLOW_MESSAGE, "data": {
            "conversationId": data["conversationId"],
            "contactInboxId": data["contactInboxId"],
            // Not part of any flow, like a template sent from the inbox.
            "flowId": "",
            "step": checkout_step(&reply),
        }}),
        JobOptions {
            job_id: Some(format!("inbound-cart-checkout-{message_id}")),
            ..JobOptions::default()
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(retailer: &str) -> Value {
        json!({ "catalogReference": { "catalogId": "c", "retailerId": retailer }, "product": null })
    }

    #[test]
    fn a_cart_resolves_only_when_every_item_does() {
        let event = json!({ "kind": "cartSent", "catalogId": "c", "note": null, "items": [
            { "catalogId": "c", "retailerId": "a", "quantity": 2, "itemPrice": 5, "currency": "EUR" },
            { "catalogId": "c", "retailerId": "b", "quantity": 1, "itemPrice": 3, "currency": "EUR" },
        ] });
        assert_eq!(resolve(&event, &[reference("a")]), None);
        let both = resolve(&event, &[reference("a"), reference("b")]).expect("resolved");
        assert_eq!(both["inboundOrder"]["items"][0]["quantity"], json!(2));
        assert!(stored_order(&both).is_some());
        let inquiry =
            json!({ "kind": "productInquiry", "product": { "catalogId": "c", "retailerId": "a" } });
        assert_eq!(
            resolve(&inquiry, &[reference("a")]),
            Some(json!({ "referredProduct": reference("a") }))
        );
    }
}
