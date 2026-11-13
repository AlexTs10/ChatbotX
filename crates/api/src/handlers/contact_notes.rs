//! `apps/builder/src/features/contact-notes/api/public.ts`
//! (`contactsNotesPublicRouter`).

use super::{ApiState, Call, Reply};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "contacts.listNotes",
    "contacts.createNote",
    "contacts.updateNote",
    "contacts.deleteNote",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D2")
}
