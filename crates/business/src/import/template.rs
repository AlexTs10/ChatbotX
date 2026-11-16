//! builder `features/contacts/lib/contacts-import-template.ts`: the CSV
//! template of `contacts.getImportTemplate` (labels of the `en` / `vi`
//! message catalogs).

/// `CONTACTS_IMPORT_TEMPLATE_FILENAME`.
pub const CONTACTS_IMPORT_TEMPLATE_FILENAME: &str = "contacts-import-template.csv";

/// `CONTACTS_IMPORT_TEMPLATE_COLUMNS`.
pub const CONTACTS_IMPORT_TEMPLATE_COLUMNS: [&str; 6] = [
    "contactId",
    "phoneNumber",
    "email",
    "firstName",
    "lastName",
    "sourceUserId",
];

/// `buildContactsImportTemplateCsv(language)`: BOM, the quoted header row
/// and one example row (`vi` / `vi-*` = Vietnamese, else English).
pub fn build_contacts_import_template_csv(language: &str) -> String {
    let _ = language;
    todo!("P3.6-D2")
}
