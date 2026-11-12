//! `packages/utils/src/sanitize-file-name.ts`.

use std::sync::OnceLock;

use regex::Regex;

use super::normalize_for_match::{nfd, strip_combining_marks};

const MAX_FILE_NAME_LENGTH: usize = 80;
const FALLBACK_BASE_NAME: &str = "file";

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}

fn trim_edge_punctuation(text: &str) -> String {
    static EDGE: OnceLock<Regex> = OnceLock::new();
    re(&EDGE, r"^[-._]+|[-._]+$")
        .replace_all(text, "")
        .into_owned()
}

fn to_slug(text: &str, max_length: usize) -> String {
    static UNSAFE: OnceLock<Regex> = OnceLock::new();
    static DASH_RUNS: OnceLock<Regex> = OnceLock::new();
    let replaced = re(&UNSAFE, "[^a-z0-9._-]+").replace_all(text, "-");
    let collapsed = re(&DASH_RUNS, "-{2,}").replace_all(&replaced, "-");
    let slug = trim_edge_punctuation(&collapsed);
    // ASCII only at this point, so a byte cut is a UTF-16 cut.
    let cut = &slug[..slug.len().min(max_length)];
    trim_edge_punctuation(cut)
}

/// `sanitizeFileName`: `"CMND mặt trước.JPG"` → `"cmnd-mat-truoc.jpg"`; the
/// extension is kept; nothing usable left becomes `"file"`.
pub fn sanitize_file_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let ascii = strip_combining_marks(&nfd(&lower)).replace('đ', "d");
    let ascii = crate::js::trim(&ascii).to_owned();
    static EXTENSION: OnceLock<Regex> = OnceLock::new();
    let extension = re(&EXTENSION, r"\.[a-z0-9]{1,10}$")
        .find(&ascii)
        .map(|m| m.as_str().to_owned())
        .unwrap_or_default();
    let stem = &ascii[..ascii.len() - extension.len()];
    let base = to_slug(stem, MAX_FILE_NAME_LENGTH - extension.len());
    let base = if base.is_empty() {
        FALLBACK_BASE_NAME.to_owned()
    } else {
        base
    };
    format!("{base}{extension}")
}
