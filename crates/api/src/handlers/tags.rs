//! `apps/builder/src/features/tags/api/public.ts` (`tagsPublicRouter`) and
//! the tag routes of `features/contacts/api/public/tags.ts`.

use super::{ApiState, Call, Reply};

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "tags.list",
    "tags.create",
    "tags.get",
    "tags.update",
    "tags.delete",
    "contacts.listTags",
    "contacts.addTags",
    "contacts.removeTags",
    "contacts.setTags",
    "contacts.addTagsByName",
    "contacts.removeTagsByName",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D2")
}
