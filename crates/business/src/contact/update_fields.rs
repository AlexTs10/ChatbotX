//! `contact/update-fields.ts` `updateFieldsAndCustomFields`.

use chatbotx_core::variables::system_field::normalize_stored_timezone;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};

use super::create::repo_err;
use super::service::{find_or_fail, now_ms, update_text_columns};
use crate::contact_custom_field::set_values_in_tx;
use crate::{Result, tx};

/// `fillableContactKeys`.
const FILLABLE_CONTACT_KEYS: &[&str] = &[
    "phoneNumber",
    "email",
    "firstName",
    "lastName",
    "gender",
    "timezone",
];

/// `supportedContactLanguages`.
const SUPPORTED_CONTACT_LANGUAGES: &[&str] = &[
    "en", "vi", "th", "id", "ms", "km", "lo", "my", "fil", "ja", "ko", "zh", "fr", "de", "es",
    "it", "pt", "nl", "ru", "ar", "hi",
];

/// `contact-locale` `normalizeLanguage` (`languageFromLocale` + the
/// supported list).
pub fn normalize_language(language: Option<&str>) -> Option<String> {
    let trimmed = language?.trim();
    if trimmed.is_empty() {
        return None;
    }
    let first = trimmed.split(['-', '_']).next()?.to_lowercase();
    SUPPORTED_CONTACT_LANGUAGES
        .contains(&first.as_str())
        .then_some(first)
}

/// `isNumericId`.
fn numeric_id(key: &str) -> Option<i64> {
    if key.is_empty() || key.len() > 18 || !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    key.parse().ok()
}

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
    tx::write(db, move |tx| {
        let existing = find_or_fail(tx, workspace_id, id)?;
        let lookup = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        let contact_inbox_id = lookup("contactInboxId");
        let client_timezone = lookup("clientTimezone");
        let language = normalize_language(lookup("language").as_deref());
        let mut submitted: Vec<(&'static str, String)> = Vec::new();
        let mut candidates: Vec<(i64, String)> = Vec::new();
        for (key, value) in &fields {
            if matches!(key.as_str(), "contactInboxId" | "language" | "clientTimezone") {
                continue;
            }
            if let Some(column) = FILLABLE_CONTACT_KEYS.iter().find(|k| **k == key.as_str()) {
                // `assignContactFieldValue`.
                let value = match *column {
                    "gender" if !matches!(value.as_str(), "male" | "female" | "unknown") => continue,
                    "timezone" => normalize_stored_timezone(Some(value)).unwrap_or_else(|| value.clone()),
                    _ => value.clone(),
                };
                submitted.retain(|(c, _)| c != column);
                submitted.push((column, value));
            } else if let Some(field_id) = numeric_id(key) {
                candidates.retain(|(c, _)| *c != field_id);
                candidates.push((field_id, value.clone()));
            }
        }
        let current = |column: &str| -> Option<String> {
            match column {
                "phoneNumber" => existing.phone_number.clone(),
                "email" => existing.email.clone(),
                "firstName" => existing.first_name.clone(),
                "lastName" => existing.last_name.clone(),
                "gender" => existing.gender.clone(),
                "timezone" => existing.timezone.clone(),
                _ => None,
            }
        };
        let changed: Vec<(&str, Option<String>)> = submitted
            .into_iter()
            .filter(|(column, value)| current(column).as_deref() != Some(value.as_str()))
            .map(|(column, value)| (column, Some(value)))
            .collect();
        // Only this workspace's custom fields.
        let mut custom = Vec::new();
        for (field_id, value) in candidates {
            let known = tx
                .query_row(
                    r#"SELECT 1 FROM "CustomField" WHERE "workspaceId" = ?1 AND "id" = ?2"#,
                    params![workspace_id, field_id],
                    |_| Ok(()),
                )
                .optional()
                .map_err(repo_err)?
                .is_some();
            if known {
                custom.push((field_id, value));
            }
        }
        let mut language_update = None;
        if let (Some(contact_inbox_id), Some(language)) =
            (contact_inbox_id.as_deref().and_then(numeric_id), language)
        {
            let stored: Option<Option<String>> = tx
                .query_row(
                    r#"SELECT "language" FROM "ContactInbox" WHERE "id" = ?1 AND "contactId" = ?2"#,
                    params![contact_inbox_id, id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(repo_err)?;
            if let Some(stored) = stored
                && stored.as_deref() != Some(language.as_str())
            {
                language_update = Some((contact_inbox_id, language));
            }
        }
        let mut outcome = UpdateOutcome::default();
        if changed.is_empty() && language_update.is_none() && custom.is_empty() {
            return Ok(outcome);
        }
        if !changed.is_empty() {
            update_text_columns(tx, workspace_id, id, &changed)?;
            outcome.contact_fields = changed.iter().map(|(c, _)| (*c).to_string()).collect();
        }
        if let Some((contact_inbox_id, language)) = language_update {
            tx.execute(
                r#"UPDATE "ContactInbox" SET "language" = ?3, "updatedAt" = ?4 WHERE "id" = ?1 AND "contactId" = ?2"#,
                params![contact_inbox_id, id, language, now_ms()],
            )
            .map_err(repo_err)?;
            outcome.language_updated = true;
        }
        if !custom.is_empty() {
            outcome.custom_field_changes =
                set_values_in_tx(tx, workspace_id, id, &custom, client_timezone.as_deref(), now_ms())?;
        }
        Ok(outcome)
    })
    .await
}
