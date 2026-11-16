//! `features/contact-inboxes/api/public.ts` (`listInboxes`),
//! `features/contacts/api/public/ad-referrals.ts` (`listAdReferrals`) and
//! `features/contacts/api/public/refresh-profile.ts` (`refreshProfile`).

use super::{ApiState, Call, Reply};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "contacts.listInboxes",
    "contacts.listAdReferrals",
    "contacts.refreshProfile",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D2 inboxes / ads / refresh stage 2")
}
