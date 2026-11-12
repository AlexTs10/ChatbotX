//! Port of `packages/utils/__tests__/spintax.test.ts`. The `{ pick }`
//! option is the `pick` closure of `apply_spintax_with`.

use std::cell::Cell;

use chatbotx_core::utils::spintax::{
    SPINTAX_MAX_BLOCK_LENGTH, SPINTAX_MAX_BRANCHES, apply_spintax_with, contains_spintax,
};

fn first(text: &str) -> String {
    apply_spintax_with(text, &mut |_| 0)
}

fn second(text: &str) -> String {
    apply_spintax_with(text, &mut |_| 1)
}

/// Port of: packages/utils/__tests__/spintax.test.ts:13 "containsSpintax > detects a block in "{Hi|Hello}""
#[test]
fn containsspintax_detects_a_block_in_hi_hello() {
    assert!(contains_spintax("{Hi|Hello}"));
}

/// Port of: packages/utils/__tests__/spintax.test.ts:13 "containsSpintax > detects a block in "{Hi|Hello|Hey} {{first_name}}""
#[test]
fn containsspintax_detects_a_block_in_hi_hello_hey_first_name() {
    assert!(contains_spintax("{Hi|Hello|Hey} {{first_name}}"));
}

/// Port of: packages/utils/__tests__/spintax.test.ts:13 "containsSpintax > detects a block in "trailing {a|b} text""
#[test]
fn containsspintax_detects_a_block_in_trailing_a_b_text() {
    assert!(contains_spintax("trailing {a|b} text"));
}

/// Port of: packages/utils/__tests__/spintax.test.ts:13 "containsSpintax > detects a block in "{Hi|}""
#[test]
fn containsspintax_detects_a_block_in_hi() {
    assert!(contains_spintax("{Hi|}"));
}

/// Port of: packages/utils/__tests__/spintax.test.ts:24 "containsSpintax > finds no block in "{{first_name}}""
#[test]
fn containsspintax_finds_no_block_in_first_name() {
    assert!(!contains_spintax("{{first_name}}"));
}

/// Port of: packages/utils/__tests__/spintax.test.ts:24 "containsSpintax > finds no block in "{{coupon:SUMMER}}""
#[test]
fn containsspintax_finds_no_block_in_coupon_summer() {
    assert!(!contains_spintax("{{coupon:SUMMER}}"));
}

/// Port of: packages/utils/__tests__/spintax.test.ts:24 "containsSpintax > finds no block in "{{bot_field:12}}""
#[test]
fn containsspintax_finds_no_block_in_bot_field_12() {
    assert!(!contains_spintax("{{bot_field:12}}"));
}

/// Port of: packages/utils/__tests__/spintax.test.ts:24 "containsSpintax > finds no block in "{{raw:field}}""
#[test]
fn containsspintax_finds_no_block_in_raw_field() {
    assert!(!contains_spintax("{{raw:field}}"));
}

/// Port of: packages/utils/__tests__/spintax.test.ts:24 "containsSpintax > finds no block in "plain text | with a pipe""
#[test]
fn containsspintax_finds_no_block_in_plain_text_with_a_pipe() {
    assert!(!contains_spintax("plain text | with a pipe"));
}

/// Port of: packages/utils/__tests__/spintax.test.ts:24 "containsSpintax > finds no block in "{no pipe here}""
#[test]
fn containsspintax_finds_no_block_in_no_pipe_here() {
    assert!(!contains_spintax("{no pipe here}"));
}

/// Port of: packages/utils/__tests__/spintax.test.ts:24 "containsSpintax > finds no block in "{a\nb|c}""
#[test]
fn containsspintax_finds_no_block_in_a_nb_c() {
    assert!(!contains_spintax("{a\nb|c}"));
}

/// Port of: packages/utils/__tests__/spintax.test.ts:24 "containsSpintax > finds no block in """
#[test]
fn containsspintax_finds_no_block_in() {
    assert!(!contains_spintax(""));
}

/// Port of: packages/utils/__tests__/spintax.test.ts:39 "applySpintax > picks a branch"
#[test]
fn applyspintax_picks_a_branch() {
    assert_eq!(first("{Hi|Hello|Hey} bạn"), "Hi bạn");
    assert_eq!(second("{Hi|Hello|Hey} bạn"), "Hello bạn");
}

/// Port of: packages/utils/__tests__/spintax.test.ts:44 "applySpintax > trims each branch"
#[test]
fn applyspintax_trims_each_branch() {
    assert_eq!(first("{ Chào | Hi } bạn"), "Chào bạn");
    assert_eq!(second("{ Chào | Hi } bạn"), "Hi bạn");
}

