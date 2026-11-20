//! `ai-agent/lib/text-extractor.ts`: plain text, CSV and XLSX (as tables),
//! Markdown, HTML, RTF, XML, VTT and properties files; the binary formats
//! (PDF, DOCX, PPTX, PPT, EPUB, e-mail) are in [`super::office`].
//!
//! ponytail: a legacy `.xls` (BIFF) workbook is not read (the ZIP reader
//! rejects it, `fileUnreadable`); add a BIFF reader when such files matter.

use std::sync::LazyLock;

use regex::Regex;

use super::chunk::{Document, Sheet, u16len};

const PDF: &[&str] = &["application/pdf"];
const DOCX: &[&str] = &[
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/msword",
];
const SPREADSHEET: &[&str] = &[
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.ms-excel",
];
const CSV: &[&str] = &["text/csv"];
const HTML: &[&str] = &["text/html", "application/xhtml+xml"];
const MARKDOWN: &[&str] = &["text/markdown", "text/x-markdown"];
const RTF: &[&str] = &["application/rtf", "text/rtf"];
const XML: &[&str] = &["application/xml", "text/xml"];
const EMAIL: &[&str] = &["message/rfc822", "application/vnd.ms-outlook"];
const EPUB: &[&str] = &["application/epub+zip"];
const PPT: &[&str] = &["application/vnd.ms-powerpoint"];
const PPTX: &[&str] =
    &["application/vnd.openxmlformats-officedocument.presentationml.presentation"];

/// Default `maxTextChars` (`MAX_EXTRACTED_TEXT_CHARS`).
pub const MAX_TEXT_CHARS: usize = 5_000_000;

/// The document types of the heavy `extractTextFromFile` reader
/// (`supportedDocumentMimeTypes`).
pub fn is_document_mime(mime: &str) -> bool {
    [
        PDF,
        DOCX,
        SPREADSHEET,
        CSV,
        HTML,
        MARKDOWN,
        RTF,
        XML,
        EMAIL,
        EPUB,
        PPT,
        PPTX,
        &["text/vtt", "text/x-java-properties"],
    ]
    .iter()
    .any(|set| set.contains(&mime))
}

/// `extractKnowledgeDocument` options.
#[derive(Debug, Clone)]
pub struct Options {
    pub source_name: String,
    pub max_text_chars: usize,
    pub force_prose: bool,
    /// 1-based row with the column names.
    pub tabular_header_row: usize,
}

/// `normalizeMimeType`, then the extension when no type is stored.
pub fn mime_of(mime: &str, path: &str) -> String {
    let normalized = mime
        .to_lowercase()
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_owned();
    if !normalized.is_empty() {
        return normalized;
    }
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "txt" | "text" => "text/plain",
        "csv" => "text/csv",
        "md" | "markdown" => "text/markdown",
        "html" | "htm" => "text/html",
        "xhtml" => "application/xhtml+xml",
        "xml" => "application/xml",
        "rtf" => "application/rtf",
        "vtt" => "text/vtt",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "docx" => DOCX[0],
        "doc" => "application/msword",
        "xlsx" => SPREADSHEET[0],
        "xls" => "application/vnd.ms-excel",
        "pptx" => PPTX[0],
        "ppt" => PPT[0],
        "epub" => EPUB[0],
        "eml" => "message/rfc822",
        _ => "",
    }
    .to_owned()
}

/// `TextDecoder("utf-8")`: lossy, a leading BOM dropped.
fn decode(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned()
}

/// `s.slice(0, n)` in UTF-16 units.
pub(super) fn cut(s: String, n: usize) -> String {
    if u16len(&s) <= n {
        return s;
    }
    let units: Vec<u16> = s.encode_utf16().take(n).collect();
    String::from_utf16_lossy(&units)
}

static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\t ]+").expect("regex"));
static BLANK_LINES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").expect("regex"));

