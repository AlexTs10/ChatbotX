//! `features/contacts/api/public/messages.ts` (`contactsMessagesPublicRouter`).

use super::{ApiState, Call, Reply};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "contacts.sendMessage",
    "contacts.listMessages",
    "contacts.getMessage",
    "contacts.triggerAutoReply",
    "contacts.sendFlow",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D2 contact messages stage 2")
}