/// Port of: packages/utils/__tests__/spintax.test.ts:49 "applySpintax > an empty branch renders as nothing"
#[test]
fn applyspintax_an_empty_branch_renders_as_nothing() {
    assert_eq!(second("Chào{ bạn|}"), "Chào");
}

/// Port of: packages/utils/__tests__/spintax.test.ts:55 "applySpintax > draws each occurrence independently"
#[test]
fn applyspintax_draws_each_occurrence_independently() {
    let picks = [0usize, 1];
    let call = Cell::new(0usize);
    let result = apply_spintax_with("{Hi|Hello} … {Hi|Hello}", &mut |_| {
        let index = call.get();
        call.set(index + 1);
        picks.get(index).copied().unwrap_or(0)
    });

    assert_eq!(result, "Hi … Hello");
    assert_eq!(call.get(), 2);
}

/// Port of: packages/utils/__tests__/spintax.test.ts:66 "applySpintax > leaves text without a block untouched"
#[test]
fn applyspintax_leaves_text_without_a_block_untouched() {
    assert_eq!(first("Chào bạn"), "Chào bạn");
}

/// Port of: packages/utils/__tests__/spintax.test.ts:72 "applySpintax > never touches the variable placeholder in "{{first_name}}""
#[test]
fn applyspintax_never_touches_the_variable_placeholder_in_first_name() {
    assert_eq!(first("{{first_name}}"), "{{first_name}}");
}

/// Port of: packages/utils/__tests__/spintax.test.ts:72 "applySpintax > never touches the variable placeholder in "Chào {{first_name}}, mã {{coupon:SUMMER}}""
#[test]
fn applyspintax_never_touches_the_variable_placeholder_in_ch_o_first_name_m_coupon_summer() {
    assert_eq!(
        first("Chào {{first_name}}, mã {{coupon:SUMMER}}"),
        "Chào {{first_name}}, mã {{coupon:SUMMER}}"
    );
}

/// Port of: packages/utils/__tests__/spintax.test.ts:72 "applySpintax > never touches the variable placeholder in "{{raw:note}} {{bot_field:12}}""
#[test]
fn applyspintax_never_touches_the_variable_placeholder_in_raw_note_bot_field_12() {
    assert_eq!(
        first("{{raw:note}} {{bot_field:12}}"),
        "{{raw:note}} {{bot_field:12}}"
    );
}

/// Port of: packages/utils/__tests__/spintax.test.ts:72 "applySpintax > never touches the variable placeholder in "{{ full_name }}""
#[test]
fn applyspintax_never_touches_the_variable_placeholder_in_full_name() {
    assert_eq!(first("{{ full_name }}"), "{{ full_name }}");
}

/// Port of: packages/utils/__tests__/spintax.test.ts:81 "applySpintax > mixes with variables in one string"
#[test]
fn applyspintax_mixes_with_variables_in_one_string() {
    assert_eq!(first("{Chào|Hi} {{first_name}}!"), "Chào {{first_name}}!");
}

/// Port of: packages/utils/__tests__/spintax.test.ts:87 "applySpintax > does not span a newline"
#[test]
fn applyspintax_does_not_span_a_newline() {
    assert_eq!(first("{Hi\n|Hello}"), "{Hi\n|Hello}");
}

/// Port of: packages/utils/__tests__/spintax.test.ts:94 "applySpintax > a JSON-like fragment does match — hence opt-in"
#[test]
fn applyspintax_a_json_like_fragment_does_match_hence_opt_in() {
    assert_eq!(first(r#"{"a": "x|y"}"#), r#""a": "x"#);
}

/// Port of: packages/utils/__tests__/spintax.test.ts:98 "applySpintax > leaves an oversized block verbatim"
#[test]
fn applyspintax_leaves_an_oversized_block_verbatim() {
    let content = format!("{}|b", "a".repeat(SPINTAX_MAX_BLOCK_LENGTH));
    let value = format!("{{{content}}}");

    assert_eq!(first(&value), value);
}

/// Port of: packages/utils/__tests__/spintax.test.ts:105 "applySpintax > leaves a block with too many branches verbatim"
#[test]
fn applyspintax_leaves_a_block_with_too_many_branches_verbatim() {
    let branches: Vec<String> = (0..=SPINTAX_MAX_BRANCHES).map(|i| i.to_string()).collect();
    let value = format!("{{{}}}", branches.join("|"));

    assert_eq!(first(&value), value);
}

/// Port of: packages/utils/__tests__/spintax.test.ts:111 "applySpintax > an out-of-range pick leaves the block verbatim"
#[test]
fn applyspintax_an_out_of_range_pick_leaves_the_block_verbatim() {
    assert_eq!(apply_spintax_with("{Hi|Hello}", &mut |_| 9), "{Hi|Hello}");
}