/// `normalizeWhitespace`.
pub fn normalize_whitespace(input: &str) -> String {
    let out: String = input
        .chars()
        .map(|c| {
            if (c as u32) < 32 && !matches!(c, '\n' | '\r' | '\t') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let out = out.replace("\r\n", "\n");
    let out = SPACES.replace_all(&out, " ");
    let out = BLANK_LINES.replace_all(&out, "\n\n");
    super::chunk::js_trim(&out).to_owned()
}

/// `extractTextFromBuffer`. `Err` = the file cannot be read as text.
pub fn text_from_bytes(bytes: &[u8], mime: &str, max: usize) -> Result<String, String> {
    use super::office;
    let is = |set: &[&str]| set.contains(&mime);
    let binary = if is(PDF) {
        Some(office::pdf(bytes)?)
    } else if is(DOCX) {
        Some(office::docx(bytes)?)
    } else if is(SPREADSHEET) {
        Some(office::xlsx(bytes)?)
    } else if is(EMAIL) {
        Some(office::email(bytes))
    } else if is(PPTX) {
        Some(office::pptx(bytes)?)
    } else if is(PPT) {
        Some(office::ppt(bytes))
    } else if is(EPUB) {
        Some(office::epub(bytes)?)
    } else {
        None
    };
    if let Some(text) = binary {
        return Ok(cut(text, max));
    }
    let text = decode(bytes);
    let out = if is(HTML) {
        html_to_text(&text)
    } else if is(MARKDOWN) {
        remove_markdown(&text)
    } else if is(RTF) {
        rtf_to_text(&text)
    } else if is(XML) {
        xml_to_text(&text)
    } else {
        // CSV, VTT, properties and every other type: the text itself.
        text
    };
    Ok(cut(normalize_whitespace(&out), max))
}

/// `extractKnowledgeDocument` on the stored bytes.
pub fn knowledge_document(bytes: &[u8], mime: &str, opts: &Options) -> Result<Document, String> {
    let csv = CSV.contains(&mime);
    if csv && std::str::from_utf8(bytes).is_err() {
        return Err("CSV file is not valid UTF-8".to_owned());
    }
    if !opts.force_prose && csv {
        let rows = parse_csv(&decode(bytes));
        let (sheets, truncated) = tabular_sheets(
            vec![(opts.source_name.clone(), rows)],
            opts.max_text_chars,
            opts.tabular_header_row,
        );
        return Ok(Document::Tabular { sheets, truncated });
    }
    if !opts.force_prose && SPREADSHEET.contains(&mime) {
        // `read` throws outside the table build: a corrupt workbook fails.
        let workbook = super::office::xlsx_matrices(bytes)?
            .into_iter()
            .map(|(name, rows)| {
                let name = if name.is_empty() {
                    opts.source_name.clone()
                } else {
                    name
                };
                (name, rows)
            })
            .collect();
        let (sheets, truncated) =
            tabular_sheets(workbook, opts.max_text_chars, opts.tabular_header_row);
        return Ok(Document::Tabular { sheets, truncated });
    }
    if !opts.force_prose && MARKDOWN.contains(&mime) {
        return Ok(Document::Markdown(cut(
            normalize_whitespace(&decode(bytes)),
            opts.max_text_chars,
        )));
    }
    Ok(Document::Prose {
        text: text_from_bytes(bytes, mime, opts.max_text_chars)?,
        fallback: None,
    })
}

/// `tabularSheetsFromWorkbook`: header row, `Column n` names, a shared
/// text budget.
fn tabular_sheets(
    workbook: Vec<(String, Vec<Vec<String>>)>,
    max_text_chars: usize,
    header_row: usize,
) -> (Vec<Sheet>, bool) {
    let mut sheets = Vec::new();
    let mut remaining = max_text_chars;
    let mut truncated = false;
    let mut consume = |value: String| -> (String, bool) {
        if value.is_empty() {
            return (value, false);
        }
        if remaining == 0 {
            truncated = true;
            return (String::new(), true);
        }
        let len = u16len(&value);
        let kept = cut(value, remaining);
        remaining -= u16len(&kept);
        let exhausted = u16len(&kept) < len;
        truncated |= exhausted;
        (kept, exhausted)
    };
    'sheets: for (name, matrix) in workbook {
        let skip = header_row.saturating_sub(1);
        let mut matrix = matrix.into_iter().skip(skip);
        let header = matrix.next().unwrap_or_default();
        let data: Vec<Vec<String>> = matrix.collect();
        if data.is_empty() {
            continue;
        }
        let columns = data.iter().map(Vec::len).fold(header.len(), usize::max);
        let mut headers = Vec::with_capacity(columns);
        for i in 0..columns {
            let h = normalize_whitespace(header.get(i).map_or("", String::as_str));
            let source = if h.is_empty() {
                format!("Column {}", i + 1)
            } else {
                h
            };
            let (value, exhausted) = consume(source);
            headers.push(value);
            if exhausted {
                break 'sheets;
            }
        }
        let mut rows = Vec::new();
        let mut stop = false;
        for source_row in &data {
            let mut row = Vec::with_capacity(columns);
            for i in 0..columns {
                let (value, exhausted) = consume(normalize_whitespace(
                    source_row.get(i).map_or("", String::as_str),
                ));
                row.push(value);
                if exhausted {
                    stop = true;
                    break;
                }
            }
            if row.iter().any(|c| !c.is_empty()) {
                rows.push(row);
            }
            if stop {
                break;
            }
        }
        if !rows.is_empty() {
            sheets.push(Sheet {
                name,
                headers,
                rows,
            });
        }
        if stop {
            break;
        }
    }
    (sheets, truncated)
}

/// The SheetJS CSV reader with `raw: true`: the delimiter is the most
/// frequent of `, \t ; |` on the first line, quotes per RFC 4180, cells kept
/// as written, blank lines dropped, short rows padded to the widest.
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let first = text.lines().next().unwrap_or("");
    let mut sep = ',';
    let mut best = 0;
    for c in [',', '\t', ';', '|'] {
        let n = first.matches(c).count();
        if n > best {
            best = n;
            sep = c;
        }
    }
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cell.push('"');
                } else {
                    quoted = false;
                }
            } else {
                cell.push(c);
            }
        } else if c == '"' && cell.is_empty() {
            quoted = true;
        } else if c == sep {
            row.push(std::mem::take(&mut cell));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut cell));
            rows.push(std::mem::take(&mut row));
        } else {
            cell.push(c);
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows.retain(|r| r.iter().any(|c| !c.is_empty()));
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for r in &mut rows {
        r.resize(width, String::new());
    }
    rows
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("extractor regex")
}

