//! `contact-inbox-context.ts`: the `contactInboxId` key of trigger metadata.

use serde_json::{Map, Value};

/// `CONTACT_INBOX_METADATA_KEY`.
pub const CONTACT_INBOX_METADATA_KEY: &str = "contactInboxId";

/// `withContactInboxMetadata`: `metadata` plus `contactInboxId` when one is
/// given (an empty id counts as none); else `metadata` as it is.
pub fn with_contact_inbox_metadata(
    metadata: Option<Map<String, Value>>,
    contact_inbox_id: Option<&str>,
) -> Option<Map<String, Value>> {
    let Some(id) = contact_inbox_id.filter(|id| !id.is_empty()) else {
        return metadata;
    };
    let mut out = metadata.unwrap_or_default();
    out.insert(CONTACT_INBOX_METADATA_KEY.to_string(), Value::from(id));
    Some(out)
}

/// `extractContactInboxId`: the `contactInboxId` of `event_data` when it is
/// a string of digits only (a snowflake id); else `None`.
pub fn extract_contact_inbox_id(event_data: Option<&Map<String, Value>>) -> Option<String> {
    match event_data?.get(CONTACT_INBOX_METADATA_KEY)? {
        Value::String(id) if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => {
            Some(id.clone())
        }
        _ => None,
    }
}
