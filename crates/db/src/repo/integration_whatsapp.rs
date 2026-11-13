//! Port of `repositories/integration-whatsapp/repository.ts`
//! (`integrationWhatsappRepository`, P3.2b).

use rusqlite::Connection;

use super::Result;

/// `{ id, inboxId }` of `listByDisplayPhoneDigits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntegrationWhatsappInboxRef {
    pub id: i64,
    pub inbox_id: i64,
}

/// `listByDisplayPhoneDigits`: integrations of the workspace whose display phone number,
/// digits only, equals `digits` (at most two; an empty `digits` matches nothing).
pub fn list_by_display_phone_digits(
    _conn: &Connection,
    _workspace_id: i64,
    _digits: &str,
) -> Result<Vec<IntegrationWhatsappInboxRef>> {
    todo!("P3.2b")
}
