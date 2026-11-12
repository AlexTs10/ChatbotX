//! Port of `packages/variables/__tests__/bot-field-variable.test.ts`. `withCache` is the real in-process
//! cache; the `withCache` call assertion reads its key / tag index.

mod support;

use chatbotx_core::variables::bot_field::*;
use chatbotx_core::variables::*;
use support::variables::*;

/// Port of: packages/variables/__tests__/bot-field-variable.test.ts:33 "Bot Field variables > accepts static text and complete numeric Bot Field variables"
#[test]
fn bot_field_variables_accepts_static_text_and_complete_numeric_bot_field_variables() {
    assert_eq!(
        parse_bot_field_variable_text("Bearer static-token"),
        BotFieldVariableTextParseResult::Valid { field_ids: vec![] }
    );
    assert_eq!(
        parse_bot_field_variable_text("key-{{bot_field:42}}"),
        BotFieldVariableTextParseResult::Valid {
            field_ids: vec!["42".into()]
        }
    );
}

/// Port of: packages/variables/__tests__/bot-field-variable.test.ts:44 "Bot Field variables > rejects non-Bot Field and malformed variables"
#[test]
fn bot_field_variables_rejects_non_bot_field_and_malformed_variables() {
    for text in [
        "{{email}}",
        "{{raw:email}}",
        "{{bot_field:abc}}",
        "{{ bot_field:42 }}",
        "{{bot_field:42}",
    ] {
        assert_eq!(
            parse_bot_field_variable_text(text),
            BotFieldVariableTextParseResult::Malformed,
            "{text}"
        );
    }
}

/// Port of: packages/variables/__tests__/bot-field-variable.test.ts:58 "Bot Field variables > resolves only fields from the requested workspace"
#[tokio::test]
async fn bot_field_variables_resolves_only_fields_from_the_requested_workspace() {
    let h = Harness::new(FakeState {
        bot_fields: vec![BotFieldRow {
            id: "42".into(),
            kind: "shortText".into(),
            value: s("workspace-secret"),
        }],
        ..Default::default()
    });
    assert_eq!(
        h.v()
            .resolve_bot_field_variable_text("Bearer {{bot_field:42}}", "workspace-1")
            .await
            .unwrap(),
        BotFieldVariableTextResolution::Resolved {
            value: "Bearer workspace-secret".into()
        }
    );
    assert!(h.fake.called("list_bot_fields(workspace-1)"));
    assert!(h.cache.contains_key("bot-fields:workspace-1:variable-map"));
    assert_eq!(
        h.cache.keys_for_tag("bot-fields"),
        vec!["bot-fields:workspace-1:variable-map".to_owned()]
    );
    assert_eq!(
        h.cache.keys_for_tag("bot-fields:workspace-1"),
        vec!["bot-fields:workspace-1:variable-map".to_owned()]
    );
}

/// Port of: packages/variables/__tests__/bot-field-variable.test.ts:82 "Bot Field variables > fails closed for deleted and empty Bot Fields"
#[tokio::test]
async fn bot_field_variables_fails_closed_for_deleted_and_empty_bot_fields() {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        h.v()
            .resolve_bot_field_variable_text("{{bot_field:42}}", "workspace-1")
            .await
            .unwrap(),
        BotFieldVariableTextResolution::Missing {
            field_id: "42".into()
        }
    );
    h.fake.set(|st| {
        st.bot_fields = vec![BotFieldRow {
            id: "42".into(),
            kind: "shortText".into(),
            value: None,
        }]
    });
    // A Bot Field write invalidates the workspace tags (the original's mock cache is a pass-through).
    h.cache.invalidate_tags(&["bot-fields:workspace-1"]);
    assert_eq!(
        h.v()
            .resolve_bot_field_variable_text("{{bot_field:42}}", "workspace-1")
            .await
            .unwrap(),
        BotFieldVariableTextResolution::Empty {
            field_id: "42".into()
        }
    );
}
