//! Port of `repositories/contact/list-where.ts`: the WHERE and ORDER BY of the contact list.

use rusqlite::types::Value;

use super::queries::contact_filter::{
    ContactInboxScope, FilterCriteria, SmartKeywordOptions, apply_contact_filter,
    build_contact_inbox_scope_where, build_smart_keyword_where,
    prune_email_phone_filter_conditions,
};
use super::sql::Sql;
use super::utils::{SortDir, SortItem, parse_order_by};

/// The sortable `Contact` columns (`parseOrderByAsObject(contactModel, ..)`).
const CONTACT_COLUMNS: &[&str] = &[
    "id",
    "createdAt",
    "updatedAt",
    "avatar",
    "phoneNumber",
    "email",
    "emailVerified",
    "emailOptIn",
    "firstName",
    "lastName",
    "fullName",
    "gender",
    "lastReadAt",
    "ref",
    "country",
    "state",
    "city",
    "location",
    "locale",
    "timezone",
    "subscribedAt",
    "broadcastSubscribedAt",
    "blockedAt",
    "totalTagged",
    "totalNewTagged",
    "workspaceId",
];

/// `DEFAULT_CONTACT_ORDER_BY`.
pub const DEFAULT_CONTACT_ORDER_BY: (&str, SortDir) = ("createdAt", SortDir::Desc);

/// `resolveContactOrderBy`: the valid sort columns of `Contact`, else `createdAt desc`.
pub fn resolve_contact_order_by(sort: Option<&[SortItem]>) -> Vec<(String, SortDir)> {
    let order = parse_order_by(CONTACT_COLUMNS, sort, None);
    if order.is_empty() {
        vec![(
            DEFAULT_CONTACT_ORDER_BY.0.to_owned(),
            DEFAULT_CONTACT_ORDER_BY.1,
        )]
    } else {
        order
    }
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
    let contact_filter = prune_email_phone_filter_conditions(
        input.contact_filter.as_ref(),
        input.include_email_and_phone,
    );
    let options = SmartKeywordOptions {
        include_email_and_phone: input.include_email_and_phone,
        ..SmartKeywordOptions::default()
    };
    let filters: Vec<Sql> = [
        input
            .keyword
            .as_deref()
            .filter(|k| !k.is_empty())
            .and_then(|k| build_smart_keyword_where(k, options)),
        contact_filter
            .as_ref()
            .and_then(|f| apply_contact_filter(f, Some(input.workspace_id), input.now)),
        input
            .inbox_scope
            .as_ref()
            .map(|s| build_contact_inbox_scope_where(s, input.now)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut parts = vec![Some(Sql::bind(
        "\"Contact\".\"workspaceId\" = ",
        input.workspace_id,
    ))];
    match filters.len() {
        0 => {}
        1 => parts.extend(filters.into_iter().map(Some)),
        _ => parts.push(Some(Sql::and(filters.into_iter().map(Some)))),
    }
    let mut scopes = Vec::new();
    if let Some(user) = input.restrict_to_assigned_user_id {
        scopes.push(Some(Sql::bind(
            "\"Conversation\".\"assignedUserId\" = ",
            user,
        )));
    }
    if !input.restrict_to_team_ids.is_empty() {
        let marks = vec!["?"; input.restrict_to_team_ids.len()].join(", ");
        scopes.push(Some(Sql::new(
            format!("\"Conversation\".\"assignedInboxTeamId\" IN ({marks})"),
            input
                .restrict_to_team_ids
                .iter()
                .copied()
                .map(Value::Integer)
                .collect(),
        )));
    }
    if !scopes.is_empty() {
        let mut exists = Sql::raw(
            "EXISTS (SELECT 1 FROM \"Conversation\" WHERE \"Conversation\".\"contactId\" = \"Contact\".\"id\" AND (",
        );
        exists.append(Sql::or(scopes));
        parts.push(Some(exists.closed("))")));
    }
    Sql::and(parts)
}
