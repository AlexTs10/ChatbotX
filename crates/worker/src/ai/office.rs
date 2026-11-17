//! The binary formats of `ai-agent/lib/text-extractor.ts`: PDF
//! (pdf-parse), DOCX (mammoth `extractRawText`), XLSX (SheetJS), PPTX,
//! PPT, EPUB and e-mail (mailparser).
//!
//! The ZIP and worksheet reader is the import one
//! (`chatbotx_business::import::xlsx`).

use std::collections::HashMap;
use std::sync::LazyLock;

use chatbotx_business::import::xlsx::{Cell, SheetCell, read_workbook, unzip_entries};
use chatbotx_core::js::number_to_string;
use regex::Regex;

use super::extract::{MAX_TEXT_CHARS, cut, html_to_text, normalize_whitespace};

/// `MAX_EXTRACTABLE_PAGES`.
const MAX_PAGES: usize = 200;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("office regex")
}

/// XML entity decoding (`&lt;` … and numeric references).
fn unescape_xml(text: &str) -> String {
    static ENTITY: LazyLock<Regex> = LazyLock::new(|| re(r"&(#x[0-9a-fA-F]+|#[0-9]+|[a-z]+);"));
    ENTITY
        .replace_all(text, |c: &regex::Captures<'_>| {
            let name = &c[1];
            let decoded = match name {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => name
                    .strip_prefix("#x")
                    .map(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| name.strip_prefix('#').map(|d| d.parse().ok()))
                    .flatten()
                    .and_then(char::from_u32),
            };
            decoded.map_or_else(|| c[0].to_owned(), String::from)
        })
        .into_owned()
}

/// `extractTextFromPdf`.
pub(super) fn pdf(bytes: &[u8]) -> Result<String, String> {
    // pdf-extract panics on some malformed files; that is a parse failure.
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem(bytes)
    }));
    match parsed {
        Ok(Ok(text)) => Ok(normalize_whitespace(&text)),
        Ok(Err(error)) => {
            tracing::warn!(err = %error, "PDF parsing failed, falling back to plain text");
            Err("PDF parsing failed".to_owned())
        }
        Err(_) => {
            tracing::warn!("PDF parsing failed, falling back to plain text");
            Err("PDF parsing failed".to_owned())
        }
    }
}

/// `extractTextFromDocx` (mammoth `extractRawText`: each paragraph's text
/// followed by a blank line; run tabs and breaks kept).
pub(super) fn docx(bytes: &[u8]) -> Result<String, String> {
    static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
        re(
            r"(?s)<w:t(?:\s[^>]*)?>(.*?)</w:t>|<w:tab\s*/>|<w:(?:br|cr)\b[^>]*/>|</w:p>|<w:p(?:\s[^>]*)?/>",
        )
    });
    let failed = || "DOCX parsing failed".to_owned();
    let entries = unzip_entries(bytes).map_err(|_| failed())?;
    let document = entries
        .iter()
        .find(|(name, _)| name == "word/document.xml")
        .map(|(_, xml)| String::from_utf8_lossy(xml).into_owned())
        .ok_or_else(failed)?;
    let mut out = String::new();
    for token in TOKEN.captures_iter(&document) {
        if let Some(text) = token.get(1) {
            out.push_str(&unescape_xml(text.as_str()));
            continue;
        }
        let tag = &token[0];
        if tag.starts_with("<w:tab") {
            out.push('\t');
        } else if tag.starts_with("<w:br") || tag.starts_with("<w:cr") {
            out.push('\n');
        } else {
            out.push_str("\n\n");
        }
    }
    Ok(normalize_whitespace(&out))
}

/// `sheet_to_json(sheet, { defval: "" })` keys: SheetJS orders integer-like
/// keys first (JS object key order).
fn js_key_order(keys: &mut [(String, String)]) {
    let index = |k: &str| -> Option<u32> {
        if k == "0"
            || (!k.starts_with('0') && !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()))
        {
            k.parse::<u32>().ok().filter(|n| *n != u32::MAX)
        } else {
            None
        }
    };
    keys.sort_by_key(|(k, _)| index(k).map_or((1, 0), |n| (0, n)));
}

/// The raw value of a cell as `${v}` prints it (`raw: true`).
fn raw_text(cell: &Cell) -> String {
    match cell {
        Cell::Empty => String::new(),
        Cell::Text(text) => text.clone(),
        Cell::Number(n) => number_to_string(*n),
        Cell::Bool(b) => b.to_string(),
    }
}

/// The displayed text of a cell (`raw: false`, `cellNF: true`, with
/// `expandLargeIntegerCells`).
fn shown_text(cell: &SheetCell) -> String {
    match &cell.value {
        Cell::Empty => String::new(),
        Cell::Text(text) => text.clone(),
        Cell::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        Cell::Number(n) => numfmt::format(*n, &cell.format),
    }
}

