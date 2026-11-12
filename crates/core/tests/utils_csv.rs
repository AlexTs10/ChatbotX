//! Port of `packages/utils/__tests__/csv.test.ts`. A cell is a `CsvCell`
//! (`null` and `undefined` are both `CsvCell::Empty`); the dialect is
//! explicit (`Always` is the TS default).

use chatbotx_core::utils::csv::{
    CsvCell, CsvDialect, UTF8_BOM, escape_csv_cell, quote_csv_field, to_csv_row,
};

fn cell(value: &str) -> CsvCell {
    CsvCell::from(value)
}

fn always(value: CsvCell) -> String {
    escape_csv_cell(&value, CsvDialect::Always)
}

fn minimal(value: CsvCell) -> String {
    escape_csv_cell(&value, CsvDialect::Minimal)
}

/// Port of: packages/utils/__tests__/csv.test.ts:5 "UTF8_BOM > encodes to the EF BB BF bytes Excel uses to detect UTF-8"
#[test]
fn utf8_bom_encodes_to_the_ef_bb_bf_bytes_excel_uses_to_detect_utf_8() {
    assert_eq!(UTF8_BOM.as_bytes(), &[0xef, 0xbb, 0xbf]);
}

/// Port of: packages/utils/__tests__/csv.test.ts:11 "quoteCsvField > always quotes and doubles embedded quotes, nothing else"
#[test]
fn quotecsvfield_always_quotes_and_doubles_embedded_quotes_nothing_else() {
    assert_eq!(quote_csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    // Trusted template values keep a leading + (no formula guard).
    assert_eq!(quote_csv_field("+14155550100"), "\"+14155550100\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:19 "escapeCsvCell > wraps a plain value in double quotes"
#[test]
fn escapecsvcell_wraps_a_plain_value_in_double_quotes() {
    assert_eq!(always(cell("hello world")), "\"hello world\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:23 "escapeCsvCell > renders "" as an empty quoted cell"
#[test]
fn escapecsvcell_renders_as_an_empty_quoted_cell() {
    assert_eq!(always(cell("")), "\"\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:23 "escapeCsvCell > renders null as an empty quoted cell"
#[test]
fn escapecsvcell_renders_null_as_an_empty_quoted_cell() {
    assert_eq!(always(CsvCell::Empty), "\"\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:23 "escapeCsvCell > renders undefined as an empty quoted cell"
#[test]
fn escapecsvcell_renders_undefined_as_an_empty_quoted_cell() {
    assert_eq!(always(CsvCell::Empty), "\"\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:31 "escapeCsvCell > stringifies numbers"
#[test]
fn escapecsvcell_stringifies_numbers() {
    assert_eq!(always(CsvCell::from(42)), "\"42\"");
    assert_eq!(always(CsvCell::from(0)), "\"0\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:36 "escapeCsvCell > escapes embedded double quotes by doubling them"
#[test]
fn escapecsvcell_escapes_embedded_double_quotes_by_doubling_them() {
    assert_eq!(always(cell("say \"hello\"")), "\"say \"\"hello\"\"\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:40 "escapeCsvCell > flattens a line break in "line1\nline2" to a space"
#[test]
fn escapecsvcell_flattens_a_line_break_in_line1_nline2_to_a_space() {
    assert_eq!(always(cell("line1\nline2")), "\"line1 line2\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:40 "escapeCsvCell > flattens a line break in "line1\r\nline2" to a space"
#[test]
fn escapecsvcell_flattens_a_line_break_in_line1_r_nline2_to_a_space() {
    assert_eq!(always(cell("line1\r\nline2")), "\"line1 line2\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:47 "escapeCsvCell > keeps Vietnamese text intact"
#[test]
fn escapecsvcell_keeps_vietnamese_text_intact() {
    assert_eq!(always(cell("Nguyễn Văn Bình")), "\"Nguyễn Văn Bình\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:51 "escapeCsvCell > guards formula injection: "=1+1" is prefixed with a single quote"
#[test]
fn escapecsvcell_guards_formula_injection_1_1_is_prefixed_with_a_single_quote() {
    assert_eq!(always(cell("=1+1")), "\"'=1+1\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:51 "escapeCsvCell > guards formula injection: "+1234" is prefixed with a single quote"
#[test]
fn escapecsvcell_guards_formula_injection_1234_is_prefixed_with_a_single_quote() {
    assert_eq!(always(cell("+1234")), "\"'+1234\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:51 "escapeCsvCell > guards formula injection: "-cmd" is prefixed with a single quote"
#[test]
fn escapecsvcell_guards_formula_injection_cmd_is_prefixed_with_a_single_quote() {
    assert_eq!(always(cell("-cmd")), "\"'-cmd\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:51 "escapeCsvCell > guards formula injection: "@SUM(A1)" is prefixed with a single quote"
#[test]
fn escapecsvcell_guards_formula_injection_sum_a1_is_prefixed_with_a_single_quote() {
    assert_eq!(always(cell("@SUM(A1)")), "\"'@SUM(A1)\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:51 "escapeCsvCell > guards formula injection: "\tleading tab" is prefixed with a single quote"
#[test]
fn escapecsvcell_guards_formula_injection_tleading_tab_is_prefixed_with_a_single_quote() {
    assert_eq!(always(cell("\tleading tab")), "\"'\tleading tab\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:51 "escapeCsvCell > guards formula injection: "\rcmd" is prefixed with a single quote"
#[test]
fn escapecsvcell_guards_formula_injection_rcmd_is_prefixed_with_a_single_quote() {
    assert_eq!(always(cell("\rcmd")), "\"'\rcmd\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:62 "escapeCsvCell > does not guard a formula character in the middle"
#[test]
fn escapecsvcell_does_not_guard_a_formula_character_in_the_middle() {
    assert_eq!(always(cell("a=1+1")), "\"a=1+1\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:66 "escapeCsvCell > guards a formula that also contains embedded quotes"
#[test]
fn escapecsvcell_guards_a_formula_that_also_contains_embedded_quotes() {
    assert_eq!(
        always(cell("=HYPERLINK(\"x\")")),
        "\"'=HYPERLINK(\"\"x\"\")\""
    );
}

/// Port of: packages/utils/__tests__/csv.test.ts:72 "escapeCsvCell in the minimal dialect > leaves a field bare when nothing needs quoting"
#[test]
fn escapecsvcell_in_the_minimal_dialect_leaves_a_field_bare_when_nothing_needs_quoting() {
    assert_eq!(minimal(cell("Ada Lovelace")), "Ada Lovelace");
    assert_eq!(minimal(CsvCell::Empty), "");
}

/// Port of: packages/utils/__tests__/csv.test.ts:77 "escapeCsvCell in the minimal dialect > quotes "a,b" because it holds a delimiter or quote"
#[test]
fn escapecsvcell_in_the_minimal_dialect_quotes_a_b_because_it_holds_a_delimiter_or_quote() {
    assert_eq!(minimal(cell("a,b")), "\"a,b\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:77 "escapeCsvCell in the minimal dialect > quotes "say \"hi\"" because it holds a delimiter or quote"
#[test]
fn escapecsvcell_in_the_minimal_dialect_quotes_say_hi_because_it_holds_a_delimiter_or_quote() {
    assert_eq!(minimal(cell("say \"hi\"")), "\"say \"\"hi\"\"\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:84 "escapeCsvCell in the minimal dialect > keeps the line break in "line1\nline2" inside a quoted field (RFC 4180)"
#[test]
fn escapecsvcell_in_the_minimal_dialect_keeps_the_line_break_in_line1_nline2_inside_a_quoted_field_rfc_4180()
 {
    assert_eq!(minimal(cell("line1\nline2")), "\"line1\nline2\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:84 "escapeCsvCell in the minimal dialect > keeps the line break in "line1\r\nline2" inside a quoted field (RFC 4180)"
#[test]
fn escapecsvcell_in_the_minimal_dialect_keeps_the_line_break_in_line1_r_nline2_inside_a_quoted_field_rfc_4180()
 {
    assert_eq!(minimal(cell("line1\r\nline2")), "\"line1\r\nline2\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:91 "escapeCsvCell in the minimal dialect > still guards formula injection: "+12025550101""
#[test]
fn escapecsvcell_in_the_minimal_dialect_still_guards_formula_injection_12025550101() {
    assert_eq!(minimal(cell("+12025550101")), "'+12025550101");
}

/// Port of: packages/utils/__tests__/csv.test.ts:91 "escapeCsvCell in the minimal dialect > still guards formula injection: "@cmd""
#[test]
fn escapecsvcell_in_the_minimal_dialect_still_guards_formula_injection_cmd() {
    assert_eq!(minimal(cell("@cmd")), "'@cmd");
}

/// Port of: packages/utils/__tests__/csv.test.ts:91 "escapeCsvCell in the minimal dialect > still guards formula injection: "=SUM(1,1)""
#[test]
fn escapecsvcell_in_the_minimal_dialect_still_guards_formula_injection_sum_1_1() {
    assert_eq!(minimal(cell("=SUM(1,1)")), "\"'=SUM(1,1)\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:91 "escapeCsvCell in the minimal dialect > still guards formula injection: "\rcmd""
#[test]
fn escapecsvcell_in_the_minimal_dialect_still_guards_formula_injection_rcmd() {
    assert_eq!(minimal(cell("\rcmd")), "\"'\rcmd\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:91 "escapeCsvCell in the minimal dialect > still guards formula injection: "=\"hello\"""
#[test]
fn escapecsvcell_in_the_minimal_dialect_still_guards_formula_injection_hello() {
    assert_eq!(minimal(cell("=\"hello\"")), "\"'=\"\"hello\"\"\"");
}

/// Port of: packages/utils/__tests__/csv.test.ts:103 "toCsvRow > joins escaped cells with commas and ends with a newline"
#[test]
fn tocsvrow_joins_escaped_cells_with_commas_and_ends_with_a_newline() {
    assert_eq!(
        to_csv_row(
            &[cell("Jane"), CsvCell::Empty, CsvCell::from(3), cell("=x")],
            CsvDialect::Always
        ),
        "\"Jane\",\"\",\"3\",\"'=x\"\n"
    );
}

/// Port of: packages/utils/__tests__/csv.test.ts:107 "toCsvRow > applies the minimal dialect to every cell when asked"
#[test]
fn tocsvrow_applies_the_minimal_dialect_to_every_cell_when_asked() {
    assert_eq!(
        to_csv_row(
            &[cell("Jane"), CsvCell::Empty, cell("a,b")],
            CsvDialect::Minimal
        ),
        "Jane,,\"a,b\"\n"
    );
}

/// Port of: packages/utils/__tests__/csv.test.ts:111 "toCsvRow > renders an empty row as a bare newline"
#[test]
fn tocsvrow_renders_an_empty_row_as_a_bare_newline() {
    assert_eq!(to_csv_row(&[], CsvDialect::Always), "\n");
}
