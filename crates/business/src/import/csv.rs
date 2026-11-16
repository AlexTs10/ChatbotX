//! The CSV reading of the imports (`csv-parse` with `columns: true`,
//! `skip_empty_lines`, `trim`, `bom`) and the CSV writing helpers
//! (`@chatbotx.io/utils/csv`).

/// `UTF8_BOM`.
pub const UTF8_BOM: &str = "\u{feff}";

/// A CSV syntax error (`csv-parse` throws; the callers map it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsvError(pub String);

/// The records of `text` (BOM stripped, fields trimmed, empty lines
/// skipped; quoted fields keep their inner spaces).
pub fn parse_records(text: &str) -> Result<Vec<Vec<String>>, CsvError> {
    let _ = text;
    todo!("P3.6-D2")
}

/// `quoteCsvField`: always quoted, `"` doubled.
pub fn quote_field(value: &str) -> String {
    let _ = value;
    todo!("P3.6-D2")
}
