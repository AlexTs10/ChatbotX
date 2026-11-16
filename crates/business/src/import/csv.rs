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
    let text = text.strip_prefix(UTF8_BOM).unwrap_or(text);
    let chars: Vec<char> = text.chars().collect();
    let mut records = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        // `skip_empty_lines`: a line with no characters at all.
        if chars[index] == '\n' {
            index += 1;
            continue;
        }
        if chars[index] == '\r' {
            index += 1;
            if chars.get(index) == Some(&'\n') {
                index += 1;
            }
            continue;
        }
        let mut record = Vec::new();
        loop {
            let (field, next) = parse_field(&chars, index)?;
            record.push(field);
            index = next;
            match chars.get(index) {
                Some(',') => index += 1,
                Some('\r') => {
                    index += 1;
                    if chars.get(index) == Some(&'\n') {
                        index += 1;
                    }
                    break;
                }
                Some('\n') => {
                    index += 1;
                    break;
                }
                None => break,
                Some(other) => {
                    return Err(CsvError(format!("unexpected character {other:?}")));
                }
            }
        }
        records.push(record);
    }
    Ok(records)
}

fn is_blank(ch: char) -> bool {
    ch == ' ' || ch == '\t'
}

/// One field from `start`; returns it and the index of the delimiter /
/// line end after it. `trim`: blanks around the field are dropped.
fn parse_field(chars: &[char], start: usize) -> Result<(String, usize), CsvError> {
    let mut index = start;
    while chars.get(index).copied().is_some_and(is_blank) {
        index += 1;
    }
    let mut out = String::new();
    if chars.get(index) == Some(&'"') {
        index += 1;
        loop {
            match chars.get(index) {
                None => return Err(CsvError("quote not closed".into())),
                Some('"') if chars.get(index + 1) == Some(&'"') => {
                    out.push('"');
                    index += 2;
                }
                Some('"') => {
                    index += 1;
                    break;
                }
                Some(ch) => {
                    out.push(*ch);
                    index += 1;
                }
            }
        }
        while chars.get(index).copied().is_some_and(is_blank) {
            index += 1;
        }
        return Ok((out, index));
    }
    while let Some(ch) = chars.get(index) {
        if matches!(ch, ',' | '\n' | '\r') {
            break;
        }
        out.push(*ch);
        index += 1;
    }
    Ok((out.trim_end_matches(is_blank).to_string(), index))
}

/// `quoteCsvField`: always quoted, `"` doubled.
pub fn quote_field(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
