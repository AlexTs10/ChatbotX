//! `features/folders/api/public.ts` (P3.6-D2 slice folders):
//! `folders.list / create / update / delete`, limited to the contacts-owned
//! folder types (`tag`, `customField`).

use super::{ApiState, Call, Reply};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "folders.list",
    "folders.create",
    "folders.update",
    "folders.delete",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D2")
}