/// Sheets of displayed cell text: (sheet name, rows).
pub(super) type Matrices = Vec<(String, Vec<Vec<String>>)>;

/// The workbook as text matrices of the displayed values
/// (`tabularSheetsFromWorkbook` input).
pub(super) fn xlsx_matrices(bytes: &[u8]) -> Result<Matrices, String> {
    Ok(read_workbook(bytes)?
        .into_iter()
        .map(|(name, rows)| {
            let rows = rows
                .iter()
                .map(|row| row.iter().map(shown_text).collect())
                .collect();
            (name, rows)
        })
        .collect())
}

/// `extractTextFromXlsx`: each data row as `Header: value` lines.
pub(super) fn xlsx(bytes: &[u8]) -> Result<String, String> {
    let sheets = read_workbook(bytes).map_err(|error| {
        tracing::warn!(err = %error, "XLSX/XLS parsing failed, falling back to plain text");
        "XLSX/XLS parsing failed".to_owned()
    })?;
    let mut blocks = Vec::new();
    for (_, rows) in sheets {
        let Some((header, data)) = rows.split_first() else {
            continue;
        };
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        // `sheet_to_json` header names: displayed text, `__EMPTY` for a blank,
        // `_n` suffixes for repeats.
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut keys = Vec::with_capacity(width);
        for column in 0..width {
            let shown = header.get(column).map(shown_text).unwrap_or_default();
            let name = if shown.is_empty() {
                "__EMPTY".to_owned()
            } else {
                shown
            };
            let key = match seen.get(&name).copied() {
                None => {
                    seen.insert(name.clone(), 1);
                    name
                }
                Some(mut counter) => {
                    let mut candidate = format!("{name}_{counter}");
                    counter += 1;
                    while seen.contains_key(&candidate) {
                        candidate = format!("{name}_{counter}");
                        counter += 1;
                    }
                    seen.insert(name, counter);
                    seen.insert(candidate.clone(), 1);
                    candidate
                }
            };
            keys.push(key);
        }
        for row in data {
            if row.iter().all(|c| c.value == Cell::Empty) {
                continue;
            }
            let mut parts: Vec<(String, String)> = keys
                .iter()
                .enumerate()
                .map(|(i, k)| {
                    (
                        k.clone(),
                        row.get(i).map(|c| raw_text(&c.value)).unwrap_or_default(),
                    )
                })
                .collect();
            js_key_order(&mut parts);
            let lines: Vec<String> = parts
                .into_iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(k, v)| format!("{k}: {v}"))
                .collect();
            if !lines.is_empty() {
                blocks.push(lines.join("\n"));
            }
        }
    }
    Ok(cut(blocks.join("\n\n"), MAX_TEXT_CHARS))
}

/// The number in `slideN.xml` / `notesSlideN.xml`.
fn slide_number(name: &str) -> u64 {
    static NUM: LazyLock<Regex> = LazyLock::new(|| re(r"slide(\d+)\.xml$"));
    NUM.captures(name)
        .and_then(|c| c[1].parse().ok())
        .unwrap_or(0)
}

/// `extractTextFromPptx`: the `<a:t>` runs of each slide, then of each
/// notes page.
pub(super) fn pptx(bytes: &[u8]) -> Result<String, String> {
    static SLIDE: LazyLock<Regex> = LazyLock::new(|| re(r"^ppt/slides/slide\d+\.xml$"));
    static NOTES: LazyLock<Regex> = LazyLock::new(|| re(r"^ppt/notesSlides/notesSlide\d+\.xml$"));
    static TEXT: LazyLock<Regex> = LazyLock::new(|| re(r"<a:t[^>]*>([^<]*)</a:t>"));
    static TAG: LazyLock<Regex> = LazyLock::new(|| re(r"<[^>]+>"));
    let entries = unzip_entries(bytes).map_err(|error| {
        tracing::warn!(err = %error, "PPTX parsing failed");
        "PPTX parsing failed".to_owned()
    })?;
    let pick = |pattern: &Regex| {
        let mut parts: Vec<&(String, Vec<u8>)> = entries
            .iter()
            .filter(|(n, _)| pattern.is_match(n))
            .collect();
        parts.sort_by_key(|(n, _)| slide_number(n));
        if parts.len() > MAX_PAGES {
            tracing::warn!(
                total = parts.len(),
                limit = MAX_PAGES,
                "PPTX slide count exceeds limit, truncating"
            );
        }
        parts.truncate(MAX_PAGES);
        parts
            .into_iter()
            .filter_map(|(_, xml)| {
                let xml = String::from_utf8_lossy(xml);
                let text = TEXT
                    .find_iter(&xml)
                    .map(|m| TAG.replace_all(m.as_str(), "").into_owned())
                    .collect::<Vec<_>>()
                    .join(" ");
                (!text.trim().is_empty()).then_some(text)
            })
            .collect::<Vec<_>>()
    };
    let mut texts = pick(&SLIDE);
    texts.extend(pick(&NOTES));
    Ok(cut(
        normalize_whitespace(&texts.join("\n\n")),
        MAX_TEXT_CHARS,
    ))
}

