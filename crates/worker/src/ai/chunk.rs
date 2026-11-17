//! `ai-agent/lib/chunkers` (prose, markdown, tabular). Lengths and slices
//! are UTF-16 code units, as JavaScript strings, so the chunks (and their
//! ids) match the original.

use std::sync::LazyLock;

use regex::Regex;

/// `TabularSheet`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sheet {
    pub name: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// `KnowledgeDocument`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Document {
    Tabular {
        sheets: Vec<Sheet>,
        truncated: bool,
    },
    Markdown(String),
    /// `fallback` = the tabular parse error the prose replaces.
    Prose {
        text: String,
        fallback: Option<String>,
    },
}

impl Document {
    /// `measureExtractedChars`.
    pub fn extracted_chars(&self) -> usize {
        match self {
            Document::Tabular { sheets, .. } => sheets
                .iter()
                .map(|s| {
                    s.headers.iter().map(|h| u16len(h)).sum::<usize>()
                        + s.rows.iter().flatten().map(|c| u16len(c)).sum::<usize>()
                })
                .sum(),
            Document::Markdown(text) | Document::Prose { text, .. } => u16len(text),
        }
    }
}

/// `ChunkResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunked {
    pub chunks: Vec<String>,
    pub strategy: &'static str,
    pub chunk_size: i64,
    pub overlap_size: i64,
}

pub fn u16len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `s.slice(from, to)` in UTF-16 units (a split pair becomes U+FFFD).
fn slice16(units: &[u16], from: usize, to: usize) -> String {
    String::from_utf16_lossy(&units[from.min(units.len())..to.min(units.len())])
}

/// `String.prototype.trim` (Rust's `trim` keeps U+FEFF).
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
}

/// `chunkKnowledgeDocument`. `Err` = `Structured chunk size must be at least 100`.
pub fn chunk_document(doc: &Document, chunk_size: i64, overlap: i64) -> Result<Chunked, String> {
    Ok(match doc {
        Document::Prose { text, .. } => Chunked {
            chunks: prose(text, chunk_size, overlap),
            strategy: "prose",
            chunk_size,
            overlap_size: overlap,
        },
        Document::Tabular { sheets, .. } => Chunked {
            chunks: tabular(sheets, chunk_size)?,
            strategy: "tabular",
            chunk_size,
            overlap_size: 0,
        },
        Document::Markdown(text) => Chunked {
            chunks: markdown(text, chunk_size, overlap)?,
            strategy: "markdown",
            chunk_size,
            overlap_size: overlap,
        },
    })
}

/// `chunkProse`: fixed windows with `overlap` units shared.
pub fn prose(text: &str, chunk_size: i64, overlap: i64) -> Vec<String> {
    let mut chunks = Vec::new();
    if text.is_empty() || chunk_size <= 0 {
        return chunks;
    }
    let units: Vec<u16> = text.encode_utf16().collect();
    let (size, overlap) = (chunk_size as usize, overlap.max(0) as usize);
    let mut start = 0;
    while start < units.len() {
        let end = (start + size).min(units.len());
        let piece = slice16(&units, start, end);
        let piece = js_trim(&piece);
        if !piece.is_empty() {
            chunks.push(piece.to_owned());
        }
        if end == units.len() {
            break;
        }
        start = end.saturating_sub(overlap);
    }
    chunks
}

const MIN_STRUCTURED_CHUNK_SIZE: i64 = 100;

fn assert_structured(chunk_size: i64) -> Result<(), String> {
    if chunk_size < MIN_STRUCTURED_CHUNK_SIZE {
        return Err(format!(
            "Structured chunk size must be at least {MIN_STRUCTURED_CHUNK_SIZE}"
        ));
    }
    Ok(())
}

fn max_label(chunk_size: i64) -> usize {
    (chunk_size / 4).max(0) as usize
}

/// `truncateLabel`.
fn truncate_label(text: &str, max: usize) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.len() <= max {
        return text.to_owned();
    }
    if max <= 1 {
        return if max == 1 {
            "…".into()
        } else {
            String::new()
        };
    }
    format!("{}…", slice16(&units, 0, max - 1))
}

