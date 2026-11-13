//! `product-message/resolve-message.ts`: the content of a `sendProducts` step
//! (JSON, `SendProductsContent`) against the sendable items (`ProductMessageItem`
//! JSON keyed by product id) and the channel's `productMessages` capability.

use std::collections::HashMap;

use serde_json::Value;

/// `ProductMessageResolution`.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// `{ ok: true, message, missing }` (`missing` = product ids).
    Ok {
        message: Value,
        missing: Vec<String>,
    },
    /// `{ ok: false, reason }`: `noProductsSendable` or `invalidMessage`.
    Failed(&'static str),
}

/// `resolveProductMessage`.
pub fn resolve_product_message(
    capability: &Value,
    content: &Value,
    sendable_by_id: &HashMap<String, Value>,
) -> Resolution {
    let _ = (capability, content, sendable_by_id);
    todo!("P3.6-D1")
}
