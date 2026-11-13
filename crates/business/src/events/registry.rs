//! `event-type-registry.ts`: which trigger event types are emitted, filtered
//! by the evaluator, or free of the channel context.

/// `EMITTED_EVENT_TYPES`: the types the trigger and webhook emitters queue.
pub const EMITTED_EVENT_TYPES: &[&str] = &[
    "tagApplied",
    "tagRemoved",
    "customFieldValueChanged",
    "contactInfoUpdated",
    "conversationTransferredToHuman",
    "conversationTransferredToBot",
    "newContact",
    "contactUnsubscribedFormBroadcast",
    "archived",
    "followUp",
    "conversationAssigned",
    "conversationUnassigned",
    "subscribedToSequence",
    "unsubscribedFromSequence",
    "contactReferredANewContact",
    "contactReferredExistingContact",
    "contactBlockedBot",
    "contactUnblockedBot",
    "incomingCall",
    "missedAudioCall",
    "callEnded",
    "callRecorded",
    "callTranscribed",
    "shoppingCartSent",
    "userAskedAboutProduct",
    "newOrder",
    "orderAccepted",
    "orderShipped",
    "orderConcluded",
    "orderCancelled",
    "orderPaid",
    "orderRefunded",
    "productOrdered",
    "productAddedToCart",
    "productRemovedFromCart",
    "categoryAddedToCart",
    "cartAbandoned",
    "pipelineItemCreated",
    "pipelineItemMovedToStage",
    "pipelineItemValueChanged",
    "pipelineItemStatusChanged",
    "pipelineItemPriorityChanged",
];

/// `ORDER_STATUS_EVENT_TYPES`.
pub const ORDER_STATUS_EVENT_TYPES: &[&str] = &[
    "newOrder",
    "orderAccepted",
    "orderShipped",
    "orderConcluded",
    "orderCancelled",
    "orderPaid",
    "orderRefunded",
];

/// `COMMERCE_EVENT_TYPES` minus the order statuses.
const COMMERCE_EXTRA_EVENT_TYPES: &[&str] = &[
    "productOrdered",
    "productAddedToCart",
    "productRemovedFromCart",
    "categoryAddedToCart",
    "cartAbandoned",
];

/// `PIPELINE_EVENT_TYPES`.
pub const PIPELINE_EVENT_TYPES: &[&str] = &[
    "pipelineItemCreated",
    "pipelineItemMovedToStage",
    "pipelineItemValueChanged",
    "pipelineItemStatusChanged",
    "pipelineItemPriorityChanged",
];

/// `EVALUATOR_FILTERED_EVENT_TYPES`: conditions filter these by product or
/// category in the evaluator, not by the event `sourceId`.
pub const EVALUATOR_FILTERED_EVENT_TYPES: &[&str] = &[
    "shoppingCartSent",
    "userAskedAboutProduct",
    "productOrdered",
    "productAddedToCart",
    "productRemovedFromCart",
    "categoryAddedToCart",
];

/// `EMITTED_EVENT_TYPE_SET.has`.
pub fn is_emitted(event_type: &str) -> bool {
    EMITTED_EVENT_TYPES.contains(&event_type)
}

/// `EVALUATOR_FILTERED_EVENT_TYPE_SET.has`.
pub fn is_evaluator_filtered(event_type: &str) -> bool {
    EVALUATOR_FILTERED_EVENT_TYPES.contains(&event_type)
}

/// `COMMERCE_EVENT_TYPE_SET.has`.
pub fn is_commerce(event_type: &str) -> bool {
    ORDER_STATUS_EVENT_TYPES.contains(&event_type)
        || COMMERCE_EXTRA_EVENT_TYPES.contains(&event_type)
}

/// `PIPELINE_EVENT_TYPE_SET.has`.
pub fn is_pipeline(event_type: &str) -> bool {
    PIPELINE_EVENT_TYPES.contains(&event_type)
}

/// `CONTEXT_FREE_EVENT_TYPE_SET.has`: commerce and pipeline events reach
/// webhooks outside a channel-originated context.
pub fn is_context_free(event_type: &str) -> bool {
    is_commerce(event_type) || is_pipeline(event_type)
}
