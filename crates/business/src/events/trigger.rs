//! The event builders of `BaseEventEmitter` (`base-emitter.ts`: one method
//! per event, each `emit(type, { workspaceId, contactId, metadata })`) and
//! the fan-out helpers of `event-dispatcher.ts` (`emit*`), which publish the
//! built events on the [`EventBus`].

use serde_json::{Map, Value};

use super::{Event, EventBus, TriggerEvent};

fn event(
    event_type: &str,
    workspace_id: &str,
    contact_id: &str,
    metadata: Option<Map<String, Value>>,
) -> TriggerEvent {
    TriggerEvent {
        event_type: event_type.to_string(),
        workspace_id: workspace_id.to_string(),
        contact_id: contact_id.to_string(),
        metadata: metadata.unwrap_or_default(),
    }
}

fn with_inbox(
    metadata: Map<String, Value>,
    contact_inbox_id: Option<&str>,
) -> Option<Map<String, Value>> {
    super::contact_inbox::with_contact_inbox_metadata(Some(metadata), contact_inbox_id)
}

/// `tagApplied`: metadata `{ sourceId: tagId, tagId }`.
pub fn tag_applied(
    workspace_id: &str,
    contact_id: &str,
    tag_id: &str,
    contact_inbox_id: Option<&str>,
) -> TriggerEvent {
    let _ = (workspace_id, contact_id, tag_id, contact_inbox_id);
    todo!("P3.6-X1")
}

/// `tagRemoved`: metadata `{ sourceId: tagId, tagId }`.
pub fn tag_removed(
    workspace_id: &str,
    contact_id: &str,
    tag_id: &str,
    contact_inbox_id: Option<&str>,
) -> TriggerEvent {
    let _ = (workspace_id, contact_id, tag_id, contact_inbox_id);
    todo!("P3.6-X1")
}

/// `customFieldChanged` (type `customFieldValueChanged`): metadata
/// `{ sourceId: customFieldId, customFieldId, customFieldName, oldValue,
/// newValue }`.
pub fn custom_field_changed(
    workspace_id: &str,
    contact_id: &str,
    custom_field_id: &str,
    custom_field_name: &str,
    old_value: Value,
    new_value: Value,
    contact_inbox_id: Option<&str>,
) -> TriggerEvent {
    let mut metadata = Map::new();
    metadata.insert("sourceId".into(), Value::from(custom_field_id));
    metadata.insert("customFieldId".into(), Value::from(custom_field_id));
    metadata.insert("customFieldName".into(), Value::from(custom_field_name));
    metadata.insert("oldValue".into(), old_value);
    metadata.insert("newValue".into(), new_value);
    event(
        "customFieldValueChanged",
        workspace_id,
        contact_id,
        with_inbox(metadata, contact_inbox_id),
    )
}

/// `contactCreated` (type `newContact`): metadata `{ name, phone, email }`
/// (a `None` field is left out, as `JSON.stringify` drops `undefined`).
pub fn contact_created(
    workspace_id: &str,
    contact_id: &str,
    name: Option<&str>,
    phone: Option<&str>,
    email: Option<&str>,
    contact_inbox_id: Option<&str>,
) -> TriggerEvent {
    let _ = (
        workspace_id,
        contact_id,
        name,
        phone,
        email,
        contact_inbox_id,
    );
    todo!("P3.6-X1")
}

/// `shoppingCartSent`: the metadata as it is (no `sourceId`).
pub fn shopping_cart_sent(
    workspace_id: &str,
    contact_id: &str,
    metadata: Map<String, Value>,
    contact_inbox_id: Option<&str>,
) -> TriggerEvent {
    let _ = (workspace_id, contact_id, metadata, contact_inbox_id);
    todo!("P3.6-X1")
}

/// `userAskedAboutProduct`: the metadata as it is (no `sourceId`).
pub fn user_asked_about_product(
    workspace_id: &str,
    contact_id: &str,
    metadata: Map<String, Value>,
    contact_inbox_id: Option<&str>,
) -> TriggerEvent {
    let _ = (workspace_id, contact_id, metadata, contact_inbox_id);
    todo!("P3.6-X1")
}

/// `orderStatus(eventType, ...)`: one of `ORDER_STATUS_EVENT_TYPES`.
pub fn order_status(
    event_type: &str,
    workspace_id: &str,
    contact_id: &str,
    metadata: Map<String, Value>,
    contact_inbox_id: Option<&str>,
) -> TriggerEvent {
    event(
        event_type,
        workspace_id,
        contact_id,
        with_inbox(metadata, contact_inbox_id),
    )
}

/// `pipelineItem(eventType, ...)`: one of `PIPELINE_EVENT_TYPES`; the
/// metadata carries `sourceId` (the pipeline id).
pub fn pipeline_item(
    event_type: &str,
    workspace_id: &str,
    contact_id: &str,
    metadata: Map<String, Value>,
    contact_inbox_id: Option<&str>,
) -> TriggerEvent {
    event(
        event_type,
        workspace_id,
        contact_id,
        with_inbox(metadata, contact_inbox_id),
    )
}

/// `cartItemChanged(eventType, ...)`.
pub fn cart_item_changed(
    event_type: &str,
    workspace_id: &str,
    contact_id: &str,
    metadata: Map<String, Value>,
    contact_inbox_id: Option<&str>,
) -> TriggerEvent {
    event(
        event_type,
        workspace_id,
        contact_id,
        with_inbox(metadata, contact_inbox_id),
    )
}

/// The events of `emitCartItemChanged`: a negative `change` is
/// `productRemovedFromCart`; else `productAddedToCart`, plus
/// `categoryAddedToCart` when `product.categoryIds` is not empty.
pub fn cart_item_changed_events(
    workspace_id: &str,
    contact_id: &str,
    metadata: &Map<String, Value>,
    contact_inbox_id: Option<&str>,
) -> Vec<TriggerEvent> {
    let _ = (workspace_id, contact_id, metadata, contact_inbox_id);
    todo!("P3.6-X1")
}

/// `emitCartItemChanged`: publishes [`cart_item_changed_events`] in order.
pub async fn emit_cart_item_changed(
    bus: &EventBus,
    workspace_id: &str,
    contact_id: &str,
    metadata: &Map<String, Value>,
    contact_inbox_id: Option<&str>,
) {
    let events = cart_item_changed_events(workspace_id, contact_id, metadata, contact_inbox_id);
    bus.publish_all(events.into_iter().map(Event::Trigger))
        .await;
}
