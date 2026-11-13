//! `features/contacts/api/public/crud.ts` (P3.6-D2 slice contacts core):
//! `contacts.get / create / update / updateLegacy / delete / block /
//! unblock / upsert`. The list, count, search, import and sub-resource
//! routes are not ported yet.

use super::{ApiState, Call, Reply};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "contacts.get",
    "contacts.create",
    "contacts.update",
    "contacts.updateLegacy",
    "contacts.delete",
    "contacts.block",
    "contacts.unblock",
    "contacts.upsert",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D2")
}