static ATX_HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(#{1,6})\s+(.+?)\s*#*\s*$").expect("heading regex"));

struct Section {
    path: Vec<String>,
    body: String,
}

fn path_label(path: &[String], max: usize) -> String {
    let full = path.join(" > ");
    if u16len(&full) <= max {
        return full;
    }
    let mut kept = path.to_vec();
    while kept.len() > 1 && u16len(&format!("… > {}", kept.join(" > "))) > max {
        kept.remove(0);
    }
    let prefix = if kept.len() == path.len() {
        ""
    } else {
        "… > "
    };
    truncate_label(&format!("{prefix}{}", kept.join(" > ")), max)
}

fn parse_sections(text: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut body: Vec<&str> = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut fence: Option<&str> = None;
    let flush = |body: &mut Vec<&str>, path: &Vec<String>, sections: &mut Vec<Section>| {
        let joined = js_trim(&body.join("\n")).to_owned();
        if !joined.is_empty() {
            sections.push(Section {
                path: path.clone(),
                body: joined,
            });
        }
        body.clear();
    };
    for line in text.split('\n') {
        let trimmed = line.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        let this = ["```", "~~~"].into_iter().find(|f| trimmed.starts_with(f));
        if let Some(this) = this {
            match fence {
                None => fence = Some(this),
                Some(open) if open == this => fence = None,
                Some(_) => {}
            }
            body.push(line);
            continue;
        }
        let heading = if fence.is_some() {
            None
        } else {
            ATX_HEADING.captures(line)
        };
        let Some(heading) = heading else {
            body.push(line);
            continue;
        };
        flush(&mut body, &path, &mut sections);
        let level = heading[1].len();
        let title = js_trim(&heading[2]).to_owned();
        // `stack.splice(level - 1); stack[level - 1] = title` (holes = "").
        stack.truncate(level - 1);
        stack.resize(level - 1, String::new());
        stack.push(title);
        path = stack.iter().filter(|s| !s.is_empty()).cloned().collect();
    }
    flush(&mut body, &path, &mut sections);
    sections
}

/// `chunkMarkdown`: sections under their heading path; no heading = prose.
pub fn markdown(text: &str, chunk_size: i64, overlap: i64) -> Result<Vec<String>, String> {
    let sections = parse_sections(text);
    if !sections.iter().any(|s| !s.path.is_empty()) {
        return Ok(prose(text, chunk_size, overlap));
    }
    assert_structured(chunk_size)?;
    let size = chunk_size as usize;
    let limit = max_label(chunk_size);
    let mut chunks = Vec::new();
    let mut current = String::new();
    for section in &sections {
        let label = if section.path.is_empty() {
            String::new()
        } else {
            path_label(&section.path, limit)
        };
        let rendered = if label.is_empty() {
            section.body.clone()
        } else {
            format!("{label}\n{}", section.body)
        };
        let sep = if current.is_empty() { "" } else { "\n\n" };
        if u16len(&current) + sep.len() + u16len(&rendered) <= size {
            current = format!("{current}{sep}{rendered}");
            continue;
        }
        if !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        if u16len(&rendered) <= size {
            current = rendered;
            continue;
        }
        if label.is_empty() {
            chunks.extend(prose(&section.body, chunk_size, overlap));
            continue;
        }
        let budget = chunk_size - u16len(&label) as i64 - 1;
        let actual = overlap.min(budget.div_euclid(2));
        for piece in prose(&section.body, budget, actual) {
            chunks.push(format!("{label}\n{piece}"));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    Ok(chunks)
}

struct Cell<'a> {
    header: String,
    value: &'a str,
}

fn oversized_row(cells: &[Cell<'_>], context: &str, chunk_size: i64, limit: usize) -> Vec<String> {
    let Some(first) = cells.first() else {
        return Vec::new();
    };
    let whole = format!("{}: {}", first.header, first.value);
    let key = truncate_label(&whole, limit);
    let rest = if key == whole { &cells[1..] } else { cells };
    let prefix = format!("{context}\n\n{key}\n");
    let budget = chunk_size - u16len(&prefix) as i64;
    let mut chunks = Vec::new();
    let mut current = String::new();
    for cell in rest {
        let line = format!("{}: {}", cell.header, cell.value);
        let sep = if current.is_empty() { "" } else { "\n" };
        if (u16len(&current) + sep.len() + u16len(&line)) as i64 <= budget {
            current = format!("{current}{sep}{line}");
            continue;
        }
        if !current.is_empty() {
            chunks.push(format!("{prefix}{}", std::mem::take(&mut current)));
        }
        if u16len(&line) as i64 <= budget {
            current = line;
            continue;
        }
        let cell_prefix = format!("{}: ", cell.header);
        for piece in prose(cell.value, budget - u16len(&cell_prefix) as i64, 0) {
            chunks.push(format!("{prefix}{cell_prefix}{piece}"));
        }
    }
    if !current.is_empty() {
        chunks.push(format!("{prefix}{current}"));
    }
    chunks
}

fn chunk_sheet(sheet: &Sheet, chunk_size: i64, limit: usize) -> Vec<String> {
    let context = format!(
        "Sheet: {}",
        truncate_label(&sheet.name, limit.saturating_sub(7).max(1))
    );
    let prefix = format!("{context}\n\n");
    let size = chunk_size as usize;
    let mut chunks = Vec::new();
    let mut current = String::new();
    for row in &sheet.rows {
        let cells: Vec<Cell<'_>> = row
            .iter()
            .enumerate()
            .filter(|(_, v)| !v.is_empty())
            .map(|(i, v)| Cell {
                header: truncate_label(
                    sheet
                        .headers
                        .get(i)
                        .filter(|h| !h.is_empty())
                        .cloned()
                        .unwrap_or_else(|| format!("Column {}", i + 1))
                        .as_str(),
                    limit,
                ),
                value: v,
            })
            .collect();
        if cells.is_empty() {
            continue;
        }
        let block = cells
            .iter()
            .map(|c| format!("{}: {}", c.header, c.value))
            .collect::<Vec<_>>()
            .join("\n");
        let sep = if current.is_empty() { "" } else { "\n\n" };
        if u16len(&prefix) + u16len(&current) + sep.len() + u16len(&block) <= size {
            current = format!("{current}{sep}{block}");
            continue;
        }
        if !current.is_empty() {
            chunks.push(format!("{prefix}{}", std::mem::take(&mut current)));
        }
        if u16len(&prefix) + u16len(&block) <= size {
            current = block;
            continue;
        }
        chunks.extend(oversized_row(&cells, &context, chunk_size, limit));
    }
    if !current.is_empty() {
        chunks.push(format!("{prefix}{current}"));
    }
    chunks
}

/// `chunkTabular`: rows as `header: value` blocks under the sheet name.
pub fn tabular(sheets: &[Sheet], chunk_size: i64) -> Result<Vec<String>, String> {
    assert_structured(chunk_size)?;
    let limit = max_label(chunk_size);
    Ok(sheets
        .iter()
        .flat_map(|s| chunk_sheet(s, chunk_size, limit))
        .collect())
}

#[cfg(test)]
mod tests {
    //! `chunkers.test.ts`.
    use super::*;

    fn digits(n: usize) -> String {
        (0..n).map(|i| char::from(b'0' + (i % 10) as u8)).collect()
    }

    /// The pre-Phase-1 `splitTextIntoChunks`, over chars (ASCII inputs).
    fn legacy(text: &str, size: usize, overlap: usize) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let mut out = Vec::new();
        if text.is_empty() || size == 0 {
            return out;
        }
        let mut start = 0;
        while start < chars.len() {
            let end = (start + size).min(chars.len());
            let piece: String = chars[start..end].iter().collect();
            if !piece.trim().is_empty() {
                out.push(piece.trim().to_owned());
            }
            if end == chars.len() {
                break;
            }
            start = end.saturating_sub(overlap);
        }
        out
    }

    fn sheet(name: &str, headers: &[&str], rows: &[&[&str]]) -> Sheet {
        Sheet {
            name: name.into(),
            headers: headers.iter().map(|s| s.to_string()).collect(),
            rows: rows
                .iter()
                .map(|r| r.iter().map(|s| s.to_string()).collect())
                .collect(),
        }
    }

    fn len(s: &str) -> usize {
        u16len(s)
    }

    #[test]
    fn chunk_prose_matches_the_legacy_chunker() {
        let ws = format!("  {}  ", "word   ".repeat(400));
        for (text, size, overlap) in [
            ("".to_owned(), 1000, 200),
            ("alpha beta gamma".to_owned(), 1000, 200),
            ("x".repeat(1000), 1000, 200),
            ("x".repeat(1001), 1000, 200),
            (digits(5432), 1000, 200),
            (ws, 1000, 200),
            (" alpha beta gamma ".to_owned(), 10, 0),
            ("alpha".to_owned(), 0, 0),
        ] {
            assert_eq!(
                prose(&text, size as i64, overlap as i64),
                legacy(&text, size, overlap)
            );
        }
    }

    #[test]
    fn chunk_tabular_renders_the_documented_chunk_format() {
        let chunks = tabular(
            &[sheet(
                "Bảng giá",
                &["Tên SP", "Giá", "Thời hạn"],
                &[
                    &["Gói Basic", "199.000", "6 tháng"],
                    &["Gói Pro", "499.000", "12 tháng"],
                ],
            )],
            1000,
        )
        .unwrap();
        assert_eq!(
            chunks,
            vec![
                [
                    "Sheet: Bảng giá",
                    "",
                    "Tên SP: Gói Basic",
                    "Giá: 199.000",
                    "Thời hạn: 6 tháng",
                    "",
                    "Tên SP: Gói Pro",
                    "Giá: 499.000",
                    "Thời hạn: 12 tháng",
                ]
                .join("\n")
            ]
        );
    }

    #[test]
    fn chunk_tabular_keeps_rows_and_column_names_together_without_crossing_sheets() {
        let chunks = tabular(
            &[
                sheet(
                    "Prices",
                    &["Product", "Price"],
                    &[&["Basic", "199000"], &["Pro", "499000"]],
                ),
                sheet("Stock", &["SKU", "Quantity"], &[&["sku-1", "12"]]),
            ],
            100,
        )
        .unwrap();
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].contains("Sheet: Prices") && chunks[0].contains("Product: Basic"));
        assert!(chunks[1].contains("Sheet: Stock") && !chunks[1].contains("Product"));
        assert!(chunks.iter().all(|c| len(c) <= 100));
    }

    #[test]
    fn chunk_tabular_skips_empty_cells_and_rows_and_names_unnamed_columns() {
        let chunks = tabular(
            &[sheet(
                "Sheet1",
                &["Name", "", "Note"],
                &[
                    &["Basic", "", "popular"],
                    &["", "", ""],
                    &["Pro", "extra", ""],
                ],
            )],
            1000,
        )
        .unwrap();
        assert_eq!(
            chunks,
            vec!["Sheet: Sheet1\n\nName: Basic\nNote: popular\n\nName: Pro\nColumn 2: extra"]
        );
    }

    #[test]
    fn chunk_tabular_never_splits_a_row_that_fits_a_chunk_across_two_chunks() {
        let rows: Vec<Vec<String>> = (0..200)
            .map(|i| vec![format!("product-{i}"), digits(30 + (i % 40))])
            .collect();
        let s = Sheet {
            name: "Catalog".into(),
            headers: vec!["Product".into(), "Description".into()],
            rows: rows.clone(),
        };
        let chunks = tabular(&[s], 1000).unwrap();
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(len(c) <= 1000 && c.starts_with("Sheet: Catalog\n\n"));
        }
        for r in &rows {
            let block = format!("Product: {}\nDescription: {}", r[0], r[1]);
            assert_eq!(chunks.iter().filter(|c| c.contains(&block)).count(), 1);
        }
    }

    #[test]
    fn chunk_tabular_splits_an_oversized_row_into_parts_of_consecutive_columns_that_repeat_the_key()
    {
        let headers: Vec<String> = (0..20).map(|i| format!("Col{i}")).collect();
        let row: Vec<String> = (0..20).map(|i| format!("{i}-{}", "v".repeat(40))).collect();
        let chunks = tabular(
            &[Sheet {
                name: "Wide".into(),
                headers: headers.clone(),
                rows: vec![row.clone()],
            }],
            200,
        )
        .unwrap();
        assert!(chunks.len() > 1 && chunks.len() < headers.len());
        for c in &chunks {
            assert!(len(c) <= 200 && c.starts_with("Sheet: Wide\n\nCol0: 0-"));
        }
        for (i, h) in headers.iter().enumerate() {
            let line = format!("{h}: {}", row[i]);
            assert!(chunks.iter().any(|c| c.split('\n').any(|l| l == line)));
        }
    }

    #[test]
    fn chunk_tabular_bounds_long_labels_and_keeps_every_character_of_a_long_key_cell() {
        let key = digits(5000);
        let chunks = tabular(
            &[Sheet {
                name: "s".repeat(500),
                headers: vec!["ID".into(), "Note".into()],
                rows: vec![vec![key.clone(), "short".into()]],
            }],
            1000,
        )
        .unwrap();
        assert!(chunks.iter().all(|c| len(c) <= 1000));
        let pieces: String = chunks
            .iter()
            .map(|c| c.split('\n').next_back().unwrap_or(""))
            .filter(|l| l.starts_with("ID: "))
            .map(|l| &l[4..])
            .collect();
        assert_eq!(pieces, key);
        assert!(chunks.last().unwrap().ends_with("\nNote: short"));
    }

    #[test]
    fn chunk_tabular_keeps_every_chunk_within_the_size_for_extreme_labels_and_cells() {
        for size in [100, 1000] {
            let long = digits(5000);
            let chunks = tabular(
                &[Sheet {
                    name: "n".repeat(500),
                    headers: vec!["k".repeat(500), "h".repeat(500)],
                    rows: vec![vec![digits(5000), long.clone()], vec!["short".into(), long]],
                }],
                size,
            )
            .unwrap();
            assert!(chunks.len() > 1);
            assert!(
                chunks
                    .iter()
                    .all(|c| len(c) <= size as usize && c.starts_with("Sheet: "))
            );
        }
    }

    #[test]
    fn chunk_tabular_rejects_structured_chunks_smaller_than_the_minimum() {
        assert!(
            tabular(&[], 99)
                .unwrap_err()
                .contains("Structured chunk size")
        );
    }

    #[test]
    fn chunk_markdown_renders_the_documented_chunk_format() {
        let text = [
            "# Chính sách",
            "## Đổi trả",
            "### Điều kiện",
            "- Sản phẩm còn nguyên tem, trong vòng 7 ngày",
            "- Có hóa đơn mua hàng",
            "### Phí",
            "Miễn phí đổi lần đầu, từ lần hai phí 30.000đ",
        ]
        .join("\n");
        assert_eq!(
            markdown(&text, 1000, 200).unwrap(),
            vec![
                [
                    "Chính sách > Đổi trả > Điều kiện",
                    "- Sản phẩm còn nguyên tem, trong vòng 7 ngày",
                    "- Có hóa đơn mua hàng",
                    "",
                    "Chính sách > Đổi trả > Phí",
                    "Miễn phí đổi lần đầu, từ lần hai phí 30.000đ",
                ]
                .join("\n")
            ]
        );
    }

    #[test]
    fn chunk_markdown_adds_nested_heading_paths_and_ignores_headings_inside_fenced_code() {
        let text = [
            "# Policy",
            "## Returns",
            "### Conditions",
            "The item must be unopened.",
            "```md",
            "# not a heading",
            "```",
            "~~~",
            "## also not a heading",
            "~~~",
        ]
        .join("\n");
        let chunks = markdown(&text, 1000, 20).unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].starts_with("Policy > Returns > Conditions\n"));
        assert!(
            chunks[0].contains("# not a heading") && chunks[0].contains("## also not a heading")
        );
    }

    #[test]
    fn chunk_markdown_builds_the_path_across_skipped_heading_levels() {
        assert_eq!(
            markdown("# Guide\n### Setup\nInstall it.\n", 1000, 0).unwrap(),
            vec!["Guide > Setup\nInstall it."]
        );
    }

    #[test]
    fn chunk_markdown_keeps_the_preamble_without_a_path_and_does_not_render_empty_parent_sections()
    {
        assert_eq!(
            markdown("Intro text.\n# A\n## B\nBody of B.", 1000, 0).unwrap(),
            vec!["Intro text.\n\nA > B\nBody of B."]
        );
    }

    #[test]
    fn chunk_markdown_keeps_a_full_path_that_fits_the_label_limit() {
        assert_eq!(
            markdown("# Policy\n## Returns rules\nKeep receipts.", 100, 0).unwrap(),
            vec!["Policy > Returns rules\nKeep receipts."]
        );
    }

    #[test]
    fn chunk_markdown_splits_a_long_section_and_repeats_its_path_on_every_piece() {
        let chunks = markdown(
            &format!("# Policy\n## Returns\n{}", digits(2500)),
            1000,
            200,
        )
        .unwrap();
        assert!(chunks.len() > 2);
        assert!(
            chunks
                .iter()
                .all(|c| len(c) <= 1000 && c.starts_with("Policy > Returns\n"))
        );
    }

    #[test]
    fn chunk_markdown_bounds_a_six_level_path_of_300_character_headings_and_keeps_the_deepest_heading()
     {
        let text = format!(
            "{}\n{}",
            (0..6)
                .map(|i| format!("{} {}{}", "#".repeat(i + 1), i + 1, "h".repeat(299)))
                .collect::<Vec<_>>()
                .join("\n"),
            "body ".repeat(80)
        );
        let chunks = markdown(&text, 100, 20).unwrap();
        assert!(chunks.len() > 1);
        assert!(
            chunks
                .iter()
                .all(|c| len(c) <= 100 && c.starts_with("… > 6h"))
        );
    }

    #[test]
    fn chunk_markdown_uses_prose_chunking_when_the_markdown_has_no_headings() {
        let text = "plain markdown text without headings".repeat(5);
        assert_eq!(markdown(&text, 30, 0).unwrap(), prose(&text, 30, 0));
    }

    #[test]
    fn chunk_markdown_rejects_structured_chunks_smaller_than_the_minimum() {
        assert!(
            markdown("# A\nbody", 99, 0)
                .unwrap_err()
                .contains("Structured chunk size")
        );
    }

    #[test]
    fn chunk_knowledge_document_reports_the_strategy_and_the_parameters_actually_used() {
        let p = chunk_document(
            &Document::Prose {
                text: "alpha".into(),
                fallback: None,
            },
            1000,
            200,
        )
        .unwrap();
        assert_eq!(
            (p.strategy, p.chunk_size, p.overlap_size),
            ("prose", 1000, 200)
        );
        let m = chunk_document(&Document::Markdown("# A\nbody".into()), 1000, 200).unwrap();
        assert_eq!(
            (m.strategy, m.chunk_size, m.overlap_size),
            ("markdown", 1000, 200)
        );
        let t = chunk_document(
            &Document::Tabular {
                sheets: vec![sheet("S", &["A"], &[&["1"]])],
                truncated: false,
            },
            1000,
            200,
        )
        .unwrap();
        assert_eq!(
            (t.strategy, t.chunk_size, t.overlap_size),
            ("tabular", 1000, 0)
        );
    }

    #[test]
    fn chunk_knowledge_document_yields_no_chunks_for_empty_documents() {
        for doc in [
            Document::Tabular {
                sheets: vec![],
                truncated: false,
            },
            Document::Prose {
                text: String::new(),
                fallback: None,
            },
            Document::Markdown(String::new()),
        ] {
            assert!(chunk_document(&doc, 1000, 200).unwrap().chunks.is_empty());
        }
    }
}
