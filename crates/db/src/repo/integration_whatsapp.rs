//! Port of `repositories/integration-whatsapp/repository.ts`
//! (`integrationWhatsappRepository`, P3.2b).

use rusqlite::Connection;
use rusqlite::types::Value;

use super::Result;
use super::sql::Sql;

/// `{ id, inboxId }` of `listByDisplayPhoneDigits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntegrationWhatsappInboxRef {
    pub id: i64,
    pub inbox_id: i64,
}

/// `listByDisplayPhoneDigits`: integrations of the workspace whose display phone number,
/// digits only, equals `digits` (at most two; an empty `digits` matches nothing).
pub fn list_by_display_phone_digits(
    conn: &Connection,
    workspace_id: i64,
    digits: &str,
) -> Result<Vec<IntegrationWhatsappInboxRef>> {
    if digits.is_empty() {
        return Ok(Vec::new());
    }
    Sql::new(
        "SELECT \"id\", \"inboxId\" FROM \"IntegrationWhatsapp\" WHERE \"workspaceId\" = ? \
         AND regexp_replace(\"displayPhoneNumber\", '\\D', '', 'g') = ? ORDER BY \"id\" LIMIT 2",
        vec![Value::Integer(workspace_id), Value::Text(digits.to_owned())],
    )
    .query_map(conn, |r| {
        Ok(IntegrationWhatsappInboxRef {
            id: r.get(0)?,
            inbox_id: r.get(1)?,
        })
    })
}