/// `extractTextFromRtf`.
fn rtf_to_text(rtf: &str) -> String {
    static HEX: LazyLock<Regex> = LazyLock::new(|| re(r"\\'[0-9a-fA-F]{2}"));
    static WORD: LazyLock<Regex> = LazyLock::new(|| re(r"\\[a-zA-Z]+-?\d* ?"));
    static GROUP: LazyLock<Regex> = LazyLock::new(|| re(r"\{[^{}]*\}"));
    static REST: LazyLock<Regex> = LazyLock::new(|| re(r"[{}\\]"));
    let text = HEX.replace_all(rtf, " ");
    let text = WORD.replace_all(&text, " ");
    let text = GROUP.replace_all(&text, " ");
    REST.replace_all(&text, " ").into_owned()
}

fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

/// `extractTextFromXml`.
fn xml_to_text(xml: &str) -> String {
    static TAG: LazyLock<Regex> = LazyLock::new(|| re(r"<[^>]+>"));
    decode_entities(&TAG.replace_all(xml, " "))
}

/// `htmlToText(html, { wordwrap: false })`, reduced: scripts, styles and the
/// head are dropped, block tags and `<br>` break lines, headings are upper
/// case, links keep their text.
// ponytail: not the full html-to-text formatter (tables, link URLs, lists).
pub(super) fn html_to_text(html: &str) -> String {
    static DROP: LazyLock<Regex> = LazyLock::new(|| {
        re(r"(?is)<(script|style|head|noscript)\b.*?</(script|style|head|noscript)\s*>")
    });
    static HEADING: LazyLock<Regex> =
        LazyLock::new(|| re(r"(?is)<h[1-6]\b[^>]*>(.*?)</h[1-6]\s*>"));
    static BREAK: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)<br\s*/?>"));
    static BLOCK: LazyLock<Regex> = LazyLock::new(|| {
        re(
            r"(?i)</?(p|div|section|article|header|footer|li|ul|ol|tr|table|blockquote|pre|hr)\b[^>]*>",
        )
    });
    static TAG: LazyLock<Regex> = LazyLock::new(|| re(r"<[^>]+>"));
    let text = DROP.replace_all(html, "");
    let text = HEADING.replace_all(&text, |c: &regex::Captures<'_>| {
        format!("\n\n{}\n\n", c[1].to_uppercase())
    });
    let text = BREAK.replace_all(&text, "\n");
    let text = BLOCK.replace_all(&text, "\n\n");
    decode_entities(&TAG.replace_all(&text, ""))
}

