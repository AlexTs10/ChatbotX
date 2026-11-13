//! Port of `repositories/contact/list-where.ts`: the WHERE and ORDER BY of the contact list.

use super::queries::contact_filter::{ContactInboxScope, FilterCriteria};
use super::sql::Sql;
use super::utils::{SortDir, SortItem};

/// `DEFAULT_CONTACT_ORDER_BY`.
pub const DEFAULT_CONTACT_ORDER_BY: (&str, SortDir) = ("createdAt", SortDir::Desc);

/// `resolveContactOrderBy`: the valid sort columns of `Contact`, else `createdAt desc`.
pub fn resolve_contact_order_by(sort: Option<&[SortItem]>) -> Vec<(String, SortDir)> {
    let _ = sort;
    todo!("P3.2b")
}

/// `ListWhereInput` (+ `now`, unix ms).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListWhereInput {
    pub workspace_id: i64,
    pub keyword: Option<String>,
    pub contact_filter: Option<FilterCriteria>,
    pub inbox_scope: Option<ContactInboxScope>,
    pub restrict_to_assigned_user_id: Option<i64>,
    pub restrict_to_team_ids: Vec<i64>,
    pub include_email_and_phone: bool,
    pub now: i64,
}

/// `buildContactListWhere`: workspace AND keyword AND (pruned) filter AND inbox scope, AND
/// the conversation assignment scope (a conversation assigned to the user or one of the
/// teams).
pub fn build_contact_list_where(input: &ListWhereInput) -> Sql {
    let _ = input;
    todo!("P3.2b")
}
