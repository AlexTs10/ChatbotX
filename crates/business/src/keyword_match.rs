//! `packages/automated-response/src/keyword-match.ts`: does a keyword rule
//! match a message.

use std::sync::LazyLock;

use chatbotx_core::utils::has_link::has_link;
use chatbotx_core::utils::levenshtein::levenshtein_distance;
use chatbotx_core::utils::normalize_for_match::normalize_for_match;
use chatbotx_db::repo::models::AutomatedResponse;
use chatbotx_db::repo::partials::automated_response::{KeywordCondition, KeywordConditionOperator};
use chatbotx_db::repo::partials::keyword_condition_schema::compile_keyword_regex;
use regex::Regex;

use crate::contact::extract::{ExtractOptions, extract_contact_info};

/// `KeywordMatchMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    Contains,
    Exact,
}

/// `keywordMatchModeByAutomatedResponseType`.
pub fn match_mode_of(kind: &str) -> MatchMode {
    if kind == "outbound" {
        MatchMode::Exact
    } else {
        MatchMode::Contains
    }
}

/// `keywordMatchesText` (`lower_case_text` is already lower-cased).
pub fn keyword_matches_text(keywords: &[String], lower_case_text: &str, mode: MatchMode) -> bool {
    let trimmed = lower_case_text.trim();
    keywords
        .iter()
        .map(|k| k.trim().to_lowercase())
        .filter(|k| !k.is_empty())
        .any(|k| match mode {
            MatchMode::Exact => trimmed == k,
            MatchMode::Contains => lower_case_text.contains(&k),
        })
}

/// `KeywordMatchInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MatchInput {
    pub text: Option<String>,
    pub attachment_file_types: Vec<String>,
}

impl MatchInput {
    pub fn text(text: &str) -> MatchInput {
        MatchInput {
            text: Some(text.to_owned()),
            attachment_file_types: Vec::new(),
        }
    }
}

/// The matched fields of an `AutomatedResponse` row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeywordRule {
    pub keywords: Vec<String>,
    pub conditions: Option<Vec<KeywordCondition>>,
    pub kind: String,
}

impl KeywordRule {
    pub fn of(row: &AutomatedResponse) -> KeywordRule {
        KeywordRule {
            keywords: serde_json::from_value(row.keywords.0.clone()).unwrap_or_default(),
            conditions: row
                .conditions
                .as_ref()
                .and_then(|c| serde_json::from_value(c.0.clone()).ok()),
            kind: row.r#type.clone(),
        }
    }
}

/// `MAX_SIMILARITY_TEXT_LENGTH`.
const MAX_SIMILARITY_TEXT_LENGTH: usize = 500;
/// `SIMILARITY_EDITS_DIVISOR`.
const SIMILARITY_EDITS_DIVISOR: usize = 5;

static WORD_SEPARATOR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\p{L}\p{N}]+").unwrap());
static WHITESPACE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static THUMBS_UP_NOISE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s|\x{FE0F}|[\x{1F3FB}-\x{1F3FF}]").unwrap());
static THUMBS_UP_ONLY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:\x{1F44D})+$").unwrap());

fn normalize_message_text(value: &str) -> String {
    WHITESPACE_RE
        .replace_all(&normalize_for_match(value), " ")
        .trim()
        .to_owned()
}

fn split_words(value: &str) -> Vec<&str> {
    WORD_SEPARATOR_RE
        .split(value)
        .filter(|w| !w.is_empty())
        .collect()
}

fn contains_whole_word(text: &str, keyword: &str) -> bool {
    let keyword_words = split_words(keyword);
    if keyword_words.is_empty() {
        return text.contains(keyword);
    }
    let text_words = split_words(text);
    text_words
        .windows(keyword_words.len())
        .any(|window| window == keyword_words.as_slice())
}

/// UTF-16 length (`String.length`).
fn js_len(value: &str) -> usize {
    value.encode_utf16().count()
}

fn is_similar_to(text: &str, keyword: &str) -> bool {
    if text == keyword {
        return true;
    }
    let (text_len, keyword_len) = (js_len(text), js_len(keyword));
    if text_len == 0
        || keyword_len == 0
        || text_len > MAX_SIMILARITY_TEXT_LENGTH
        || keyword_len > MAX_SIMILARITY_TEXT_LENGTH
    {
        return false;
    }
    let max_distance = text_len.max(keyword_len) / SIMILARITY_EDITS_DIVISOR;
    if text_len.abs_diff(keyword_len) > max_distance {
        return false;
    }
    levenshtein_distance(text, keyword) <= max_distance
}

