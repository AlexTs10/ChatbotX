//! Port of `packages/database/__tests__/fb-comment-reply-texts.test.ts` (P3.2b, ported;
//! `toBe(flow)` (same object) is equality of the returned value).

use chatbotx_db::repo::partials::comment_automation::{
    CommentReply, CommentReplyText, CommentReplyType, normalize_reply_texts, resolve_reply_texts,
};

fn text(value: Option<&str>, values: Option<&[&str]>) -> CommentReply {
    CommentReply {
        type_: CommentReplyType::Text,
        value: value.map(str::to_owned),
        values: values.map(|list| {
            list.iter()
                .map(|v| CommentReplyText {
                    value: (*v).to_owned(),
                })
                .collect()
        }),
    }
}

fn texts(values: &[&str]) -> Vec<CommentReplyText> {
    values
        .iter()
        .map(|v| CommentReplyText {
            value: (*v).to_owned(),
        })
        .collect()
}

#[test]
fn resolve_reply_texts_prefers_the_message_list() {
    assert_eq!(
        resolve_reply_texts(&text(Some("stale"), Some(&["a", "b"]))),
        vec!["a", "b"]
    );
}

#[test]
fn resolve_reply_texts_falls_back_to_the_legacy_single_value_every_row_predating_the_list_has_only_this()
 {
    assert_eq!(
        resolve_reply_texts(&text(Some("legacy"), None)),
        vec!["legacy"]
    );
}

#[test]
fn resolve_reply_texts_trims_and_drops_blanks_so_an_empty_editor_never_posts_an_empty_comment() {
    assert_eq!(
        resolve_reply_texts(&text(Some(""), Some(&["  kept  ", "   ", ""]))),
        vec!["kept"]
    );
}

#[test]
fn resolve_reply_texts_returns_nothing_for_an_unset_reply() {
    assert!(resolve_reply_texts(&text(None, None)).is_empty());
    assert!(resolve_reply_texts(&text(Some(""), Some(&[]))).is_empty());
}

#[test]
fn normalize_reply_texts_mirrors_the_first_message_onto_value_so_anything_still_reading_it_sees_real_content()
 {
    let result = normalize_reply_texts(text(Some("stale"), Some(&["a", "b"])));
    assert_eq!(result.value.as_deref(), Some("a"));
    assert_eq!(result.values, Some(texts(&["a", "b"])));
}

#[test]
fn normalize_reply_texts_builds_the_list_from_a_legacy_payload_so_a_client_sending_only_value_is_not_ignored()
 {
    assert_eq!(
        normalize_reply_texts(text(Some("only"), None)),
        text(Some("only"), Some(&["only"]))
    );
}

#[test]
fn normalize_reply_texts_leaves_non_text_replies_alone_value_there_is_a_flow_or_agent_id() {
    let flow = CommentReply {
        type_: CommentReplyType::Flow,
        value: Some("flow-1".to_owned()),
        values: None,
    };
    assert_eq!(normalize_reply_texts(flow.clone()), flow);
}