/// `remove-markdown` with `stripListLeaders`, `gfm` and `useImgAltText`.
// ponytail: the library's rules without backreferences (emphasis of 1-3
// markers); exotic nesting can differ.
pub fn remove_markdown(md: &str) -> String {
    static RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
        vec![
            (re(r"(?m)^(-\s*?|\*\s*?|_\s*?){3,}\s*"), ""),
            (re(r"(?m)^([\s\t]*)([*\-+]|\d+\.)\s+"), "$1"),
            (re(r"\n={2,}"), "\n"),
            (re(r"~{3}.*\n"), ""),
            (re(r"~~"), ""),
            (re(r"`{3}.*\n"), ""),
            (re(r"<[^>]*>"), ""),
            (re(r"^[=\-]{2,}\s*$"), ""),
            (re(r"\[\^.+?\](: .*?$)?"), ""),
            (re(r"\s{0,2}\[.*?\]: .*?$"), ""),
            (re(r"!\[(.*?)\][\[(].*?[\])]"), "$1"),
            (re(r"\[([^\]]*?)\][\[(].*?[\])]"), "$1"),
            (re(r"(?m)^\s{0,3}>\s?"), ""),
            (re(r#"^\s{1,2}\[(.*?)\]: (\S+)( ".*?")?\s*$"#), ""),
            (
                re(r"(?m)^(\n)?\s*#{1,6}\s*( (.+))? +#+$|^(\n)?\s*#{1,6}\s*( (.+))?$"),
                "$1$3$4$6",
            ),
            (re(r"\*\*\*(\S(?:.*?\S)?)\*\*\*"), "$1"),
            (re(r"\*\*(\S(?:.*?\S)?)\*\*"), "$1"),
            (re(r"\*(\S(?:.*?\S)?)\*"), "$1"),
            (re(r"(^|\W)__(\S(?:.*?\S)?)__($|\W)"), "$1$2$3"),
            (re(r"(^|\W)_(\S(?:.*?\S)?)_($|\W)"), "$1$2$3"),
            (re(r"`(.+?)`"), "$1"),
            (re(r"~(.*?)~"), "$1"),
        ]
    });
    let mut out = md.to_owned();
    for (rule, with) in RULES.iter() {
        out = rule.replace_all(&out, *with).into_owned();
    }
    out
}

#[cfg(test)]
mod tests {
    //! `text-extractor.test.ts` (the text formats).
    use super::*;

    fn opts(name: &str) -> Options {
        Options {
            source_name: name.into(),
            max_text_chars: MAX_TEXT_CHARS,
            force_prose: false,
            tabular_header_row: 1,
        }
    }

    fn tabular_len(doc: &Document) -> usize {
        match doc {
            Document::Tabular { sheets, .. } => sheets
                .iter()
                .flat_map(|s| s.headers.iter().chain(s.rows.iter().flatten()))
                .map(|c| u16len(c))
                .sum(),
            _ => 0,
        }
    }

