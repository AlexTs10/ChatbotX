//! `apps/builder/src/features/custom-fields/api/public.ts`
//! (`customFieldsPublicRouter`).

use super::{ApiState, Call, Reply};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "customFields.list",
    "customFields.create",
    "customFields.get",
    "customFields.update",
    "customFields.delete",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D2")
}
