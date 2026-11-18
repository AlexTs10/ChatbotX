//! `afterStatusChange` / `announcePlaced` / `afterPaid` and their helpers:
//! the events, the new-order push, the customer's messages and flows. All
//! best effort: the order already stands.

use chatbotx_core::encryption::Encryption;
use chatbotx_db::Db;
use chatbotx_db::repo::models::{Conversation, Order};
use chatbotx_jobs::{Enqueuer, JobOptions, QueueName};
use chatbotx_rt::Realtime;
use serde_json::{Map, Value, json};

use super::{get_detail, is_online, major, messages};
use crate::events::{Event, EventBus, trigger};
use crate::message::{Outgoing, OutgoingInput, OutgoingJob, create_outgoing};
use crate::{ecommerce_settings, tx};

/// What the order services reach besides the database.
#[derive(Clone, Copy)]
pub struct Deps<'a> {
    pub db: &'a Db,
    pub jobs: Option<&'a dyn Enqueuer>,
    pub events: &'a EventBus,
    pub realtime: Option<&'a Realtime>,
    /// The provider secrets (`None` = `ENCRYPTION_KEY` unset).
    pub encryption: Option<&'a Encryption>,
    pub app_url: &'a str,
}

impl Deps<'_> {
    async fn enqueue(&self, queue: QueueName, name: &str, data: Value, job_id: String) {
        let Some(jobs) = self.jobs else { return };
        let opts = JobOptions {
            job_id: Some(job_id),
            ..JobOptions::default()
        };
        if let Err(error) = jobs.add(queue, name, data, opts).await {
            tracing::warn!(%error, job = name, "[order] could not queue a job");
        }
    }

    /// `contact_messages::deliver`: the realtime `messageCreated` and the jobs.
    pub async fn deliver(&self, conversation: &Conversation, outgoing: Outgoing) {
        if let (Some(realtime), Some(payload)) = (self.realtime, outgoing.realtime) {
            let route = chatbotx_rt::protocol::route_for_conversation(
                Some(&chatbotx_rt::protocol::ConversationRoute {
                    assigned_inbox_team_id: conversation
                        .assigned_inbox_team_id
                        .map(|i| i.to_string()),
                    assigned_user_id: conversation.assigned_user_id.map(|i| i.to_string()),
                    inbox_id: None,
                }),
                None,
            );
            let mut event =
                chatbotx_rt::Envelope::new(chatbotx_rt::EventType::MessageCreated, payload);
            event.route = Some(route);
            realtime
                .queue_workspace_event(&conversation.workspace_id.to_string(), event)
                .await;
        }
        let Some(jobs) = self.jobs else { return };
        for OutgoingJob {
            queue,
            name,
            data,
            attempts,
        } in outgoing.jobs
        {
            let opts = JobOptions {
                attempts,
                ..JobOptions::default()
            };
            if let Err(error) = jobs.add(queue, name, data, opts).await {
                tracing::error!(%error, job = name, "Failed to enqueue outgoing-message job");
            }
        }
    }
}

/// The trigger event of the status an order moved to.
fn status_event(status: &str) -> &'static str {
    match status {
        "accepted" => "orderAccepted",
        "shipped" => "orderShipped",
        "concluded" => "orderConcluded",
        "cancelled" => "orderCancelled",
        _ => "newOrder",
    }
}

struct WorkspaceInfo {
    language: &'static str,
    timezone: String,
}

