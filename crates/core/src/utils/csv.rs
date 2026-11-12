//! `packages/utils/src/csv.ts`.

/// UTF-8 byte-order mark, written once before the header row.
pub const UTF8_BOM: &str = "\u{FEFF}";

/// One CSV cell value (`string | number | null | undefined`).
#[derive(Debug, Clone, PartialEq)]
pub enum CsvCell {
    Text(String),
    Number(f64),
    Empty,
}

impl From<&str> for CsvCell {
    fn from(value: &str) -> CsvCell {
        CsvCell::Text(value.to_owned())
    }
}

impl From<String> for CsvCell {
    fn from(value: String) -> CsvCell {
        CsvCell::Text(value)
    }
}

impl From<f64> for CsvCell {
    fn from(value: f64) -> CsvCell {
        CsvCell::Number(value)
    }
}

impl From<i64> for CsvCell {
    fn from(value: i64) -> CsvCell {
        CsvCell::Number(value as f64)
    }
}

impl From<i32> for CsvCell {
    fn from(value: i32) -> CsvCell {
        CsvCell::Number(f64::from(value))
    }
}

impl<T: Into<CsvCell>> From<Option<T>> for CsvCell {
    fn from(value: Option<T>) -> CsvCell {
        value.map_or(CsvCell::Empty, Into::into)
    }
}

/// The CSV dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CsvDialect {
    /// Every field quoted, line breaks flattened to a space.
    #[default]
    Always,
    /// RFC 4180: quoted only when needed, line breaks kept.
    Minimal,
}

/// `quoteCsvField`: always quotes, doubles embedded quotes (trusted text).
pub fn quote_csv_field(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\"\""))
}

fn flatten_line_breaks(text: &str) -> String {
    text.replace("\r\n", " ").replace('\n', " ")
}

/// `escapeCsvCell`: an untrusted value as one field; a leading formula
/// character (`= + - @ \t \r`) is prefixed with `'`.
pub fn escape_csv_cell(value: &CsvCell, dialect: CsvDialect) -> String {
    let raw = match value {
        CsvCell::Text(text) => text.clone(),
        CsvCell::Number(n) => crate::js::number_to_string(*n),
        CsvCell::Empty => String::new(),
    };
    let text = match dialect {
        CsvDialect::Always => flatten_line_breaks(&raw),
        CsvDialect::Minimal => raw,
    };
    let guarded = if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{text}")
    } else {
        text
    };
    let needs_quoting = guarded.contains(['"', ',', '\r', '\n']);
    if dialect == CsvDialect::Minimal && !needs_quoting {
        guarded
    } else {
        quote_csv_field(&guarded)
    }
}

/// `toCsvRow`: escaped cells joined with commas, newline-terminated.
pub fn to_csv_row(cells: &[CsvCell], dialect: CsvDialect) -> String {
    let joined: Vec<String> = cells.iter().map(|c| escape_csv_cell(c, dialect)).collect();
    format!("{}\n", joined.join(","))
}
