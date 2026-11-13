//! `productMessageService.listSendable` with the Meta catalog source
//! (`catalog-sources.ts`): only WhatsApp sends product messages.

use chatbotx_db::Db;
use serde_json::Value;

use crate::errors::Result;

/// `listSendable` input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListSendable {
    pub workspace_id: i64,
    pub channel: String,
    pub keyword: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// `SendableProductsResult`: `data` items are `{id, name, imageUrl, price, currency}`.
#[derive(Debug, Clone, PartialEq)]
pub struct Sendable {
    pub ready: bool,
    /// `channelUnsupported` or `catalogNotConnected` when not ready.
    pub reason: Option<&'static str>,
    pub data: Vec<Value>,
    pub page_count: i64,
}

/// `listSendable`.
pub async fn list_sendable(db: &Db, input: ListSendable) -> Result<Sendable> {
    let _ = (db, input);
    todo!("P3.6-D1")
}