async fn workspace(db: &Db, workspace_id: i64) -> Option<WorkspaceInfo> {
    tx::read(db, move |conn| {
        Ok(conn.query_row(
            r#"SELECT "language", "timezone" FROM "Workspace" WHERE "id" = ?1"#,
            [workspace_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )?)
    })
    .await
    .ok()
    .map(|(language, timezone)| WorkspaceInfo {
        language: messages::language(Some(&language)),
        timezone,
    })
}

/// `afterStatusChange`: placed (`from` none) or moved.
pub(crate) async fn after_status_change(
    deps: Deps<'_>,
    order: &Order,
    from: Option<&str>,
    message: Option<&str>,
    refunded: bool,
    payment_expired: bool,
) {
    if from.is_none() {
        if !super::is_awaiting_online_payment(order) {
            announce_placed(deps, order).await;
        }
        return;
    }
    if order.contact_id.is_none() {
        return;
    }
    let language = workspace(deps.db, order.workspace_id)
        .await
        .map_or("en", |w| w.language);
    // An online order whose money never came was never announced.
    if is_online(&order.payment_method) && order.paid_at.is_none() {
        if payment_expired {
            let text = messages::status_text(&order.status, None, false, true, language);
            message_customer(deps, order, text).await;
        }
        return;
    }
    announce_events(deps, order, status_event(&order.status), false).await;
    if refunded {
        announce_events(deps, order, "orderRefunded", false).await;
    }
    let notify = match ecommerce_settings::get(deps.db, order.workspace_id).await {
        Ok(settings) => settings["general"]["notifyOnCartStatusChange"] == true,
        Err(error) => {
            tracing::warn!(%error, order_id = order.id, "[order] could not tell the customer about the order");
            return;
        }
    };
    let always = order.status == "cancelled" || order.status == "concluded";
    if notify || always || message.is_some() {
        let text =
            messages::status_text(&order.status, message, refunded, payment_expired, language);
        message_customer(deps, order, text).await;
    }
}

/// `announcePlaced`: the new-order push, `newOrder` + `productOrdered`, the
/// confirmation and the checkout flow. Keyed by the order: sent once.
pub async fn announce_placed(deps: Deps<'_>, order: &Order) {
    deps.enqueue(
        QueueName::Notification,
        "notifyNewOrder",
        json!({
            "type": "notifyNewOrder",
            "data": { "workspaceId": order.workspace_id.to_string(), "orderId": order.id.to_string() },
        }),
        format!("new-order-{}", order.id),
    )
    .await;
    if order.contact_id.is_none() {
        return;
    }
    announce_events(deps, order, "newOrder", true).await;
    let settings = match ecommerce_settings::get(deps.db, order.workspace_id).await {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!(%error, order_id = order.id, "[order] could not tell the customer about the order");
            return;
        }
    };
    confirm_to_customer(deps, order, &settings).await;
    start_checkout_flow(deps, order, &settings).await;
}

/// `afterPaid`: `orderPaid`, then the order is announced as placed.
pub async fn after_paid(deps: Deps<'_>, order: &Order) {
    if order.contact_id.is_some() {
        announce_events(deps, order, "orderPaid", false).await;
    }
    announce_placed(deps, order).await;
}

/// `startCheckoutFlow`: the web shop's "start a flow" checkout.
async fn start_checkout_flow(deps: Deps<'_>, order: &Order, settings: &Value) {
    let checkout = &settings["checkout"];
    let (Some(flow_id), Some(conversation_id), Some(contact_inbox_id)) = (
        checkout["checkoutFlowId"].as_str(),
        order.conversation_id,
        order.contact_inbox_id,
    ) else {
        return;
    };
    if order.source != "webshop" || checkout["checkoutMode"] != "flow" {
        return;
    }
    deps.enqueue(
        QueueName::Integration,
        "sendFlow",
        json!({ "type": "sendFlow", "data": {
            "conversationId": conversation_id.to_string(),
            "contactInboxId": contact_inbox_id.to_string(),
            "flowId": flow_id,
        }}),
        format!("webshop-checkout-flow-{}", order.id),
    )
    .await;
}

/// `confirmToCustomer`: the built-in text, the store's flow, or nothing.
async fn confirm_to_customer(deps: Deps<'_>, order: &Order, settings: &Value) {
    let checkout = &settings["checkout"];
    match checkout["orderConfirmationMode"].as_str() {
        Some("none") => {}
        Some("flow") => {
            if let Some(flow_id) = checkout["orderConfirmationFlowId"].as_str() {
                run_flow_for_customer(deps, order, flow_id).await;
            }
        }
        _ => {
            let Ok(detail) = get_detail(deps.db, order.workspace_id, order.id).await else {
                return;
            };
            let info = workspace(deps.db, order.workspace_id).await;
            let (language, timezone) = info
                .as_ref()
                .map_or(("en", "UTC"), |w| (w.language, w.timezone.as_str()));
            let o = &detail.order;
            let summary = messages::Summary {
                number: o.number,
                currency: &o.currency,
                subtotal: major(o.subtotal),
                discount_total: major(o.discount_total),
                shipping_total: major(o.shipping_total),
                service_fee_total: major(o.service_fee_total),
                tax_total: major(o.tax_total),
                coupon_discount: major(o.coupon_discount),
                discount_code: o.discount_code.as_deref(),
                total: major(o.total),
                customer_name: o.customer_name.as_deref(),
                customer_email: o.customer_email.as_deref(),
                customer_phone: o.customer_phone.as_deref(),
                shipping_type: o.shipping_type.as_deref(),
                shipping_address: o.shipping_address.as_ref().map(|a| &a.0),
                payment_method: &o.payment_method,
                payment_status: &o.payment_status,
                created_at: o.created_at,
                items: detail
                    .items
                    .iter()
                    .map(|i| (i.name.as_str(), i.variant_label.as_deref(), i.quantity))
                    .collect(),
            };
            let text = messages::confirmation_text(
                &summary,
                language,
                &messages::Payments::of(&settings["payments"]),
                settings["shipping"]["storeAddress"].as_str(),
                timezone,
            );
            message_customer(deps, order, text).await;
        }
    }
}

