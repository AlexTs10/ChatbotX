//! `pipelines.listComments`, `addComment`, `deleteComment`, `addAttachment`,
//! `removeAttachment`, `listActivity` (`api/public-items.ts`).

use crate::orpc::OrpcError;

/// `listActivityPublicRequest.parse(...).groups`: `None` = every kind.
pub fn activity_groups_query(
    _params: &[(&'static str, String)],
    _query: Option<&str>,
) -> Result<Option<Vec<String>>, OrpcError> {
    todo!("P3.6-D1p")
}
