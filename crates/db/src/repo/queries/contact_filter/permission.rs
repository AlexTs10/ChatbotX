//! Port of `queries/contact-filter/permission.ts`: email / phone field pruning.

use super::FilterCriteria;

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
    let _ = (contact_filter, field);
    todo!("P3.2b")
}

/// `pruneContactFilterFields`: drops the conditions on `excluded_fields`; keeps the other
/// criteria fields (`timezone`); an emptied filter becomes `and`.
pub fn prune_contact_filter_fields(
    contact_filter: Option<&FilterCriteria>,
    excluded_fields: &[&str],
) -> Option<FilterCriteria> {
    let _ = (contact_filter, excluded_fields);
    todo!("P3.2b")
}

/// `pruneEmailPhoneFilterConditions`.
pub fn prune_email_phone_filter_conditions(
    contact_filter: Option<&FilterCriteria>,
    can_view_email_and_phone: bool,
) -> Option<FilterCriteria> {
    let _ = (contact_filter, can_view_email_and_phone);
    todo!("P3.2b")
}
