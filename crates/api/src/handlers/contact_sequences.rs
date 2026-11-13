//! `features/contact-sequences/api/public.ts` (P3.6-D2 slice contact
//! sequences): `contacts.listSequences / subscribeSequences /
//! unsubscribeSequences / setSequences`.

use super::{ApiState, Call, Reply};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "contacts.listSequences",
    "contacts.subscribeSequences",
    "contacts.unsubscribeSequences",
    "contacts.setSequences",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D2")
}
