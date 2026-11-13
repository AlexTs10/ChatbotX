//! `features/reflinks/api/public.ts`: `reflinks.*` (the chat-widget routes
//! are not ported yet).

use super::{ApiState, Call, Reply};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "reflinks.list",
    "reflinks.create",
    "reflinks.get",
    "reflinks.update",
    "reflinks.delete",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D1")
}