fn is_thumbs_up(text: &str) -> bool {
    THUMBS_UP_ONLY_RE.is_match(&THUMBS_UP_NOISE_RE.replace_all(text, ""))
}

fn matches_regex(text: &str, pattern: &str) -> bool {
    // ponytail: compiled per message (the original keeps a 500-entry cache);
    // add one when regex rules show up in profiles.
    compile_keyword_regex(pattern).is_some_and(|re| re.is_match(text))
}

const IMAGE_FILE_TYPES: [&str; 2] = ["image", "gif"];
const VIDEO_FILE_TYPES: [&str; 1] = ["video"];

/// `isKeywordMediaFileType`.
pub fn is_keyword_media_file_type(file_type: &str) -> bool {
    IMAGE_FILE_TYPES.contains(&file_type) || VIDEO_FILE_TYPES.contains(&file_type)
}

fn has_attachment(input: &MatchInput, file_types: &[&str]) -> bool {
    input
        .attachment_file_types
        .iter()
        .any(|t| file_types.contains(&t.as_str()))
}

fn matches_keyword_value(operator: KeywordConditionOperator, text: &str, keyword: &str) -> bool {
    use KeywordConditionOperator::*;
    match operator {
        Is => text == keyword,
        SimilarTo => is_similar_to(text, keyword),
        ContainsWholeWord => contains_whole_word(text, keyword),
        BeginsWith => text.starts_with(keyword),
        EndsWith => text.ends_with(keyword),
        _ => text.contains(keyword),
    }
}

/// `keywordConditionMatches`.
pub fn keyword_condition_matches(condition: &KeywordCondition, input: &MatchInput) -> bool {
    use KeywordConditionOperator::*;
    let raw = input.text.as_deref().unwrap_or("");
    match condition.operator {
        ContainsPhone => {
            let options = ExtractOptions {
                skip_email: true,
                ..ExtractOptions::default()
            };
            return extract_contact_info(Some(raw), None, options)
                .phone_number
                .is_some();
        }
        ContainsEmail => {
            let options = ExtractOptions {
                skip_phone: true,
                ..ExtractOptions::default()
            };
            return extract_contact_info(Some(raw), None, options)
                .email
                .is_some();
        }
        ContainsLink => return has_link(raw),
        ContainsImage => return has_attachment(input, &IMAGE_FILE_TYPES),
        ContainsVideo => return has_attachment(input, &VIDEO_FILE_TYPES),
        IsThumbsUp => return is_thumbs_up(raw),
        MatchesRegex => return condition.values.iter().any(|p| matches_regex(raw, p)),
        _ => {}
    }
    let text = normalize_message_text(raw);
    let keywords: Vec<String> = condition
        .values
        .iter()
        .map(|v| normalize_message_text(v))
        .filter(|k| !k.is_empty())
        .collect();
    if condition.operator == NotContains {
        return !keywords.iter().any(|k| text.contains(k.as_str()));
    }
    if text.is_empty() {
        return false;
    }
    keywords
        .iter()
        .any(|k| matches_keyword_value(condition.operator, &text, k))
}

/// `ruleMatchesMessage`.
pub fn rule_matches_message(rule: &KeywordRule, input: &MatchInput) -> bool {
    if let Some(conditions) = rule.conditions.as_ref().filter(|c| !c.is_empty()) {
        return conditions
            .iter()
            .all(|c| keyword_condition_matches(c, input));
    }
    let Some(text) = input.text.as_deref().filter(|t| !t.is_empty()) else {
        return false;
    };
    keyword_matches_text(
        &rule.keywords,
        &text.to_lowercase(),
        match_mode_of(&rule.kind),
    )
}

/// `matchesAnyKeywordRule`.
pub fn matches_any_keyword_rule(input: &MatchInput, rules: &[KeywordRule]) -> bool {
    rules.iter().any(|r| rule_matches_message(r, input))
}

#[cfg(test)]
mod tests {
    //! `packages/automated-response/__tests__/keyword-match.test.ts`.
    use super::*;

