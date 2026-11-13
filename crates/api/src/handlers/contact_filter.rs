//! `apps/builder/src/features/contact-filter/api/public.ts`
//! (`contactsFilterFieldsPublicRouter`, P3.6-D2 slice filter fields):
//! `contacts.listFilterFields` and `contacts.resolveFilterValueLabels`.

use super::{ApiState, Call, Reply};

/// The ported routes.
pub const ROUTES: &[&str] = &["contacts.listFilterFields", "contacts.resolveFilterValueLabels"];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D2 filter fields stage 2")
}
