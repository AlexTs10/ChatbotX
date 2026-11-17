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
    let base = language.split(['-', '_']).next().unwrap_or_default();
    let (labels, example): ([&str; 6], [&str; 6]) = if base.eq_ignore_ascii_case("vi") {
        (
            [
                "ID Liên hệ",
                "Số điện thoại",
                "Email",
                "Tên",
                "Họ",
                "ID người dùng WhatsApp",
            ],
            [
                "1234567890",
                "+84155550100",
                "an.nguyen@example.com",
                "An",
                "Nguyễn",
                "user.9187654321098765",
            ],
        )
    } else {
        (
            [
                "Contact ID",
                "Phone number",
                "Email",
                "First name",
                "Last name",
                "WhatsApp User ID",
            ],
            [
                "1234567890",
                "+14155550100",
                "john.doe@example.com",
                "John",
                "Doe",
                "user.9187654321098765",
            ],
        )
    };
    let row = |cells: [&str; 6]| {
        cells
            .iter()
            .map(|cell| super::csv::quote_field(cell))
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "{}{}\n{}\n",
        super::csv::UTF8_BOM,
        row(labels),
        row(example)
    )
}
