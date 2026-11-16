//! `features/questionnaires/api/public.ts`: `questionnaires.*`.

use super::{ApiState, Call, Reply};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "questionnaires.list",
    "questionnaires.create",
    "questionnaires.get",
    "questionnaires.update",
    "questionnaires.delete",
    "questionnaires.rename",
    "questionnaires.duplicate",
    "questionnaires.listSubmissions",
    "questionnaires.getSubmission",
    "questionnaires.deleteSubmission",
    "questionnaires.getSubmissionStats",
];

/// Runs one of [`ROUTES`].
pub async fn handle(_state: &ApiState, _route_id: &str, _call: Call<'_>) -> Reply {
    todo!("P3.6-D1")
}
