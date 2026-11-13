//! `apps/builder/src/features/bot-fields/api/public.ts`
//! (`botFieldsPublicRouter`, P3.6-D2 slice bot fields).

use super::{ApiState, Call, Reply};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "botFields.list",
    "botFields.create",
    "botFields.get",
    "botFields.set",
    "botFields.update",
    "botFields.setMany",
    "botFields.bulkUpdate",
    "botFields.applyOperation",
    "botFields.reset",
    "botFields.resetMany",
    "botFields.delete",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D2")
}
