//! The XLSX side of the imports (Q.D1i): `readXlsxHeaders` /
//! `createImportXlsxParser` (`@chatbotx.io/imports/parsers/xlsx`, ExcelJS)
//! and the writer of `createProductImportTemplate`.
//!
//! A workbook is a ZIP of XML parts. The reader takes the first worksheet
//! of `xl/workbook.xml` and gives its cell values as ExcelJS
//! `toImportCellValue` does: a string, a number, a boolean, or nothing.
//! ponytail: a date-formatted number stays a serial number (ExcelJS gives a
//! `Date`); no import field reads a date today.

use std::collections::HashMap;
use std::io::Read as _;

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

/// `MAX_XLSX_ARCHIVE_ENTRIES`.
const MAX_ENTRIES: usize = 10_000;
/// `MAX_UNCOMPRESSED_XLSX_BYTES`.
const MAX_UNCOMPRESSED_BYTES: u64 = 100 * 1024 * 1024;

/// One cell value after `toImportCellValue`.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Empty,
    Text(String),
    Number(f64),
    Bool(bool),
}

impl Cell {
    /// `cellToHeader`: the trimmed text of the value (`""` when empty).
    pub fn header(&self) -> String {
        match self {
            Cell::Empty => String::new(),
            Cell::Text(text) => chatbotx_core::js::trim(text).to_string(),
            Cell::Number(n) => chatbotx_core::js::number_to_string(*n),
            Cell::Bool(b) => b.to_string(),
        }
    }

    /// The JSON value of an imported row cell (`None` = `undefined`).
    pub fn json(&self) -> Option<serde_json::Value> {
        match self {
            Cell::Empty => None,
            Cell::Text(text) => Some(serde_json::Value::String(text.clone())),
            Cell::Number(n) => serde_json::Number::from_f64(*n).map(serde_json::Value::Number),
            Cell::Bool(b) => Some(serde_json::Value::Bool(*b)),
        }
    }
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// The entries of a ZIP archive (`assertXlsxArchiveWithinLimits` + unzip).
fn unzip(bytes: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    let corrupt = || "Can't find end of central directory : is this a zip file ?".to_string();
    // End of central directory: the last `PK\x05\x06` (a comment may follow).
    let eocd = (0..=bytes.len().saturating_sub(22))
        .rev()
        .find(|&at| bytes[at..].starts_with(b"PK\x05\x06"))
        .ok_or_else(corrupt)?;
    let count = usize::from(u16_at(bytes, eocd + 10).ok_or_else(corrupt)?);
    if count > MAX_ENTRIES {
        return Err(format!("XLSX archive entry limit exceeded ({MAX_ENTRIES})"));
    }
    let mut at = u32_at(bytes, eocd + 16).ok_or_else(corrupt)? as usize;
    let mut total: u64 = 0;
    let mut entries = HashMap::new();
    for _ in 0..count {
        if !bytes
            .get(at..)
            .is_some_and(|b| b.starts_with(b"PK\x01\x02"))
        {
            return Err(corrupt());
        }
        let field = |offset: usize| u16_at(bytes, at + offset).map(usize::from);
        let (Some(method), Some(name_len), Some(extra_len), Some(comment_len)) =
            (field(10), field(28), field(30), field(32))
        else {
            return Err(corrupt());
        };
        let compressed = u32_at(bytes, at + 20).ok_or_else(corrupt)? as usize;
        let size = u64::from(u32_at(bytes, at + 24).ok_or_else(corrupt)?);
        let local = u32_at(bytes, at + 42).ok_or_else(corrupt)? as usize;
        let name = bytes.get(at + 46..at + 46 + name_len).ok_or_else(corrupt)?;
        let name = String::from_utf8_lossy(name).into_owned();
        at += 46 + name_len + extra_len + comment_len;
        total += size;
        if total > MAX_UNCOMPRESSED_BYTES {
            return Err(format!(
                "XLSX uncompressed size limit exceeded ({MAX_UNCOMPRESSED_BYTES} bytes)"
            ));
        }
        let (Some(local_name), Some(local_extra)) =
            (u16_at(bytes, local + 26), u16_at(bytes, local + 28))
        else {
            return Err(corrupt());
        };
        let start = local + 30 + usize::from(local_name) + usize::from(local_extra);
        let data = bytes.get(start..start + compressed).ok_or_else(corrupt)?;
        let content = match method {
            0 => data.to_vec(),
            8 => {
                let mut out = Vec::new();
                flate2::read::DeflateDecoder::new(data)
                    .take(size + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| e.to_string())?;
                out
            }
            other => return Err(format!("Unsupported compression method {other}")),
        };
        entries.insert(name, content);
    }
    Ok(entries)
}

/// The value of the attribute `name` of an element.
fn attribute(element: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| {
            quick_xml::escape::unescape(&String::from_utf8_lossy(&a.value))
                .ok()
                .map(|v| v.into_owned())
        })
}

