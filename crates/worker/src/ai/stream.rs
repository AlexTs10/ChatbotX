//! `processStreamingText` + `processTextForImagesAndLinks`
//! (`packages/ai/src/core/stream.ts`): a model answer as the chat parts it
//! is sent in. Each paragraph is one segment; inside a segment each image /
//! link / URL is its own part, the text around it is cleaned of markdown.
//!
//! ponytail: the text is split after generation (the original splits while
//! streaming); the parts and their order are the same.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

static ANY_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"!\[[^\]]*\]\(\s*([^)\s\r\n]+)\s*\)|\[[^\]]+\]\(\s*([^)\s\r\n]+)\s*\)|(https?://[^\s)\]]+(?:\?[^\s)\]]*)?)",
    )
    .expect("token regex")
});
static ONLY_EMOJI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\x{1F300}-\x{1F9FF}]+$").expect("emoji regex"));
static NOISY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[-*.\s]+$").expect("noise regex"));
// ponytail: the original's `(\*\*|__)…\1` back-reference is two passes here.
static BOLD: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r"\*\*([^*_]+?)\*\*").expect("bold regex"),
        Regex::new(r"__([^*_]+?)__").expect("bold regex"),
    ]
});
static ITALIC: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r"\*([^*_]+?)\*").expect("italic regex"),
        Regex::new(r"_([^*_]+?)_").expect("italic regex"),
    ]
});
static LEADING_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[-*]\s*").expect("marker regex"));
static TRAILING_LIST_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n\s*[-*]\s*$").expect("marker regex"));

const PARAGRAPH_SEPARATOR: &str = "\n\n";

fn clean_text(value: &str) -> String {
    let mut text = value.to_owned();
    for re in BOLD.iter().chain(ITALIC.iter()) {
        text = re.replace_all(&text, "$1").into_owned();
    }
    let text = LEADING_MARKER.replace(&text, "").trim().to_owned();
    TRAILING_LIST_MARKER.replace(&text, "").trim().to_owned()
}

fn is_meaningful(part: &str) -> bool {
    !(part.trim().is_empty() || ONLY_EMOJI.is_match(part) || NOISY.is_match(part))
}

/// `processTextForImagesAndLinks`.
pub fn text_parts(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = 0;
    for caps in ANY_TOKEN.captures_iter(text) {
        let token = caps.get(0).expect("whole match");
        if token.start() > cursor {
            let before = clean_text(&text[cursor..token.start()]);
            if !before.is_empty() {
                parts.push(before);
            }
        }
        let url = (1..=3)
            .find_map(|i| caps.get(i))
            .map(|m| m.as_str().trim().to_owned())
            .unwrap_or_default();
        if !url.is_empty() && seen.insert(url.clone()) {
            parts.push(url);
        }
        cursor = token.end();
    }
    if cursor < text.len() {
        let tail = clean_text(&text[cursor..]);
        if !tail.is_empty() {
            parts.push(tail);
        }
    }
    parts
        .into_iter()
        .map(|p| p.trim().to_owned())
        .filter(|p| is_meaningful(p))
        .collect()
}

/// `processStreamingText` with `sendParts`: the parts of every paragraph.
pub fn message_parts(full_text: &str) -> Vec<String> {
    full_text
        .split(PARAGRAPH_SEPARATOR)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .flat_map(text_parts)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_paragraphs_links_and_markdown() {
        let text = "**Hello** there!\n\nSee ![shoe](https://x.io/a.png) and https://x.io/p?id=1\n\n- 🎉\n\n---";
        assert_eq!(
            message_parts(text),
            vec![
                "Hello there!",
                "See",
                "https://x.io/a.png",
                "and",
                "https://x.io/p?id=1",
            ]
        );
        assert_eq!(
            text_parts("[a](https://x.io) [b](https://x.io)"),
            vec!["https://x.io"]
        );
        assert_eq!(text_parts("- _note_ item\n-"), vec!["note item"]);
    }
}
