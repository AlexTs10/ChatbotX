//! `features/pipelines/api/public.ts` + `public-items.ts`: `pipelines.*`.
//! A workspace token acts with `PIPELINE_TOKEN_SCOPE` (bypasses the ACL).

use super::{ApiState, Call, Reply};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "pipelines.list",
    "pipelines.create",
    "pipelines.get",
    "pipelines.update",
    "pipelines.delete",
    "pipelines.duplicate",
    "pipelines.createStage",
    "pipelines.updateStage",
    "pipelines.deleteStage",
    "pipelines.reorderStages",
    "pipelines.listFields",
    "pipelines.createField",
    "pipelines.updateField",
    "pipelines.deleteField",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D1p")
}
