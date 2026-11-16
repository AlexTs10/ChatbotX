//! `features/contacts/api/public/bulk.ts` (`contactsBulkPublicRouter`,
//! P3.6-D2 slice contacts bulk).

use super::{ApiState, Call, Reply};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "contacts.bulkAddTags",
    "contacts.bulkRemoveTags",
    "contacts.bulkTagByStats",
    "contacts.bulkDelete",
    "contacts.bulkSubscribeSequences",
    "contacts.bulkUnsubscribeSequences",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D2 contacts bulk stage 2")
}
