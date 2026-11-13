//! `apps/builder/src/features/contacts/api/public/custom-fields.ts`
//! (`contactsCustomFieldsPublicRouter`, P3.6-D2 slice contact custom-field
//! values).

use super::{ApiState, Call, Reply};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "contacts.listCustomFields",
    "contacts.getCustomField",
    "contacts.setCustomField",
    "contacts.setCustomFieldLegacy",
    "contacts.applyCustomFieldOperations",
    "contacts.clearCustomField",
    "contacts.clearCustomFields",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D2")
}