    #[test]
    fn extract_text_from_file_unchanged_legacy_output() {
        for (mime, input, expected) in [
            (
                "text/csv",
                "Product,Price\nCà phê,10000",
                "Product,Price\nCà phê,10000",
            ),
            (
                "text/markdown",
                "# Policy\n\n- Keep the **receipt**\n\n[Link](https://x.io)",
                "Policy\n\nKeep the receipt\n\nLink",
            ),
            (
                "text/plain",
                "Line one\r\nLine\ttwo\u{0}end",
                "Line one\nLine two end",
            ),
        ] {
            assert_eq!(
                text_from_bytes(input.as_bytes(), mime, MAX_TEXT_CHARS).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn reads_a_vietnamese_csv_without_bom_as_a_table_named_after_the_file() {
        let doc = knowledge_document(
            b"Product,Price\nC\xc3\xa0 ph\xc3\xaa,10000",
            "text/csv",
            &opts("prices.csv"),
        );
        assert_eq!(
            doc.unwrap(),
            Document::Tabular {
                sheets: vec![Sheet {
                    name: "prices.csv".into(),
                    headers: vec!["Product".into(), "Price".into()],
                    rows: vec![vec!["Cà phê".into(), "10000".into()]],
                }],
                truncated: false,
            }
        );
    }

    #[test]
    fn keeps_csv_cells_as_written_instead_of_reformatting_dates_and_numbers() {
        let doc = knowledge_document(
            b"Phone,Date,US date,Rate,Big\n0901234567,2024-01-15,1/2/2024,10%,12345678901234567890",
            "text/csv",
            &opts("contacts.csv"),
        )
        .unwrap();
        let Document::Tabular { sheets, .. } = doc else {
            panic!("tabular")
        };
        assert_eq!(
            sheets[0].rows,
            vec![vec![
                "0901234567",
                "2024-01-15",
                "1/2/2024",
                "10%",
                "12345678901234567890"
            ]]
        );
    }

    #[test]
    fn preserves_markdown_structure_only_for_ai_file_extraction() {
        assert_eq!(
            knowledge_document(
                b"# Policy\n\n- Keep the receipt",
                "text/markdown",
                &opts("policy.md")
            )
            .unwrap(),
            Document::Markdown("# Policy\n\n- Keep the receipt".into())
        );
    }

    #[test]
    fn returns_other_types_as_prose_identical_to_extract_text_from_file() {
        let bytes = b"Line one\r\nLine\ttwo";
        let legacy = text_from_bytes(bytes, "text/plain", MAX_TEXT_CHARS).unwrap();
        assert_eq!(
            knowledge_document(bytes, "text/plain", &opts("notes.txt")).unwrap(),
            Document::Prose {
                text: legacy,
                fallback: None
            }
        );
    }

    #[test]
    fn rejects_a_windows_1252_csv_instead_of_embedding_u_fffd() {
        let bytes = [0x4e, 0x61, 0x6d, 0x65, 0x0a, 0x43, 0x61, 0x66, 0xe9];
        for force_prose in [false, true] {
            let o = Options {
                force_prose,
                ..opts("notes.csv")
            };
            assert!(
                knowledge_document(&bytes, "text/csv", &o)
                    .unwrap_err()
                    .contains("not valid UTF-8")
            );
        }
    }

    #[test]
    fn accepts_a_utf8_csv_with_a_bom() {
        let mut bytes = vec![0xef, 0xbb, 0xbf];
        bytes.extend_from_slice("Tên,Giá\nCà phê,50000\n".as_bytes());
        let doc = knowledge_document(&bytes, "text/csv", &opts("notes.csv")).unwrap();
        let Document::Tabular { sheets, .. } = doc else {
            panic!("tabular")
        };
        assert_eq!(sheets[0].headers, vec!["Tên", "Giá"]);
        assert_eq!(sheets[0].rows, vec![vec!["Cà phê", "50000"]]);
    }

    #[test]
    fn returns_no_sheets_when_the_header_row_is_past_the_last_data_row() {
        let o = Options {
            tabular_header_row: 20,
            ..opts("prices.csv")
        };
        assert_eq!(
            knowledge_document(b"Product,Price\nBasic,199000", "text/csv", &o).unwrap(),
            Document::Tabular {
                sheets: vec![],
                truncated: false
            }
        );
    }

    #[test]
    fn keeps_markdown_with_invalid_utf8_bytes_as_markdown() {
        assert_eq!(
            knowledge_document(b"# Title\nbad \xff", "text/markdown", &opts("notes.md")).unwrap(),
            Document::Markdown("# Title\nbad \u{fffd}".into())
        );
    }

    #[test]
    fn cuts_a_50000_character_cell_to_the_text_budget() {
        let csv = format!("Title,Body\nLong,{}", "x".repeat(50_000));
        let o = Options {
            max_text_chars: 10_000,
            ..opts("notes.csv")
        };
        let doc = knowledge_document(csv.as_bytes(), "text/csv", &o).unwrap();
        assert!(matches!(
            doc,
            Document::Tabular {
                truncated: true,
                ..
            }
        ));
        assert_eq!(tabular_len(&doc), 10_000);
    }

    #[test]
    fn keeps_a_table_with_more_rows_than_a_spread_argument_list_can_hold() {
        let csv = format!(
            "SKU\n{}",
            (0..150_000)
                .map(|i| format!("s{i}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let Document::Tabular { sheets, .. } =
            knowledge_document(csv.as_bytes(), "text/csv", &opts("skus.csv")).unwrap()
        else {
            panic!("tabular")
        };
        assert_eq!(sheets[0].rows.len(), 150_000);
    }

    #[test]
    fn cuts_markdown_longer_than_the_text_budget() {
        let md = format!("# Title\n{}", "a".repeat(500));
        let o = Options {
            max_text_chars: 100,
            ..opts("long.md")
        };
        assert_eq!(
            knowledge_document(md.as_bytes(), "text/markdown", &o).unwrap(),
            Document::Markdown(format!("# Title\n{}", "a".repeat(92)))
        );
    }

    #[test]
    fn rejects_a_corrupt_xlsx_instead_of_embedding_binary_garbage() {
        let bytes = b"PK\x03\x04garbage-bytes-not-a-zip";
        assert!(knowledge_document(bytes, SPREADSHEET[0], &opts("broken.xlsx")).is_err());
        assert!(text_from_bytes(bytes, "application/pdf", MAX_TEXT_CHARS).is_err());
    }
}
