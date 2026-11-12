//! Port of `packages/variables/__tests__/deep-replace-contact-variables.test.ts`. `Math.random` (the spintax
//! seam) is the `pick` option.

mod support;

use chatbotx_core::variables::bot_field::format_bot_field_reference;
use chatbotx_core::variables::contact_variable::ContactInboxArg;
use chatbotx_core::variables::deep_replace::{DeepOptions, DeepSource};
use chatbotx_core::variables::*;
use serde_json::json;
use support::variables::*;

fn contact() -> ContactModel {
    ContactModel {
        id: "contact-1".into(),
        workspace_id: "workspace-1".into(),
        first_name: s("Ada"),
        locale: None,
        timezone: s("UTC"),
        ..Default::default()
    }
}

fn source() -> DeepSource {
    DeepSource {
        contact_inbox: ContactInboxArg::Model(ContactInboxModel {
            id: "contact-inbox-1".into(),
            ..Default::default()
        }),
        workspace: Some(WorkspaceModel {
            id: "workspace-1".into(),
            timezone: s("UTC"),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn custom_field(name: &str, value: &str) -> ContactCustomFieldValue {
    ContactCustomFieldValue {
        key: name.into(),
        kind: "shortText".into(),
        value: s(value),
        description: String::new(),
    }
}

fn bot_field(id: &str, value: &str) -> BotFieldRow {
    BotFieldRow {
        id: id.into(),
        kind: "shortText".into(),
        value: s(value),
    }
}

/// `vi.spyOn(Math, "random").mockReturnValue(0)`: always the first branch.
const PICK_FIRST: DeepOptions = DeepOptions {
    spintax: false,
    pick: Some(first_branch),
};

fn first_branch(_: usize) -> usize {
    0
}

/// Port of: packages/variables/__tests__/deep-replace-contact-variables.test.ts:83 "resolveContactVariablesDeep bot fields > resolves a bot_field:<id> token nested inside an object structure"
#[tokio::test]
async fn resolvecontactvariablesdeep_bot_fields_resolves_a_bot_field_id_token_nested_inside_an_object_structure()
 {
    let h = Harness::new(FakeState {
        contact: Some(contact()),
        bot_fields: vec![bot_field("1", "9am - 5pm")],
        ..Default::default()
    });
    let input = json!({ "message": format!("Hours: {{{{{}}}}}", format_bot_field_reference("1")) });
    let result = h
        .v()
        .resolve_contact_variables_deep("contact-1", &input, source(), DeepOptions::default())
        .await
        .unwrap();
    assert_eq!(result, json!({ "message": "Hours: 9am - 5pm" }));
    assert!(h.fake.called("list_bot_fields(workspace-1)"));
}

/// Port of: packages/variables/__tests__/deep-replace-contact-variables.test.ts:101 "resolveContactVariablesDeep bot fields > returns the input unchanged, without loading contact data, when it has no {{ placeholder"
#[tokio::test]
async fn resolvecontactvariablesdeep_bot_fields_returns_the_input_unchanged_without_loading_contact_data_when_it_has_no_placeholder()
 {
    let h = Harness::new(FakeState::default());
    let input = json!({ "message": "No variables here" });
    let result = h
        .v()
        .resolve_contact_variables_deep("contact-1", &input, source(), DeepOptions::default())
        .await
        .unwrap();
    assert_eq!(result, json!({ "message": "No variables here" }));
    assert_eq!(h.fake.count("find_contact"), 0);
    assert_eq!(h.fake.count("list_bot_fields"), 0);
}

/// Port of: packages/variables/__tests__/deep-replace-contact-variables.test.ts:133 "resolveContactVariablesDeep bot fields > resolves known tokens at every depth of a realistic nested payload, leaves an unknown bot field literal, and never mutates the source object"
#[tokio::test]
async fn resolvecontactvariablesdeep_bot_fields_resolves_known_tokens_at_every_depth_of_a_realistic_nested_payload_leaves_an_unknown_bot_field_literal_and_never_mutates_the_source_object()
 {
    let h = Harness::new(FakeState {
        contact: Some(contact()),
        custom_fields: vec![custom_field("company", "Acme Inc")],
        bot_fields: vec![bot_field("1", "Enterprise Plan")],
        ..Default::default()
    });
    let known = format!("{{{{{}}}}}", format_bot_field_reference("1"));
    let unknown = format!("{{{{{}}}}}", format_bot_field_reference("999"));
    let build_payload = || {
        json!({
            "event": "order.created",
            "headers": { "x-workspace": "workspace-1" },
            "jsonBody": {
                "customer": { "greeting": "Hi {{company}}", "tags": ["vip", format!("Plan: {known}")] },
                "items": [
                    { "sku": "SKU-1", "note": format!("Legacy note: {unknown}") },
                    { "sku": "SKU-2", "note": "no variables here" }
                ]
            },
            "meta": null
        })
    };
    // The input is borrowed immutably: the walk can only build a new value.
    let input = build_payload();
    let result = h
        .v()
        .resolve_contact_variables_deep("contact-1", &input, source(), DeepOptions::default())
        .await
        .unwrap();
    assert_eq!(
        result,
        json!({
            "event": "order.created",
            "headers": { "x-workspace": "workspace-1" },
            "jsonBody": {
                "customer": { "greeting": "Hi Acme Inc", "tags": ["vip", "Plan: Enterprise Plan"] },
                "items": [
                    { "sku": "SKU-1", "note": format!("Legacy note: {unknown}") },
                    { "sku": "SKU-2", "note": "no variables here" }
                ]
            },
            "meta": null
        })
    );
    assert_eq!(input, build_payload());
    assert_ne!(result, input);
}

/// Port of: packages/variables/__tests__/deep-replace-contact-variables.test.ts:212 "resolveContactVariablesDeep spintax > leaves a {a|b} block alone by default"
#[tokio::test]
async fn resolvecontactvariablesdeep_spintax_leaves_a_a_b_block_alone_by_default() {
    let h = Harness::new(FakeState {
        contact: Some(contact()),
        ..Default::default()
    });
    let input = json!({ "message": "{Chào|Hi} {{first_name}}" });
    let result = h
        .v()
        .resolve_contact_variables_deep("contact-1", &input, source(), PICK_FIRST)
        .await
        .unwrap();
    assert_eq!(result, json!({ "message": "{Chào|Hi} Ada" }));
}

/// Port of: packages/variables/__tests__/deep-replace-contact-variables.test.ts:226 "resolveContactVariablesDeep spintax > picks a branch when the caller opts in"
#[tokio::test]
async fn resolvecontactvariablesdeep_spintax_picks_a_branch_when_the_caller_opts_in() {
    let h = Harness::new(FakeState {
        contact: Some(contact()),
        ..Default::default()
    });
    let input = json!({ "message": "{Chào|Hi} {{first_name}}" });
    let options = DeepOptions {
        spintax: true,
        ..PICK_FIRST
    };
    let result = h
        .v()
        .resolve_contact_variables_deep("contact-1", &input, source(), options)
        .await
        .unwrap();
    assert_eq!(result, json!({ "message": "Chào Ada" }));
}

/// Port of: packages/variables/__tests__/deep-replace-contact-variables.test.ts:243 "resolveContactVariablesDeep spintax > spins text that has no {{ placeholder, without loading contact data"
#[tokio::test]
async fn resolvecontactvariablesdeep_spintax_spins_text_that_has_no_placeholder_without_loading_contact_data()
 {
    let h = Harness::new(FakeState::default());
    let input = json!({ "message": "{Chào|Hi} bạn" });
    let options = DeepOptions {
        spintax: true,
        ..PICK_FIRST
    };
    let result = h
        .v()
        .resolve_contact_variables_deep("contact-1", &input, source(), options)
        .await
        .unwrap();
    assert_eq!(result, json!({ "message": "Chào bạn" }));
    assert_eq!(h.fake.count("find_contact"), 0);
    assert_eq!(h.fake.count("list_bot_fields"), 0);
}

/// Port of: packages/variables/__tests__/deep-replace-contact-variables.test.ts:260 "resolveContactVariablesDeep spintax > never spins a value substituted from contact data"
#[tokio::test]
async fn resolvecontactvariablesdeep_spintax_never_spins_a_value_substituted_from_contact_data() {
    let h = Harness::new(FakeState {
        contact: Some(contact()),
        custom_fields: vec![custom_field("note", "Giá {tốt|xấu}")],
        ..Default::default()
    });
    let input = json!({ "message": "{Chào|Hi}: {{note}}" });
    let options = DeepOptions {
        spintax: true,
        ..PICK_FIRST
    };
    let result = h
        .v()
        .resolve_contact_variables_deep("contact-1", &input, source(), options)
        .await
        .unwrap();
    assert_eq!(result, json!({ "message": "Chào: Giá {tốt|xấu}" }));
}