/// `runFlowForCustomer`: one job per order.
async fn run_flow_for_customer(deps: Deps<'_>, order: &Order, flow_id: &str) {
    let Some(contact_id) = order.contact_id else {
        return;
    };
    match crate::conversation::resolve_contact_inbox_for_send(
        deps.db,
        order.workspace_id,
        contact_id,
        order.inbox_id,
    )
    .await
    {
        Ok((found, contact_inbox)) => {
            deps.enqueue(
                QueueName::Integration,
                "sendFlow",
                json!({ "type": "sendFlow", "data": {
                    "conversationId": found.conversation.id.to_string(),
                    "contactInboxId": contact_inbox.id.to_string(),
                    "flowId": flow_id,
                }}),
                format!("order-confirmation-flow-{}", order.id),
            )
            .await;
        }
        Err(error) => {
            tracing::warn!(%error, order_id = order.id, "[order] could not run the order confirmation flow");
        }
    }
}

/// `announceOrderEvents`: the trigger / webhook event, and `productOrdered`
/// once when placed.
pub(crate) async fn announce_events(
    deps: Deps<'_>,
    order: &Order,
    event: &str,
    product_ordered: bool,
) {
    let Some(contact_id) = order.contact_id else {
        return;
    };
    let (workspace_id, order_id) = (order.workspace_id, order.id);
    let read = tx::read(deps.db, move |conn| {
        let detail = super::find_detail(conn, workspace_id, order_id)?;
        let ids: Vec<i64> = detail.items.iter().filter_map(|i| i.product_id).collect();
        let products = chatbotx_db::repo::product::find_by_ids(conn, workspace_id, &ids)?;
        Ok((detail, products))
    })
    .await;
    let (detail, products) = match read {
        Ok(read) => read,
        Err(error) => {
            tracing::warn!(%error, order_id, "[order] could not announce the order events");
            return;
        }
    };
    let d = &detail.order;
    let product_list: Vec<Value> = detail
        .items
        .iter()
        .map(|item| {
            let sub = item
                .product_id
                .and_then(|id| products.iter().find(|p| p.id == id))
                .and_then(|p| p.subcategory_id);
            let category_ids: Vec<String> = [item.category_id, sub]
                .into_iter()
                .flatten()
                .map(|i| i.to_string())
                .collect();
            json!({
                "productId": item.product_id.map(|i| i.to_string()),
                "variantId": item.variant_id.map(|i| i.to_string()),
                "name": item.name,
                "categoryIds": category_ids,
                "quantity": item.quantity,
                "lineTotal": super::money(item.line_total),
            })
        })
        .collect();
    let metadata: Map<String, Value> = match json!({
        "orderId": d.id.to_string(),
        "orderNumber": d.number,
        "status": order.status,
        "source": d.source,
        "total": super::money(d.total),
        "currency": d.currency,
        "discountCode": d.discount_code,
        "channel": d.channel,
        "conversationId": d.conversation_id.map(|i| i.to_string()),
        "products": product_list,
    }) {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    let ws = order.workspace_id.to_string();
    let contact = contact_id.to_string();
    let inbox = order.contact_inbox_id.map(|i| i.to_string());
    let mut events = vec![Event::Trigger(trigger::order_status(
        event,
        &ws,
        &contact,
        metadata.clone(),
        inbox.as_deref(),
    ))];
    if product_ordered {
        events.push(Event::Trigger(trigger::product_ordered(
            &ws,
            &contact,
            metadata,
            inbox.as_deref(),
        )));
    }
    deps.events.publish_all(events).await;
}

/// `messageCustomer`: best effort (no conversation, a closed window).
pub(crate) async fn message_customer(deps: Deps<'_>, order: &Order, text: String) {
    let Some(contact_id) = order.contact_id else {
        return;
    };
    let sent = async {
        let (found, contact_inbox) = crate::conversation::resolve_contact_inbox_for_send(
            deps.db,
            order.workspace_id,
            contact_id,
            order.inbox_id,
        )
        .await?;
        let input = OutgoingInput {
            text: Some(text),
            ..OutgoingInput::default()
        };
        let outgoing =
            create_outgoing(deps.db, &found.conversation, &contact_inbox, input, None).await?;
        deps.deliver(&found.conversation, outgoing).await;
        Ok::<_, crate::errors::BusinessError>(())
    }
    .await;
    if let Err(error) = sent {
        tracing::warn!(%error, order_id = order.id, "[order] could not message the customer about a status change");
    }
}