/// The XML text events: a text run or an entity reference.
fn text_of(event: &Event<'_>) -> Option<String> {
    match event {
        Event::Text(text) => text.decode().ok().map(|t| t.into_owned()),
        Event::CData(data) => data.decode().ok().map(|t| t.into_owned()),
        Event::GeneralRef(reference) => {
            let name = reference.decode().ok()?;
            quick_xml::escape::unescape(&format!("&{name};"))
                .ok()
                .map(|t| t.into_owned())
        }
        _ => None,
    }
}

fn xml(entries: &HashMap<String, Vec<u8>>, name: &str) -> Option<String> {
    entries
        .get(name)
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
}

/// The path of the first worksheet (`workbook.worksheets[0]`).
fn first_sheet_path(entries: &HashMap<String, Vec<u8>>) -> Option<String> {
    let workbook = xml(entries, "xl/workbook.xml")?;
    let mut reader = Reader::from_str(&workbook);
    let mut rel_id = None;
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == b"sheet" => {
                rel_id = attribute(&e, b"r:id");
                break;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let rel_id = rel_id?;
    let rels = xml(entries, "xl/_rels/workbook.xml.rels")?;
    let mut reader = Reader::from_str(&rels);
    while let Ok(event) = reader.read_event() {
        match event {
            Event::Start(e) | Event::Empty(e)
                if e.local_name().as_ref() == b"Relationship"
                    && attribute(&e, b"Id").as_deref() == Some(rel_id.as_str()) =>
            {
                let target = attribute(&e, b"Target")?;
                return Some(match target.strip_prefix('/') {
                    Some(absolute) => absolute.to_string(),
                    None => format!("xl/{target}"),
                });
            }
            Event::Eof => break,
            _ => {}
        }
    }
    None
}

/// `xl/sharedStrings.xml`: the text of each `<si>` (rich-text runs joined,
/// phonetic runs left out).
fn shared_strings(entries: &HashMap<String, Vec<u8>>) -> Vec<String> {
    let Some(text) = xml(entries, "xl/sharedStrings.xml") else {
        return Vec::new();
    };
    let mut reader = Reader::from_str(&text);
    let mut out = Vec::new();
    let (mut current, mut in_t, mut in_phonetic) = (String::new(), false, false);
    while let Ok(event) = reader.read_event() {
        match &event {
            Event::Start(e) => match e.local_name().as_ref() {
                b"si" => current.clear(),
                b"t" => in_t = true,
                b"rPh" => in_phonetic = true,
                _ => {}
            },
            Event::End(e) => match e.local_name().as_ref() {
                b"si" => out.push(std::mem::take(&mut current)),
                b"t" => in_t = false,
                b"rPh" => in_phonetic = false,
                _ => {}
            },
            Event::Empty(e) if e.local_name().as_ref() == b"si" => out.push(String::new()),
            Event::Eof => break,
            other => {
                if in_t
                    && !in_phonetic
                    && let Some(text) = text_of(other)
                {
                    current.push_str(&text);
                }
            }
        }
    }
    out
}

/// The zero-based column of a cell reference (`"C7"` = 2).
fn column_of(reference: &str) -> Option<usize> {
    let letters: Vec<u8> = reference
        .bytes()
        .take_while(u8::is_ascii_alphabetic)
        .collect();
    if letters.is_empty() {
        return None;
    }
    let mut column = 0usize;
    for letter in letters {
        column = column * 26 + usize::from(letter.to_ascii_uppercase() - b'A') + 1;
    }
    Some(column - 1)
}

/// The rows of the first worksheet with at least one value
/// (`eachRow({ includeEmpty: false })`), each as its cells from column A.
pub fn read_first_sheet(bytes: &[u8]) -> Result<Vec<Vec<Cell>>, String> {
    let entries = unzip(bytes)?;
    let path = first_sheet_path(&entries).ok_or("The workbook has no worksheets")?;
    let sheet = xml(&entries, &path).ok_or("The workbook has no worksheets")?;
    let strings = shared_strings(&entries);
    let mut reader = Reader::from_str(&sheet);
    let mut rows = Vec::new();
    let mut row: Vec<Cell> = Vec::new();
    // The open cell: its column and type, the `<v>` / inline text read.
    let mut cell: Option<(usize, String)> = None;
    let (mut value, mut in_value) = (String::new(), false);
    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        match &event {
            Event::Start(e) | Event::Empty(e) => {
                let empty = matches!(event, Event::Empty(_));
                match e.local_name().as_ref() {
                    b"row" if !empty => row.clear(),
                    b"c" => {
                        let column = attribute(e, b"r")
                            .and_then(|r| column_of(&r))
                            .unwrap_or(row.len());
                        let kind = attribute(e, b"t").unwrap_or_default();
                        value.clear();
                        if empty {
                            set_cell(&mut row, column, Cell::Empty);
                        } else {
                            cell = Some((column, kind));
                        }
                    }
                    b"v" | b"t" if !empty => in_value = true,
                    _ => {}
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"v" | b"t" => in_value = false,
                b"c" => {
                    if let Some((column, kind)) = cell.take() {
                        set_cell(&mut row, column, cell_value(&kind, &value, &strings));
                    }
                }
                b"row" => {
                    if row.iter().any(|c| *c != Cell::Empty) {
                        rows.push(std::mem::take(&mut row));
                    }
                    row.clear();
                }
                _ => {}
            },
            Event::Eof => break,
            other => {
                if in_value
                    && cell.is_some()
                    && let Some(text) = text_of(other)
                {
                    value.push_str(&text);
                }
            }
        }
    }
    Ok(rows)
}

fn set_cell(row: &mut Vec<Cell>, column: usize, value: Cell) {
    if row.len() <= column {
        row.resize(column + 1, Cell::Empty);
    }
    row[column] = value;
}

/// `toImportCellValue` of one `<c>`: shared / inline / formula strings,
/// booleans and numbers; an error cell or an empty string is nothing.
fn cell_value(kind: &str, value: &str, strings: &[String]) -> Cell {
    let text = |text: String| {
        if text.is_empty() {
            Cell::Empty
        } else {
            Cell::Text(text)
        }
    };
    match kind {
        "s" => value
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|index| strings.get(index).cloned())
            .map_or(Cell::Empty, text),
        "str" | "inlineStr" => text(value.to_string()),
        "b" => Cell::Bool(value.trim() == "1"),
        "e" => Cell::Empty,
        _ if value.trim().is_empty() => Cell::Empty,
        _ => value
            .trim()
            .parse::<f64>()
            .map_or(Cell::Empty, Cell::Number),
    }
}