/// `extractTextFromPpt`: UTF-16LE runs of at least 4 printable characters.
pub(super) fn ppt(bytes: &[u8]) -> String {
    const MIN_RUN_CHARS: usize = 4;
    let unit = |i: usize| u16::from(bytes[i]) | (u16::from(bytes[i + 1]) << 8);
    let mut texts = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let cu = unit(i);
        if (0x20..0xd800).contains(&cu) || (cu > 0xdfff && cu < 0xfffe) {
            let start = i;
            while i + 1 < bytes.len() {
                let cu = unit(i);
                if cu < 0x20 || (0xd800..=0xdfff).contains(&cu) || cu >= 0xfffe {
                    break;
                }
                i += 2;
            }
            if (i - start) / 2 >= MIN_RUN_CHARS {
                let units: Vec<u16> = (start..i).step_by(2).map(unit).collect();
                texts.push(String::from_utf16_lossy(&units));
            }
        } else {
            i += 1;
        }
    }
    normalize_whitespace(&texts.join(" "))
}

/// `extractTextFromEpub`: the spine documents (or every HTML file) as text.
pub(super) fn epub(bytes: &[u8]) -> Result<String, String> {
    static OPF: LazyLock<Regex> = LazyLock::new(|| re(r#"full-path="([^"]+\.opf)""#));
    static ITEMREF: LazyLock<Regex> = LazyLock::new(|| re(r#"<itemref\s[^>]*idref="([^"]+)""#));
    static CONTENT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\.(html|xhtml|htm)$"));
    let entries = unzip_entries(bytes).map_err(|error| {
        tracing::warn!(err = %error, "EPUB parsing failed");
        "EPUB parsing failed".to_owned()
    })?;
    let file = |name: &str| {
        entries
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
    };
    let mut paths: Vec<String> = Vec::new();
    let opf_path =
        file("META-INF/container.xml").and_then(|xml| OPF.captures(&xml).map(|c| c[1].to_owned()));
    if let Some(opf_path) = opf_path
        && let Some(opf) = file(&opf_path)
    {
        let dir = opf_path.rsplit_once('/').map_or("", |(d, _)| d);
        for idref in ITEMREF.captures_iter(&opf) {
            let id = regex::escape(&idref[1]);
            let href = re(&format!(r#"<item[^>]+id="{id}"[^>]+href="([^"]+)""#))
                .captures(&opf)
                .or_else(|| {
                    re(&format!(r#"<item[^>]+href="([^"]+)"[^>]+id="{id}""#)).captures(&opf)
                })
                .map(|c| c[1].to_owned());
            if let Some(href) = href {
                paths.push(if dir.is_empty() {
                    href
                } else {
                    format!("{dir}/{href}")
                });
            }
        }
    }
    if paths.is_empty() {
        paths = entries
            .iter()
            .filter(|(n, _)| CONTENT.is_match(n))
            .map(|(n, _)| n.clone())
            .collect();
    }
    if paths.len() > MAX_PAGES {
        tracing::warn!(
            total = paths.len(),
            limit = MAX_PAGES,
            "EPUB chapter count exceeds limit, truncating"
        );
        paths.truncate(MAX_PAGES);
    }
    let texts: Vec<String> = paths
        .iter()
        .filter_map(|p| file(p))
        .map(|html| html_to_text(&html))
        .filter(|t| !t.trim().is_empty())
        .collect();
    Ok(cut(
        normalize_whitespace(&texts.join("\n\n")),
        MAX_TEXT_CHARS,
    ))
}

/// mailparser's address `text`: `Name <address>` per mailbox.
fn address_text(address: Option<&mail_parser::Address<'_>>) -> String {
    let Some(address) = address else {
        return String::new();
    };
    address
        .iter()
        .map(|addr| match (addr.name(), addr.address()) {
            (Some(name), Some(email)) => format!("{name} <{email}>"),
            (Some(name), None) => name.to_owned(),
            (None, Some(email)) => email.to_owned(),
            (None, None) => String::new(),
        })
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// `extractTextFromEmail`: subject, from, to, the text body and the HTML
/// body as text; an unparsable file is read as text (a mostly binary one,
/// an Outlook `.msg`, gets a note and its first 1000 characters).
pub(super) fn email(bytes: &[u8]) -> String {
    let lossy = || String::from_utf8_lossy(bytes).into_owned();
    let Some(message) = mail_parser::MessageParser::default().parse(bytes) else {
        let decoded = lossy();
        let total = decoded.chars().count();
        let printable = decoded
            .chars()
            .filter(|c| matches!(c, '\x20'..='\x7e' | '\n' | '\r' | '\t'))
            .count();
        if total > 0 && (printable as f64) / (total as f64) < 0.5 {
            let head = cut(decoded, 1000);
            return normalize_whitespace(&format!(
                "[Outlook MSG file - content extraction may be limited]\n\n{head}"
            ));
        }
        return normalize_whitespace(&decoded);
    };
    let mut parts = Vec::new();
    if let Some(subject) = message.subject().filter(|s| !s.is_empty()) {
        parts.push(format!("Subject: {subject}"));
    }
    let from = address_text(message.from());
    if !from.is_empty() {
        parts.push(format!("From: {from}"));
    }
    let to = address_text(message.to());
    if !to.is_empty() {
        parts.push(format!("To: {to}"));
    }
    if let Some(text) = message.body_text(0).filter(|t| !t.is_empty()) {
        parts.push(text.into_owned());
    }
    let html =
        message
            .html_body
            .iter()
            .find_map(|id| match &message.parts.get(*id as usize)?.body {
                mail_parser::PartType::Html(html) => Some(html.to_string()),
                _ => None,
            });
    if let Some(html) = html {
        parts.push(html_to_text(&html));
    }
    let result = normalize_whitespace(&parts.join("\n\n"));
    if result.trim().is_empty() {
        return normalize_whitespace(&lossy());
    }
    result
}

/// SheetJS SSF: the displayed text of a number under a format code.
// ponytail: the subset knowledge files use (General, digit / grouping /
// decimal / percent patterns, dates and times); fractions, scientific and
// interleaved-literal patterns print as General.
pub(crate) mod numfmt {
    use chatbotx_core::js::number_to_string;

    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    const DAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];

    /// One lexed piece of a format section.
    #[derive(Debug, PartialEq)]
    enum Piece {
        Literal(String),
        Raw(char),
    }

    fn lex(section: &str) -> Vec<Piece> {
        let mut out = Vec::new();
        let mut chars = section.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' => {
                    let lit: String = chars.by_ref().take_while(|c| *c != '"').collect();
                    out.push(Piece::Literal(lit));
                }
                '\\' => {
                    if let Some(next) = chars.next() {
                        out.push(Piece::Literal(next.to_string()));
                    }
                }
                '_' => {
                    chars.next();
                    out.push(Piece::Literal(" ".to_owned()));
                }
                '*' => {
                    chars.next();
                }
                '[' => {
                    let inner: String = chars.by_ref().take_while(|c| *c != ']').collect();
                    // `[h]` / `[mm]` / `[ss]` elapsed time reads as the plain token.
                    if !inner.is_empty()
                        && inner
                            .chars()
                            .all(|c| matches!(c.to_ascii_lowercase(), 'h' | 'm' | 's'))
                    {
                        out.extend(inner.chars().map(Piece::Raw));
                    }
                }
                other => out.push(Piece::Raw(other)),
            }
        }
        out
    }

    fn sections(code: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let (mut start, mut quoted, mut bracket) = (0, false, false);
        for (i, c) in code.char_indices() {
            match c {
                '"' if !bracket => quoted = !quoted,
                '[' if !quoted => bracket = true,
                ']' if !quoted => bracket = false,
                ';' if !quoted && !bracket => {
                    out.push(&code[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        out.push(&code[start..]);
        out
    }

    /// SSF `General`.
    pub fn general(v: f64) -> String {
        if v.fract() == 0.0 {
            // `expandLargeIntegerCells`: every digit of a large integer.
            return if v.abs() >= 1e21 {
                format!("{v:.0}")
            } else {
                number_to_string(v)
            };
        }
        let a = v.abs();
        if !(1e-4..1e11).contains(&a) {
            return number_to_string(v);
        }
        let int_digits = if a < 1.0 {
            1
        } else {
            a.log10().floor() as usize + 1
        };
        let decimals = 10usize.saturating_sub(int_digits);
        let fixed = format!("{v:.decimals$}");
        let trimmed = if fixed.contains('.') {
            fixed.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            fixed
        };
        if trimmed == "-0" {
            "0".to_owned()
        } else {
            trimmed
        }
    }

    pub fn format(v: f64, code: &str) -> String {
        let all = sections(code);
        let (section, v, sign) = if v < 0.0 && all.len() >= 2 {
            (all[1], -v, "")
        } else if v == 0.0 && all.len() >= 3 {
            (all[2], v, "")
        } else {
            (all[0], v.abs(), if v < 0.0 { "-" } else { "" })
        };
        let trimmed = section.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("general") || trimmed == "@" {
            return format!("{sign}{}", general(v));
        }
        let pieces = lex(section);
        let is_date = pieces.iter().any(|p| matches!(p, Piece::Raw(c) if matches!(c.to_ascii_lowercase(), 'y' | 'd' | 'h' | 's' | 'm')));
        if is_date {
            return date(v, &pieces).unwrap_or_else(|| format!("{sign}{}", general(v)));
        }
        number(v, &pieces).map_or_else(|| format!("{sign}{}", general(v)), |s| format!("{sign}{s}"))
    }

    fn number(mut v: f64, pieces: &[Piece]) -> Option<String> {
        let placeholder = |p: &Piece| matches!(p, Piece::Raw('0' | '#' | '?'));
        let first = pieces.iter().position(placeholder)?;
        let last = pieces.iter().rposition(placeholder)?;
        let pattern: String = pieces[first..=last]
            .iter()
            .map(|p| match p {
                Piece::Raw(c) if matches!(c, '0' | '#' | '?' | ',' | '.') => Some(*c),
                _ => None,
            })
            .collect::<Option<String>>()?;
        let affix = |range: &[Piece]| {
            range
                .iter()
                .map(|p| match p {
                    Piece::Literal(s) => s.clone(),
                    Piece::Raw(c) => c.to_string(),
                })
                .collect::<String>()
        };
        let mut prefix = affix(&pieces[..first]);
        let mut suffix = affix(&pieces[last + 1..]);
        if prefix.contains(['E', 'e', '/']) || suffix.contains(['E', 'e', '/']) {
            return None;
        }
        // Thousands scaling: commas right after the last placeholder.
        while let Some(rest) = suffix.strip_prefix(',') {
            v /= 1000.0;
            suffix = rest.to_owned();
        }
        let percents = prefix.matches('%').count() + suffix.matches('%').count();
        for _ in 0..percents {
            v *= 100.0;
        }
        let (int_pat, dec_pat) = pattern.split_once('.').unwrap_or((&pattern, ""));
        let decimals = dec_pat
            .chars()
            .filter(|c| matches!(c, '0' | '#' | '?'))
            .count();
        let min_decimals = dec_pat.chars().filter(|c| *c == '0').count();
        let fixed = format!("{v:.decimals$}");
        let (int_digits, dec_digits) = fixed.split_once('.').unwrap_or((&fixed, ""));
        let min_int = int_pat.chars().filter(|c| *c == '0').count();
        let mut int_digits = int_digits.trim_start_matches('0').to_owned();
        while int_digits.len() < min_int {
            int_digits.insert(0, '0');
        }
        if int_pat.contains(',') {
            let bytes = int_digits.as_bytes();
            let mut grouped = String::new();
            for (i, b) in bytes.iter().enumerate() {
                if i > 0 && (bytes.len() - i) % 3 == 0 {
                    grouped.push(',');
                }
                grouped.push(*b as char);
            }
            int_digits = grouped;
        }
        let mut dec = dec_digits.to_owned();
        while dec.len() > min_decimals && dec.ends_with('0') {
            dec.pop();
        }
        prefix.push_str(&int_digits);
        if !dec.is_empty() || (dec_pat.contains('0')) {
            prefix.push('.');
            prefix.push_str(&dec);
        } else if pattern.ends_with('.') {
            prefix.push('.');
        }
        prefix.push_str(&suffix);
        Some(prefix)
    }

    /// Serial day number to (y, m, d, weekday, h, min, s) in the 1900 date
    /// system (with Excel's 1900-02-29).
    fn parts(v: f64) -> Option<(i32, u32, u32, u32, u32, u32, u32)> {
        use chrono::{Datelike as _, NaiveDate};
        if !(0.0..2_958_466.0).contains(&v) {
            return None;
        }
        let mut days = v.floor() as i64;
        let mut secs = ((v - v.floor()) * 86_400.0).round() as i64;
        if secs >= 86_400 {
            secs -= 86_400;
            days += 1;
        }
        let (y, m, d, wd) = if days == 60 {
            (1900, 2, 29, 3)
        } else {
            let base = NaiveDate::from_ymd_opt(1899, 12, if days > 60 { 30 } else { 31 })?;
            let date = base.checked_add_days(chrono::Days::new(days as u64))?;
            (
                date.year(),
                date.month(),
                date.day(),
                date.weekday().num_days_from_sunday(),
            )
        };
        let (h, mi, s) = (
            (secs / 3600) as u32,
            ((secs / 60) % 60) as u32,
            (secs % 60) as u32,
        );
        Some((y, m, d, wd, h, mi, s))
    }

    fn date(v: f64, pieces: &[Piece]) -> Option<String> {
        let (y, mo, d, wd, h, mi, s) = parts(v)?;
        // Runs of the same token letter.
        let mut runs: Vec<(char, usize)> = Vec::new();
        let mut literals: Vec<Option<String>> = Vec::new();
        let raw: Vec<char> = pieces
            .iter()
            .map(|p| match p {
                Piece::Raw(c) => *c,
                Piece::Literal(_) => '\0',
            })
            .collect();
        let mut i = 0;
        while i < pieces.len() {
            match &pieces[i] {
                Piece::Literal(s) => {
                    runs.push(('\0', 0));
                    literals.push(Some(s.clone()));
                    i += 1;
                }
                Piece::Raw(c) => {
                    let rest: String = raw[i..].iter().take(5).collect();
                    if rest.to_ascii_uppercase().starts_with("AM/PM") {
                        runs.push(('a', 5));
                        literals.push(None);
                        i += 5;
                        continue;
                    }
                    if rest.to_ascii_uppercase().starts_with("A/P") {
                        runs.push(('p', 3));
                        literals.push(None);
                        i += 3;
                        continue;
                    }
                    let lower = c.to_ascii_lowercase();
                    if matches!(lower, 'y' | 'm' | 'd' | 'h' | 's') {
                        let mut n = 0;
                        while i + n < raw.len() && raw[i + n].to_ascii_lowercase() == lower {
                            n += 1;
                        }
                        runs.push((lower, n));
                        literals.push(None);
                        i += n;
                    } else {
                        runs.push(('\0', 0));
                        literals.push(Some(c.to_string()));
                        i += 1;
                    }
                }
            }
        }
        let twelve = runs.iter().any(|(c, _)| matches!(c, 'a' | 'p'));
        let tokens: Vec<usize> = (0..runs.len()).filter(|&i| runs[i].0 != '\0').collect();
        let mut out = String::new();
        for (index, (c, n)) in runs.iter().enumerate() {
            if let Some(lit) = &literals[index] {
                out.push_str(lit);
                continue;
            }
            let pos = tokens.iter().position(|&t| t == index).unwrap_or(0);
            let prev = pos.checked_sub(1).map(|p| runs[tokens[p]].0);
            let next = tokens.get(pos + 1).map(|&t| runs[t].0);
            let hour12 = if h % 12 == 0 { 12 } else { h % 12 };
            let pad = |value: u32, width: usize| format!("{value:0width$}");
            let text = match c {
                'y' if *n <= 2 => pad((y % 100) as u32, 2),
                'y' => y.to_string(),
                'm' if *n <= 2 && (prev == Some('h') || next == Some('s')) => pad(mi, *n),
                'm' if *n <= 2 => pad(mo, *n),
                'm' if *n == 3 => MONTHS[mo as usize - 1][..3].to_owned(),
                'm' if *n == 4 => MONTHS[mo as usize - 1].to_owned(),
                'm' => MONTHS[mo as usize - 1][..1].to_owned(),
                'd' if *n <= 2 => pad(d, *n),
                'd' if *n == 3 => DAYS[wd as usize][..3].to_owned(),
                'd' => DAYS[wd as usize].to_owned(),
                'h' => pad(if twelve { hour12 } else { h }, (*n).min(2)),
                's' => pad(s, (*n).min(2)),
                'a' => if h < 12 { "AM" } else { "PM" }.to_owned(),
                'p' => if h < 12 { "A" } else { "P" }.to_owned(),
                _ => String::new(),
            };
            out.push_str(&text);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    //! `text-extractor.test.ts` (XLSX, PDF) and one case per other format.
    use chatbotx_business::import::xlsx::zip;

    use super::super::chunk::{Document, Sheet};
    use super::super::extract::{Options, knowledge_document, text_from_bytes};
    use super::*;

    const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

    /// A workbook of `(name, <sheetData> rows)` and the `styles.xml` body.
    fn workbook(sheets: &[(&str, String)], styles: &str) -> Vec<u8> {
        let mut parts: Vec<(String, String)> = Vec::new();
        let mut list = String::new();
        let mut rels = String::new();
        for (i, (name, rows)) in sheets.iter().enumerate() {
            let n = i + 1;
            list.push_str(&format!(
                r#"<sheet name="{name}" sheetId="{n}" r:id="rId{n}"/>"#
            ));
            rels.push_str(&format!(
                r#"<Relationship Id="rId{n}" Target="worksheets/sheet{n}.xml"/>"#
            ));
            parts.push((
                format!("xl/worksheets/sheet{n}.xml"),
                format!("<worksheet><sheetData>{rows}</sheetData></worksheet>"),
            ));
        }
        parts.push((
            "xl/workbook.xml".into(),
            format!("<workbook><sheets>{list}</sheets></workbook>"),
        ));
        parts.push((
            "xl/_rels/workbook.xml.rels".into(),
            format!("<Relationships>{rels}</Relationships>"),
        ));
        parts.push((
            "xl/styles.xml".into(),
            format!("<styleSheet>{styles}</styleSheet>"),
        ));
        let parts: Vec<(&str, String)> =
            parts.iter().map(|(n, c)| (n.as_str(), c.clone())).collect();
        zip(&parts)
    }

    fn s(r: &str, text: &str) -> String {
        format!(r#"<c r="{r}" t="inlineStr"><is><t>{text}</t></is></c>"#)
    }

    fn n(r: &str, v: &str, style: usize) -> String {
        format!(r#"<c r="{r}" s="{style}"><v>{v}</v></c>"#)
    }

    fn row(r: usize, cells: &[String]) -> String {
        format!(r#"<row r="{r}">{}</row>"#, cells.concat())
    }

    fn opts(name: &str, max: usize) -> Options {
        Options {
            source_name: name.into(),
            max_text_chars: max,
            force_prose: false,
            tabular_header_row: 1,
        }
    }

    #[test]
    fn names_unnamed_columns_and_skips_sheets_without_data_rows() {
        let bytes = workbook(
            &[
                ("Empty", row(1, &[s("A1", "Only"), s("B1", "Header")])),
                (
                    "Prices",
                    row(1, &[s("A1", "Product"), s("B1", ""), s("C1", "Price")])
                        + &row(2, &[s("A2", "Basic"), s("B2", "x"), n("C2", "199000", 0)]),
                ),
            ],
            "",
        );
        assert_eq!(
            knowledge_document(&bytes, XLSX, &opts("prices.xlsx", MAX_TEXT_CHARS)).unwrap(),
            Document::Tabular {
                sheets: vec![Sheet {
                    name: "Prices".into(),
                    headers: vec!["Product".into(), "Column 2".into(), "Price".into()],
                    rows: vec![vec!["Basic".into(), "x".into(), "199000".into()]],
                }],
                truncated: false,
            }
        );
    }

    const STYLES: &str = r#"<numFmts><numFmt numFmtId="164" formatCode="dd/mm/yyyy"/><numFmt numFmtId="165" formatCode="0000000000"/></numFmts><cellStyleXfs><xf numFmtId="3"/></cellStyleXfs><cellXfs><xf numFmtId="0"/><xf numFmtId="164"/><xf numFmtId="165"/><xf numFmtId="3"/><xf numFmtId="9"/></cellXfs>"#;

    fn rows_of(doc: Document) -> Vec<Vec<String>> {
        match doc {
            Document::Tabular { sheets, .. } => sheets.into_iter().flat_map(|s| s.rows).collect(),
            other => panic!("not tabular: {other:?}"),
        }
    }

    #[test]
    fn reads_xlsx_dates_as_displayed_instead_of_excel_serial_numbers() {
        let bytes = workbook(
            &[(
                "Orders",
                row(1, &[s("A1", "Date")]) + &row(2, &[n("A2", "45306", 1)]),
            )],
            STYLES,
        );
        let doc = knowledge_document(&bytes, XLSX, &opts("orders.xlsx", MAX_TEXT_CHARS)).unwrap();
        assert_eq!(rows_of(doc), vec![vec!["15/01/2024".to_owned()]]);
    }

    #[test]
    fn prints_every_digit_of_a_large_general_integer_but_keeps_explicit_formats() {
        let header: Vec<String> = ["Id", "Padded", "Grouped", "Rate", "Small"]
            .iter()
            .zip(["A1", "B1", "C1", "D1", "E1"])
            .map(|(t, r)| s(r, t))
            .collect();
        let values = [
            n("A2", "123456789012", 0),
            n("B2", "901234567", 2),
            n("C2", "150000000000", 3),
            n("D2", "0.256", 4),
            n("E2", "42", 0),
        ];
        let bytes = workbook(&[("Orders", row(1, &header) + &row(2, &values))], STYLES);
        let doc = knowledge_document(&bytes, XLSX, &opts("orders.xlsx", MAX_TEXT_CHARS)).unwrap();
        assert_eq!(
            rows_of(doc),
            vec![
                vec!["123456789012", "0901234567", "150,000,000,000", "26%", "42"]
                    .into_iter()
                    .map(String::from)
                    .collect::<Vec<_>>()
            ]
        );
    }

    #[test]
    fn stops_a_20000_row_workbook_exactly_at_the_text_budget() {
        let mut data = row(1, &[s("A1", "SKU"), s("B1", "Name")]);
        for i in 0..20_000 {
            let r = i + 2;
            data.push_str(&row(
                r,
                &[
                    s(&format!("A{r}"), &format!("sku-{i}")),
                    s(&format!("B{r}"), &format!("product {i}")),
                ],
            ));
        }
        let bytes = workbook(&[("Catalog", data)], "");
        let doc = knowledge_document(&bytes, XLSX, &opts("catalog.xlsx", 10_000)).unwrap();
        let Document::Tabular { sheets, truncated } = doc else {
            panic!("not tabular")
        };
        assert!(truncated);
        let len: usize = sheets
            .iter()
            .flat_map(|s| s.headers.iter().chain(s.rows.iter().flatten()))
            .map(|c| c.encode_utf16().count())
            .sum();
        assert_eq!(len, 10_000);
        assert!(sheets[0].rows.len() < 20_000);
    }

    #[test]
    fn xlsx_prose_is_header_value_blocks() {
        let bytes = workbook(
            &[(
                "S",
                row(1, &[s("A1", "Name"), s("B1", "Name"), s("C1", "")])
                    + &row(2, &[s("A2", "a"), n("B2", "1.5", 1), s("C2", "z")])
                    + &row(4, &[s("A4", "b")]),
            )],
            STYLES,
        );
        assert_eq!(
            text_from_bytes(&bytes, XLSX, MAX_TEXT_CHARS).unwrap(),
            "Name: a\nName_1: 1.5\n__EMPTY: z\n\nName: b"
        );
    }

    #[test]
    fn number_formats() {
        assert_eq!(numfmt::format(1234.5, "#,##0.00"), "1,234.50");
        assert_eq!(numfmt::format(-3.0, "0"), "-3");
        assert_eq!(numfmt::format(0.5, "0.0%"), "50.0%");
        assert_eq!(numfmt::format(45306.75, "m/d/yy h:mm"), "1/15/24 18:00");
        assert_eq!(numfmt::format(45306.75, "h:mm AM/PM"), "6:00 PM");
        assert_eq!(numfmt::format(45306.0, "d-mmm-yy"), "15-Jan-24");
        assert_eq!(numfmt::format(1.0 / 3.0, "General"), "0.333333333");
        assert_eq!(numfmt::format(12.0, "\"$\"#,##0"), "$12");
    }

    #[test]
    fn docx_paragraphs_tabs_and_entities() {
        let xml = r#"<w:document><w:body><w:p><w:pPr><w:tabs><w:tab w:val="left" w:pos="720"/></w:tabs></w:pPr><w:r><w:t>Hello</w:t><w:tab/><w:t xml:space="preserve">World &amp; co</w:t></w:r></w:p><w:p><w:r><w:t>Second</w:t><w:br/><w:t>line</w:t></w:r></w:p></w:body></w:document>"#;
        let bytes = zip(&[("word/document.xml", xml.to_owned())]);
        assert_eq!(
            text_from_bytes(
                &bytes,
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                MAX_TEXT_CHARS
            )
            .unwrap(),
            "Hello World & co\n\nSecond\nline"
        );
    }

    #[test]
    fn pptx_slides_in_number_order_then_notes() {
        let slide = |t: &str| {
            format!("<p:sld><a:p><a:r><a:t>{t}</a:t></a:r><a:r><a:t>!</a:t></a:r></a:p></p:sld>")
        };
        let bytes = zip(&[
            ("ppt/slides/slide10.xml", slide("Ten")),
            ("ppt/slides/slide2.xml", slide("Two")),
            ("ppt/notesSlides/notesSlide1.xml", slide("Note")),
            ("ppt/slides/slide1.xml", slide("One")),
        ]);
        assert_eq!(
            text_from_bytes(
                &bytes,
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                MAX_TEXT_CHARS
            )
            .unwrap(),
            "One !\n\nTwo !\n\nTen !\n\nNote !"
        );
    }

    #[test]
    fn epub_follows_the_spine() {
        let bytes = zip(&[
            ("META-INF/container.xml", r#"<rootfile full-path="OEBPS/content.opf"/>"#.to_owned()),
            (
                "OEBPS/content.opf",
                r#"<manifest><item id="c2" href="b.xhtml"/><item href="a.xhtml" id="c1"/></manifest><spine><itemref idref="c1"/><itemref idref="c2"/></spine>"#.to_owned(),
            ),
            ("OEBPS/b.xhtml", "<html><body><p>Second</p></body></html>".to_owned()),
            ("OEBPS/a.xhtml", "<html><body><p>First</p></body></html>".to_owned()),
        ]);
        assert_eq!(
            text_from_bytes(&bytes, "application/epub+zip", MAX_TEXT_CHARS).unwrap(),
            "First\n\nSecond"
        );
    }

    #[test]
    fn email_headers_and_body() {
        let raw = b"Subject: Hello\r\nFrom: Ann <ann@example.com>\r\nTo: bob@example.com\r\nContent-Type: text/plain\r\n\r\nBody text\r\n";
        assert_eq!(
            text_from_bytes(raw, "message/rfc822", MAX_TEXT_CHARS).unwrap(),
            "Subject: Hello\n\nFrom: Ann <ann@example.com>\n\nTo: bob@example.com\n\nBody text"
        );
    }

    #[test]
    fn ppt_reads_utf16_runs() {
        let mut bytes: Vec<u8> = "Hello slide"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        bytes.extend([1, 0]);
        bytes.extend("ab".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(
            text_from_bytes(&bytes, "application/vnd.ms-powerpoint", MAX_TEXT_CHARS).unwrap(),
            "Hello slide"
        );
    }

    #[test]
    fn a_broken_pdf_is_a_parse_failure() {
        assert_eq!(
            pdf(b"%PDF-1.4 not really").unwrap_err(),
            "PDF parsing failed"
        );
    }
}
