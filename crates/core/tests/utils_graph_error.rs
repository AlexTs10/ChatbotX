//! Port of `packages/utils/__tests__/graph-error.test.ts`.

use chatbotx_core::utils::graph_error::{GraphErrorSource, format_graph_error_message};
use serde_json::json;

fn s(value: &str) -> Option<String> {
    Some(value.to_owned())
}

/// Port of: packages/utils/__tests__/graph-error.test.ts:5 "formatGraphErrorMessage > keeps the code, the subcode, and both sentences"
#[test]
fn formatgrapherrormessage_keeps_the_code_the_subcode_and_both_sentences() {
    assert_eq!(
        format_graph_error_message(&GraphErrorSource {
            code: Some(json!(10)),
            sub_code: Some(json!(1_893_063)),
            message: s("Application does not have permission for this action"),
            user_message: s(
                "Bạn tạm thời bị hạn chế gửi tin nhắn. Tìm hiểu thêm về thời gian và lý do chúng tôi hạn chế khả năng nhắn tin."
            ),
        }),
        s(
            "(#10 - 1893063) Application does not have permission for this action. Bạn tạm thời bị hạn chế gửi tin nhắn. Tìm hiểu thêm về thời gian và lý do chúng tôi hạn chế khả năng nhắn tin."
        )
    );
}

/// Port of: packages/utils/__tests__/graph-error.test.ts:20 "formatGraphErrorMessage > prints the code alone when Meta sent no subcode"
#[test]
fn formatgrapherrormessage_prints_the_code_alone_when_meta_sent_no_subcode() {
    assert_eq!(
        format_graph_error_message(&GraphErrorSource {
            code: Some(json!(190)),
            message: s("Access token expired"),
            ..Default::default()
        }),
        s("(#190) Access token expired")
    );
}

/// Port of: packages/utils/__tests__/graph-error.test.ts:29 "formatGraphErrorMessage > writes no prefix at all when there is no code"
#[test]
fn formatgrapherrormessage_writes_no_prefix_at_all_when_there_is_no_code() {
    assert_eq!(
        format_graph_error_message(&GraphErrorSource {
            message: s("boom"),
            ..Default::default()
        }),
        s("boom")
    );
}

/// Port of: packages/utils/__tests__/graph-error.test.ts:33 "formatGraphErrorMessage > treats the -1 unknown sentinel as no code, never as (#-1)"
#[test]
fn formatgrapherrormessage_treats_the_1_unknown_sentinel_as_no_code_never_as_1() {
    assert_eq!(
        format_graph_error_message(&GraphErrorSource {
            code: Some(json!(-1)),
            sub_code: Some(json!(-1)),
            message: s("boom"),
            ..Default::default()
        }),
        s("boom")
    );
}

/// Port of: packages/utils/__tests__/graph-error.test.ts:39 "formatGraphErrorMessage > ignores our own placeholder codes, which identify nothing"
#[test]
fn formatgrapherrormessage_ignores_our_own_placeholder_codes_which_identify_nothing() {
    assert_eq!(
        format_graph_error_message(&GraphErrorSource {
            code: Some(json!("messengerError")),
            message: s("Messenger API call failed"),
            ..Default::default()
        }),
        s("Messenger API call failed")
    );
}

/// Port of: packages/utils/__tests__/graph-error.test.ts:48 "formatGraphErrorMessage > reads a numeric subcode sent as a string"
#[test]
fn formatgrapherrormessage_reads_a_numeric_subcode_sent_as_a_string() {
    assert_eq!(
        format_graph_error_message(&GraphErrorSource {
            code: Some(json!(100)),
            sub_code: Some(json!("33")),
            message: s("boom"),
            ..Default::default()
        }),
        s("(#100 - 33) boom")
    );
}

/// Port of: packages/utils/__tests__/graph-error.test.ts:54 "formatGraphErrorMessage > prints a repeated sentence only once"
#[test]
fn formatgrapherrormessage_prints_a_repeated_sentence_only_once() {
    assert_eq!(
        format_graph_error_message(&GraphErrorSource {
            message: s("Token expired"),
            user_message: s("Token expired"),
            ..Default::default()
        }),
        s("Token expired")
    );
}

/// Port of: packages/utils/__tests__/graph-error.test.ts:63 "formatGraphErrorMessage > does not add a second period to a message that already ends in one"
#[test]
fn formatgrapherrormessage_does_not_add_a_second_period_to_a_message_that_already_ends_in_one() {
    assert_eq!(
        format_graph_error_message(&GraphErrorSource {
            code: Some(json!(4)),
            message: s("Rate limit reached."),
            user_message: s("Try again later."),
            ..Default::default()
        }),
        s("(#4) Rate limit reached. Try again later.")
    );
}

/// Port of: packages/utils/__tests__/graph-error.test.ts:73 "formatGraphErrorMessage > uses whichever sentence Meta sent when only one is present"
#[test]
fn formatgrapherrormessage_uses_whichever_sentence_meta_sent_when_only_one_is_present() {
    assert_eq!(
        format_graph_error_message(&GraphErrorSource {
            code: Some(json!(10)),
            user_message: s("Bạn bị hạn chế"),
            ..Default::default()
        }),
        s("(#10) Bạn bị hạn chế")
    );
}

/// Port of: packages/utils/__tests__/graph-error.test.ts:79 "formatGraphErrorMessage > does not repeat the code Meta already put in front of its own message"
#[test]
fn formatgrapherrormessage_does_not_repeat_the_code_meta_already_put_in_front_of_its_own_message() {
    assert_eq!(
        format_graph_error_message(&GraphErrorSource {
            code: Some(json!(100)),
            sub_code: Some(json!(2_018_001)),
            message: s("(#100) Param recipient[id] must be a valid ID string"),
            ..Default::default()
        }),
        s("(#100 - 2018001) Param recipient[id] must be a valid ID string")
    );
}

/// Port of: packages/utils/__tests__/graph-error.test.ts:89 "formatGraphErrorMessage > keeps a bracketed code that belongs to some other failure"
#[test]
fn formatgrapherrormessage_keeps_a_bracketed_code_that_belongs_to_some_other_failure() {
    assert_eq!(
        format_graph_error_message(&GraphErrorSource {
            code: Some(json!(100)),
            message: s("(#190) Access token expired"),
            ..Default::default()
        }),
        s("(#100) (#190) Access token expired")
    );
}

/// Port of: packages/utils/__tests__/graph-error.test.ts:98 "formatGraphErrorMessage > falls back to the caller's text when the code prefix was the whole message"
#[test]
fn formatgrapherrormessage_falls_back_to_the_caller_s_text_when_the_code_prefix_was_the_whole_message()
 {
    assert_eq!(
        format_graph_error_message(&GraphErrorSource {
            code: Some(json!(100)),
            message: s("(#100)"),
            ..Default::default()
        }),
        None
    );
}

/// Port of: packages/utils/__tests__/graph-error.test.ts:104 "formatGraphErrorMessage > returns undefined when the body carried no text, so callers can fall back"
#[test]
fn formatgrapherrormessage_returns_undefined_when_the_body_carried_no_text_so_callers_can_fall_back()
 {
    assert_eq!(
        format_graph_error_message(&GraphErrorSource {
            code: Some(json!(10)),
            ..Default::default()
        }),
        None
    );
    assert_eq!(
        format_graph_error_message(&GraphErrorSource {
            message: s("   "),
            ..Default::default()
        }),
        None
    );
    assert_eq!(
        format_graph_error_message(&GraphErrorSource::default()),
        None
    );
}