/// `readXlsxHeaders`: the first row with a header (`Invalid XLSX file: …`
/// when unreadable).
pub fn read_headers(bytes: &[u8]) -> Result<Vec<String>, String> {
    let rows = read_first_sheet(bytes).map_err(|e| format!("Invalid XLSX file: {e}"))?;
    rows.iter()
        .map(|row| row.iter().map(Cell::header).collect::<Vec<_>>())
        .find(|headers| headers.iter().any(|h| !h.is_empty()))
        .ok_or_else(|| "Invalid XLSX file: The first worksheet has no header row".to_string())
}

/// `parseXlsxRows`: the header row, then each data row as `header -> value`
/// (empty headers skipped; a row without any value is left out).
pub fn read_records(
    bytes: &[u8],
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, String> {
    let rows = read_first_sheet(bytes).map_err(|e| format!("Invalid XLSX file: {e}"))?;
    let mut headers: Option<Vec<String>> = None;
    let mut out = Vec::new();
    for row in rows {
        let Some(names) = &headers else {
            let candidate: Vec<String> = row.iter().map(Cell::header).collect();
            if candidate.iter().any(|h| !h.is_empty()) {
                headers = Some(candidate);
            }
            continue;
        };
        let mut record = serde_json::Map::new();
        for (index, name) in names.iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            if let Some(value) = row.get(index).and_then(Cell::json) {
                record.insert(name.clone(), value);
            }
        }
        if !record.is_empty() {
            out.push(record);
        }
    }
    if headers.is_none() {
        return Err("Invalid XLSX file: The first worksheet has no header row".into());
    }
    Ok(out)
}