    fn kw(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    fn cond(operator: KeywordConditionOperator, values: &[&str]) -> KeywordCondition {
        KeywordCondition {
            operator,
            values: kw(values),
        }
    }

    fn matches(condition: &KeywordCondition, text: &str) -> bool {
        keyword_condition_matches(condition, &MatchInput::text(text))
    }

    fn inbound(conditions: Option<Vec<KeywordCondition>>) -> KeywordRule {
        KeywordRule {
            keywords: Vec::new(),
            conditions,
            kind: "inbound".into(),
        }
    }

    use KeywordConditionOperator::*;

    #[test]
    fn keyword_matches_text_cases() {
        assert!(keyword_matches_text(
            &kw(&["PRICE"]),
            "what is the price?",
            MatchMode::Contains
        ));
        assert!(keyword_matches_text(
            &kw(&["test"]),
            "htest1",
            MatchMode::Contains
        ));
        assert!(!keyword_matches_text(&[], "hello", MatchMode::Contains));
        assert!(keyword_matches_text(
            &kw(&["price"]),
            "price",
            MatchMode::Exact
        ));
        assert!(!keyword_matches_text(
            &kw(&["price"]),
            "what is the price?",
            MatchMode::Exact
        ));
        assert!(keyword_matches_text(
            &kw(&["PRICE"]),
            "  price  ",
            MatchMode::Exact
        ));
        assert!(!keyword_matches_text(
            &kw(&[" "]),
            "hello world",
            MatchMode::Contains
        ));
        assert!(!keyword_matches_text(&kw(&[" "]), "", MatchMode::Exact));
        assert!(!keyword_matches_text(&kw(&[" "]), "   ", MatchMode::Exact));
        assert!(keyword_matches_text(
            &kw(&[" price "]),
            "the price is high",
            MatchMode::Contains
        ));
    }

    #[test]
    fn maps_contact_rules_to_contains_and_page_rules_to_exact() {
        assert_eq!(match_mode_of("inbound"), MatchMode::Contains);
        assert_eq!(match_mode_of("outbound"), MatchMode::Exact);
    }

    #[test]
    fn matches_any_keyword_rule_cases() {
        let rule = |k: &[&str]| KeywordRule {
            keywords: kw(k),
            conditions: None,
            kind: "inbound".into(),
        };
        assert!(matches_any_keyword_rule(
            &MatchInput::text("I need support"),
            &[rule(&["pricing"]), rule(&["support"])]
        ));
        assert!(!matches_any_keyword_rule(&MatchInput::text("hello"), &[]));
        assert!(!matches_any_keyword_rule(
            &MatchInput::text("hello"),
            &[rule(&[])]
        ));
    }

    #[test]
    fn rules_without_conditions_keep_the_legacy_contains_exact_modes() {
        let legacy = |kind: &str| KeywordRule {
            keywords: kw(&["test"]),
            conditions: None,
            kind: kind.into(),
        };
        assert!(rule_matches_message(
            &legacy("inbound"),
            &MatchInput::text("htest1")
        ));
        assert!(!rule_matches_message(
            &legacy("outbound"),
            &MatchInput::text("htest1")
        ));
        assert!(rule_matches_message(
            &legacy("outbound"),
            &MatchInput::text(" TEST ")
        ));
    }

    #[test]
    fn ands_conditions_and_ors_the_values_inside_one_condition() {
        let rule = inbound(Some(vec![
            cond(Contains, &["muốn", "quan tâm"]),
            cond(ContainsWholeWord, &["ok"]),
            cond(NotContains, &["test"]),
        ]));
        assert!(rule_matches_message(
            &rule,
            &MatchInput::text("Mình quan tâm, ok bạn")
        ));
        assert!(rule_matches_message(
            &rule,
            &MatchInput::text("muon mua ok")
        ));
        assert!(!rule_matches_message(
            &rule,
            &MatchInput::text("muốn mua book")
        ));
        assert!(!rule_matches_message(
            &rule,
            &MatchInput::text("muốn ok test")
        ));
    }

    #[test]
    fn an_attachment_only_message_only_matches_attachment_conditions() {
        let image_only = MatchInput {
            text: None,
            attachment_file_types: kw(&["image"]),
        };
        assert!(rule_matches_message(
            &inbound(Some(vec![cond(ContainsImage, &[])])),
            &image_only
        ));
        assert!(!rule_matches_message(
            &inbound(Some(vec![cond(Contains, &["hi"])])),
            &image_only
        ));
    }

    #[test]
    fn ignores_case_and_vietnamese_accents() {
        assert!(matches(&cond(Contains, &["Tư Vấn"]), "cần tu van"));
        assert!(matches(&cond(Is, &["đặt hàng"]), "  DAT   hang "));
        assert!(!matches(&cond(Is, &["đặt hàng"]), "đặt hàng nhé"));
    }

    #[test]
    fn similar_to_tolerates_small_typos_but_not_different_messages() {
        let c = cond(SimilarTo, &["tư vấn giúp mình"]);
        assert!(matches(&c, "tu vaan giup minh"));
        assert!(!matches(&c, "giá bao nhiêu"));
    }

    #[test]
    fn similar_to_allows_one_edit_per_five_characters_no_more() {
        let c = cond(SimilarTo, &["abcdefghij"]);
        assert!(matches(&c, "abcdefghxy"));
        assert!(!matches(&c, "abcdefgxyz"));
        assert!(!matches(&c, "abcdefghij and much more"));
    }

    #[test]
    fn matches_regex_reuses_a_compiled_pattern_across_messages() {
        let c = cond(MatchesRegex, &["^m(a|ã) ?\\d+$"]);
        assert!(matches(&c, "MA 12"));
        assert!(matches(&c, "mã12"));
        assert!(!matches(&c, "ma x"));
    }

    #[test]
    fn contains_whole_word_respects_word_boundaries() {
        let c = cond(ContainsWholeWord, &["ok"]);
        assert!(matches(&c, "ok bạn"));
        assert!(matches(&c, "vậy, OK!"));
        assert!(!matches(&c, "book"));
        assert!(matches(
            &cond(ContainsWholeWord, &["giá sỉ"]),
            "cho xin gia si nhé"
        ));
    }

    #[test]
    fn begins_with_and_ends_with() {
        assert!(matches(&cond(BeginsWith, &["chào"]), "Chao shop"));
        assert!(!matches(&cond(BeginsWith, &["chào"]), "shop chào"));
        assert!(matches(&cond(EndsWith, &["nhé"]), "gửi giúp nhe"));
    }

    #[test]
    fn not_contains_requires_none_of_the_values() {
        let c = cond(NotContains, &["test", "spam"]);
        assert!(matches(&c, "hello"));
        assert!(!matches(&c, "this is spam"));
    }

    #[test]
    fn detects_phone_numbers_emails_and_links() {
        assert!(matches(
            &cond(ContainsPhone, &[]),
            "sđt của mình 0912 345 678"
        ));
        assert!(!matches(&cond(ContainsPhone, &[]), "mình muốn mua 2 cái"));
        assert!(matches(
            &cond(ContainsEmail, &[]),
            "mail: An.Nguyen@example.com"
        ));
        assert!(!matches(&cond(ContainsEmail, &[]), "không có mail"));
        assert!(matches(&cond(ContainsLink, &[]), "xem https://shop.vn/a"));
        assert!(matches(&cond(ContainsLink, &[]), "vào example.com nhé"));
        assert!(!matches(&cond(ContainsLink, &[]), "không có link"));
    }

    #[test]
    fn detects_image_and_video_attachments() {
        let with = |types: &[&str]| MatchInput {
            text: Some("xem nè".into()),
            attachment_file_types: kw(types),
        };
        let image = cond(ContainsImage, &[]);
        let video = cond(ContainsVideo, &[]);
        assert!(keyword_condition_matches(&image, &with(&["image"])));
        assert!(keyword_condition_matches(&image, &with(&["gif"])));
        assert!(!keyword_condition_matches(&image, &with(&["video"])));
        assert!(keyword_condition_matches(&video, &with(&["video"])));
        assert!(!keyword_condition_matches(&video, &with(&[])));
    }

    #[test]
    fn is_thumbs_up_accepts_only_thumbs_up_emoji_in_any_skin_tone() {
        let c = cond(IsThumbsUp, &[]);
        assert!(matches(&c, "👍"));
        assert!(matches(&c, " 👍🏽👍 "));
        assert!(!matches(&c, "👍 ok"));
        assert!(!matches(&c, ""));
    }

    #[test]
    fn matches_regex_is_case_insensitive_and_safe_from_catastrophic_backtracking() {
        assert!(matches(&cond(MatchesRegex, &["^order-\\d+$"]), "ORDER-123"));
        let started = std::time::Instant::now();
        assert!(!matches(
            &cond(MatchesRegex, &["(a+)+$"]),
            &format!("{}b", "a".repeat(5000))
        ));
        assert!(matches(&cond(MatchesRegex, &["^\\d+$"]), &"1".repeat(3000)));
        assert!(!matches(
            &cond(MatchesRegex, &["^\\d+$"]),
            &format!("{}x", "1".repeat(3000))
        ));
        assert!(started.elapsed().as_millis() < 1000);
        assert!(!matches(&cond(MatchesRegex, &["(unclosed"]), "x"));
    }
}
