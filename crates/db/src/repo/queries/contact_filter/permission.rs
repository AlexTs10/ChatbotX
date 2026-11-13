//! Port of `queries/contact-filter/permission.ts`: email / phone field pruning.

use serde_json::Value as Json;

use super::{CriteriaOperator, FilterCriteria};

/// The string `field` of an object condition.
fn condition_field(c: &Json) -> Option<&str> {
    c.get("field").and_then(Json::as_str)
}

/// `EMAIL_PHONE_FILTER_FIELDS`.
pub const EMAIL_PHONE_FILTER_FIELDS: &[&str] = &[
    "email",
    "phone",
    "hasContactInfo",
    "emailWasVerified",
    "optedInForEmail",
    "existingContact",
];

/// `contactFilterUsesField`.
pub fn contact_filter_uses_field(contact_filter: Option<&FilterCriteria>, field: &str) -> bool {
    contact_filter.is_some_and(|f| {
        f.conditions
            .iter()
            .any(|c| condition_field(c) == Some(field))
    })
}

/// `pruneContactFilterFields`: drops the conditions on `excluded_fields`; keeps the other
/// criteria fields (`timezone`); an emptied filter becomes `and`.
pub fn prune_contact_filter_fields(
    contact_filter: Option<&FilterCriteria>,
    excluded_fields: &[&str],
) -> Option<FilterCriteria> {
    let filter = contact_filter?;
    if excluded_fields.is_empty() || filter.conditions.is_empty() {
        return Some(filter.clone());
    }
    let conditions: Vec<Json> = filter
        .conditions
        .iter()
        .filter(|c| condition_field(c).is_none_or(|f| !excluded_fields.contains(&f)))
        .cloned()
        .collect();
    Some(FilterCriteria {
        operator: if conditions.is_empty() {
            CriteriaOperator::And
        } else {
            filter.operator
        },
        conditions,
        ..filter.clone()
    })
}

/// `pruneEmailPhoneFilterConditions`.
pub fn prune_email_phone_filter_conditions(
    contact_filter: Option<&FilterCriteria>,
    can_view_email_and_phone: bool,
) -> Option<FilterCriteria> {
    if can_view_email_and_phone {
        contact_filter.cloned()
    } else {
        prune_contact_filter_fields(contact_filter, EMAIL_PHONE_FILTER_FIELDS)
    }
}
