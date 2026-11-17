//! Ports (P3.6-D2 slice contact imports, part one):
//!
//! - `packages/business/__tests__/import-column-map.test.ts`, the
//!   `suggestContactImportColumnMap` case (1, ported). The three
//!   `resolveProductImportColumnMap` cases are in `import_product_column_map.rs`
//!   (Q.D1i).
//! - `apps/builder/__tests__/contacts-import-template.test.ts` (6, ported).

use chatbotx_business::import::csv::UTF8_BOM;
use chatbotx_business::import::suggest_contact_import_column_map;
use chatbotx_business::import::template::{
    CONTACTS_IMPORT_TEMPLATE_COLUMNS, CONTACTS_IMPORT_TEMPLATE_FILENAME,
    build_contacts_import_template_csv,
};

fn headers(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn suggest_contact_import_column_map_matches_the_builder_s_contact_headers() {
    assert_eq!(
        suggest_contact_import_column_map(&headers(&["Phone", "Email", "First name"])),
        vec![
            ("phoneNumber", "Phone".to_string()),
            ("email", "Email".to_string()),
            ("firstName", "First name".to_string()),
        ]
    );
}

const EN_TEMPLATE: &str = concat!(
    "\"Contact ID\",\"Phone number\",\"Email\",\"First name\",\"Last name\",\"WhatsApp User ID\"\n",
    "\"1234567890\",\"+14155550100\",\"john.doe@example.com\",\"John\",\"Doe\",\"user.9187654321098765\"\n",
);
const VI_TEMPLATE: &str = concat!(
    "\"ID Liên hệ\",\"Số điện thoại\",\"Email\",\"Tên\",\"Họ\",\"ID người dùng WhatsApp\"\n",
    "\"1234567890\",\"+84155550100\",\"an.nguyen@example.com\",\"An\",\"Nguyễn\",\"user.9187654321098765\"\n",
);

fn body(csv: &str) -> &str {
    csv.strip_prefix(UTF8_BOM).expect("BOM")
}

#[test]
fn build_contacts_import_template_csv_returns_quoted_english_headers_and_an_example_row_with_a_utf_8_bom_for_the_en_language()
 {
    let csv = build_contacts_import_template_csv("en");
    assert!(csv.starts_with(UTF8_BOM));
    assert_eq!(body(&csv), EN_TEMPLATE);
}

#[test]
fn build_contacts_import_template_csv_returns_quoted_vietnamese_headers_and_example_row_for_the_vi_language()
 {
    assert_eq!(body(&build_contacts_import_template_csv("vi")), VI_TEMPLATE);
}

#[test]
fn build_contacts_import_template_csv_falls_back_to_the_english_template_for_any_language_other_than_vi()
 {
    assert_eq!(body(&build_contacts_import_template_csv("fr")), EN_TEMPLATE);
    assert_eq!(body(&build_contacts_import_template_csv("")), EN_TEMPLATE);
}

#[test]
fn build_contacts_import_template_csv_resolves_regional_variants_to_their_base_language() {
    assert_eq!(
        body(&build_contacts_import_template_csv("vi-VN")),
        VI_TEMPLATE
    );
}

#[test]
fn build_contacts_import_template_csv_emits_a_header_row_and_exactly_one_example_data_row() {
    let csv = build_contacts_import_template_csv("en");
    let rows: Vec<&str> = body(&csv).trim_end().split('\n').collect();
    assert_eq!(
        CONTACTS_IMPORT_TEMPLATE_COLUMNS,
        [
            "contactId",
            "phoneNumber",
            "email",
            "firstName",
            "lastName",
            "sourceUserId"
        ]
    );
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(
            row.split(',').count(),
            CONTACTS_IMPORT_TEMPLATE_COLUMNS.len()
        );
    }
}

#[test]
fn build_contacts_import_template_csv_exposes_a_stable_download_filename() {
    assert_eq!(
        CONTACTS_IMPORT_TEMPLATE_FILENAME,
        "contacts-import-template.csv"
    );
}