// --- writer --------------------------------------------------------------

fn escape(text: &str) -> String {
    quick_xml::escape::escape(text).into_owned()
}

/// The ZIP archive of `parts` (stored entries).
fn zip(parts: &[(&str, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, content) in parts {
        let mut crc = flate2::Crc::new();
        crc.update(content.as_bytes());
        let (crc, size) = (crc.sum(), content.len() as u32);
        let offset = out.len() as u32;
        let name_len = name.len() as u16;
        // Version 20, no flags, stored, 1980-01-01 00:00.
        let common = |buf: &mut Vec<u8>| {
            for half in [20u16, 0, 0, 0, 0x21] {
                buf.extend_from_slice(&half.to_le_bytes());
            }
            for word in [crc, size, size] {
                buf.extend_from_slice(&word.to_le_bytes());
            }
            buf.extend_from_slice(&name_len.to_le_bytes());
            buf.extend_from_slice(&0u16.to_le_bytes());
        };
        out.extend_from_slice(b"PK\x03\x04");
        common(&mut out);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(content.as_bytes());
        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&20u16.to_le_bytes());
        common(&mut central);
        // Comment length, disk, internal (2) and external (4) attributes.
        central.extend_from_slice(&[0; 10]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let (central_offset, central_size) = (out.len() as u32, central.len() as u32);
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&[0; 4]);
    let count = parts.len() as u16;
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn column_name(mut index: usize) -> String {
    let mut name = Vec::new();
    loop {
        name.push(b'A' + (index % 26) as u8);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    name.reverse();
    String::from_utf8(name).unwrap_or_default()
}

const XMLNS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const HEADER: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;

/// A one-sheet workbook: `rows` with a bold first row, every column
/// `width` wide (`createProductImportTemplate`).
pub fn write_workbook(sheet_name: &str, rows: &[Vec<Cell>], width: u32) -> Vec<u8> {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let mut data = String::new();
    for (r, row) in rows.iter().enumerate() {
        data.push_str(&format!(r#"<row r="{}">"#, r + 1));
        let style = if r == 0 { r#" s="1""# } else { "" };
        for (c, cell) in row.iter().enumerate() {
            let reference = format!("{}{}", column_name(c), r + 1);
            match cell {
                Cell::Empty => {}
                Cell::Text(text) => data.push_str(&format!(
                    r#"<c r="{reference}"{style} t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#,
                    escape(text)
                )),
                Cell::Number(n) => data.push_str(&format!(
                    r#"<c r="{reference}"{style}><v>{}</v></c>"#,
                    chatbotx_core::js::number_to_string(*n)
                )),
                Cell::Bool(b) => data.push_str(&format!(
                    r#"<c r="{reference}"{style} t="b"><v>{}</v></c>"#,
                    u8::from(*b)
                )),
            }
        }
        data.push_str("</row>");
    }
    let parts = [
        (
            "[Content_Types].xml",
            format!(
                r#"{HEADER}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#
            ),
        ),
        (
            "_rels/.rels",
            format!(
                r#"{HEADER}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL_NS}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/workbook.xml",
            format!(
                r#"{HEADER}<workbook xmlns="{XMLNS}" xmlns:r="{REL_NS}"><sheets><sheet name="{}" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
                escape(sheet_name)
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            format!(
                r#"{HEADER}<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL_NS}/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="{REL_NS}/styles" Target="styles.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/styles.xml",
            format!(
                r#"{HEADER}<styleSheet xmlns="{XMLNS}"><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs></styleSheet>"#
            ),
        ),
        (
            "xl/worksheets/sheet1.xml",
            format!(
                r#"{HEADER}<worksheet xmlns="{XMLNS}" xmlns:r="{REL_NS}"><cols><col min="1" max="{columns}" width="{width}" customWidth="1"/></cols><sheetData>{data}</sheetData></worksheet>"#
            ),
        ),
    ];
    zip(&parts)
}
