//! `features/import/api/public.ts` (`importPublicRouter`, P3.6-D2 slice
//! contact imports, part one).

use super::{ApiState, Call, Reply};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "contacts.createImportUpload",
    "contacts.getImportTemplate",
    "contacts.peekImportHeaders",
    "contacts.listImports",
    "contacts.getImport",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D2 contact imports stage 2")
}
