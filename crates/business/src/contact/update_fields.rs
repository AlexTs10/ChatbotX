//! `contact/update-fields.ts` `updateFieldsAndCustomFields`.

use chatbotx_db::Db;

use crate::Result;

/// One stored custom field change (`PendingContactCustomFieldChange`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomFieldChange {
    pub custom_field_id: i64,
    pub custom_field_name: String,
    pub old_value: Option<String>,
    pub new_value: String,
}

/// What the update wrote (the original's audit / event decisions).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateOutcome {
    /// The changed standard fields (`contactFields` keys).
    pub contact_fields: Vec<String>,
    /// The contact inbox language was changed.
    pub language_updated: bool,
    pub custom_field_changes: Vec<CustomFieldChange>,
}

impl UpdateOutcome {
    /// `hasRealChange` (the audit record condition).
    pub fn has_real_change(&self) -> bool {
        !self.contact_fields.is_empty()
            || self.language_updated
            || !self.custom_field_changes.is_empty()
    }
}

/// `updateFieldsAndCustomFields`: `fields` in input order (a JSON object of
/// strings).
pub async fn update_fields_and_custom_fields(
    db: &Db,
    workspace_id: i64,
    id: i64,
    fields: Vec<(String, String)>,
) -> Result<UpdateOutcome> {
    let _ = (db, workspace_id, id, fields);
    todo!("P3.6-D2")
}
