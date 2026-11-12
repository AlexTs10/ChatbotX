//! Port of `packages/variables/__tests__/contact-variable.test.ts`.

mod support;

use chatbotx_core::variables::bot_field::format_bot_field_reference;
use chatbotx_core::variables::contact_variable::{ContactInboxArg, GetAllInput};
use chatbotx_core::variables::text::extract_variables;
use chatbotx_core::variables::*;
use support::iso_ms;
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

fn contact_inbox() -> ContactInboxModel {
    ContactInboxModel {
        id: "contact-inbox-1".into(),
        created_at: Some(iso_ms("2026-01-02T03:04:05.000Z")),
        ..Default::default()
    }
}

fn workspace() -> WorkspaceModel {
    WorkspaceModel {
        id: "workspace-1".into(),
        timezone: s("UTC"),
        ..Default::default()
    }
}

/// `createVariables(fields, botFields)`: custom fields default to `text`.
fn variables(
    fields: &[(&str, &str, Option<&str>)],
    bot: &[(&str, Option<&str>)],
) -> VariableContext {
    let bot: Vec<(&str, &str, Option<&str>)> = bot
        .iter()
        .map(|(id, value)| (*id, "text", *value))
        .collect();
    VariableContext {
        contact: contact(),
        contact_inbox: Some(contact_inbox()),
        custom_fields_map: custom_fields(fields),
        bot_fields_map: Some(bot_fields(&bot)),
        workspace: Some(workspace()),
        ..Default::default()
    }
}

async fn replace(h: &Harness, text: &str, variables: &VariableContext) -> String {
    h.v().replace_all(text, variables, None).await.unwrap()
}

fn token(id: &str) -> String {
    format!("{{{{{}}}}}", format_bot_field_reference(id))
}

fn gender_variables(gender: Option<&str>, language: &str) -> VariableContext {
    VariableContext {
        contact: ContactModel {
            gender: gender.map(str::to_owned),
            ..contact()
        },
        workspace: Some(WorkspaceModel {
            language: s(language),
            ..workspace()
        }),
        ..variables(&[], &[])
    }
}

async fn render_gender(
    workspace_language: Option<&str>,
    contact_locale: Option<&str>,
    inbox_language: Option<&str>,
) -> String {
    let h = Harness::new(FakeState::default());
    let variables = VariableContext {
        contact: ContactModel {
            gender: s("female"),
            locale: contact_locale.map(str::to_owned),
            ..contact()
        },
        contact_inbox: Some(ContactInboxModel {
            language: inbox_language.map(str::to_owned),
            ..contact_inbox()
        }),
        workspace: Some(WorkspaceModel {
            language: workspace_language.map(str::to_owned),
            ..workspace()
        }),
        ..variables(&[], &[])
    };
    replace(&h, "{{gender}}", &variables).await
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:153 "contactVariableService.replaceAll > renders null system fields as empty strings"
#[tokio::test]
async fn contactvariableservice_replaceall_renders_null_system_fields_as_empty_strings() {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(&h, "Locale: {{locale}}.", &variables(&[], &[])).await,
        "Locale: ."
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:162 "contactVariableService.replaceAll > substitutes system fields with values"
#[tokio::test]
async fn contactvariableservice_replaceall_substitutes_system_fields_with_values() {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(&h, "First name: {{first_name}}.", &variables(&[], &[])).await,
        "First name: Ada."
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:171 "contactVariableService.replaceAll > renders missing custom field values as empty strings"
#[tokio::test]
async fn contactvariableservice_replaceall_renders_missing_custom_field_values_as_empty_strings() {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            "Plan: {{plan}}.",
            &variables(&[("plan", "text", None)], &[])
        )
        .await,
        "Plan: ."
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:185 "contactVariableService.replaceAll > keeps unknown placeholders literal"
#[tokio::test]
async fn contactvariableservice_replaceall_keeps_unknown_placeholders_literal() {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            "{{not_a_field}} {{locale2}} {{status}}",
            &variables(&[("status", "text", Some("active"))], &[])
        )
        .await,
        "{{not_a_field}}  active"
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:199 "contactVariableService.replaceAll > resolves a bare custom field name containing spaces and diacritics"
#[tokio::test]
async fn contactvariableservice_replaceall_resolves_a_bare_custom_field_name_containing_spaces_and_diacritics()
 {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            "Xin chào {{fullname upper}}!",
            &variables(&[("fullname upper", "text", Some("MÁ CHÁN"))], &[])
        )
        .await,
        "Xin chào MÁ CHÁN!"
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:216 "contactVariableService.replaceAll > still routes coupon: and raw: prefixes correctly after widening the bare-name branch"
#[tokio::test]
async fn contactvariableservice_replaceall_still_routes_coupon_and_raw_prefixes_correctly_after_widening_the_bare_name_branch()
 {
    let h = Harness::new(FakeState {
        coupon: "CODE1".into(),
        ..Default::default()
    });
    assert_eq!(
        replace(
            &h,
            "{{coupon:topic-1}} {{raw:Full Name}}",
            &variables(&[("Full Name", "text", Some("Ada Lovelace"))], &[])
        )
        .await,
        "CODE1 Ada Lovelace"
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:229 "contactVariableService.replaceAll > still leaves an unknown bare key literal after widening the bare-name branch"
#[tokio::test]
async fn contactvariableservice_replaceall_still_leaves_an_unknown_bare_key_literal_after_widening_the_bare_name_branch()
 {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(&h, "{{totally unknown}}", &variables(&[], &[])).await,
        "{{totally unknown}}"
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:238 "contactVariableService.replaceAll > supports dotted system variables and keeps unknown dotted placeholders literal"
#[tokio::test]
async fn contactvariableservice_replaceall_supports_dotted_system_variables_and_keeps_unknown_dotted_placeholders_literal()
 {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            "{{ai.queued.messages}} {{foo.bar}}",
            &variables(&[], &[])
        )
        .await,
        " {{foo.bar}}"
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:247 "contactVariableService.replaceAll > substitutes coupon topic variables with issued coupon code"
#[tokio::test]
async fn contactvariableservice_replaceall_substitutes_coupon_topic_variables_with_issued_coupon_code()
 {
    let h = Harness::new(FakeState {
        coupon: "HHFgpe".into(),
        ..Default::default()
    });
    assert_eq!(
        replace(
            &h,
            "Mã giảm giá của bạn là {{coupon:11619011544072192}}",
            &variables(&[], &[])
        )
        .await,
        "Mã giảm giá của bạn là HHFgpe"
    );
    assert!(
        h.fake
            .called("resolve_coupon_variable(workspace-1, contact-1, 11619011544072192)")
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:264 "contactVariableService.replaceAll > extracts and resolves raw custom field variables verbatim"
#[tokio::test]
async fn contactvariableservice_replaceall_extracts_and_resolves_raw_custom_field_variables_verbatim()
 {
    assert_eq!(
        extract_variables("{{raw:Full Name}} {{raw:Ngày sinh}}"),
        vec!["raw:Full Name".to_owned(), "raw:Ngày sinh".to_owned()]
    );
    let h = Harness::new(FakeState::default());
    let vars = variables(
        &[
            ("Full Name", "text", Some("Ada Lovelace")),
            ("Ngày sinh", "date", Some("2026-07-23T00:00:00.000Z")),
        ],
        &[],
    );
    assert_eq!(
        replace(&h, "{{raw:Full Name}} {{raw:Ngày sinh}}", &vars).await,
        "Ada Lovelace 2026-07-23T00:00:00.000Z"
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:288 "contactVariableService.replaceAll > keeps unknown raw variables literal and preserves a real raw-prefixed field name"
#[tokio::test]
async fn contactvariableservice_replaceall_keeps_unknown_raw_variables_literal_and_preserves_a_real_raw_prefixed_field_name()
 {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            "{{raw:Missing}} {{raw:X}}",
            &variables(&[("raw:X", "text", Some("field named raw colon x"))], &[])
        )
        .await,
        "{{raw:Missing}} field named raw colon x"
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:302 "contactVariableService.replaceAll > does not render custom field null values as string null"
#[tokio::test]
async fn contactvariableservice_replaceall_does_not_render_custom_field_null_values_as_string_null()
{
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            "Broken: {{broken}}.",
            &variables(&[("broken", "text", None)], &[])
        )
        .await,
        "Broken: ."
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:316 "contactVariableService.replaceAll > sanity-checks referenced system field names"
#[test]
fn contactvariableservice_replaceall_sanity_checks_referenced_system_field_names() {
    assert!(SystemFieldType::ALL.iter().any(|k| k.as_str() == "locale"));
    assert!(
        SystemFieldType::ALL
            .iter()
            .any(|k| k.as_str() == "first_name")
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:323 "contactVariableService.replaceAll bot fields > resolves a bot_field:<id> token to the workspace field's stored value"
#[tokio::test]
async fn contactvariableservice_replaceall_bot_fields_resolves_a_bot_field_id_token_to_the_workspace_field_s_stored_value()
 {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            &format!("Support hours: {}", token("1")),
            &variables(&[], &[("1", Some("9am - 5pm"))])
        )
        .await,
        "Support hours: 9am - 5pm"
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:332 "contactVariableService.replaceAll bot fields > renders a bot field with no stored value as an empty string"
#[tokio::test]
async fn contactvariableservice_replaceall_bot_fields_renders_a_bot_field_with_no_stored_value_as_an_empty_string()
 {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            &format!("Hours: {}.", token("1")),
            &variables(&[], &[("1", None)])
        )
        .await,
        "Hours: ."
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:341 "contactVariableService.replaceAll bot fields > keeps a bot_field token for a deleted/unknown id literal, matching unknown-variable behavior"
#[tokio::test]
async fn contactvariableservice_replaceall_bot_fields_keeps_a_bot_field_token_for_a_deleted_unknown_id_literal_matching_unknown_variable_behavior()
 {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            &token("999"),
            &variables(&[], &[("1", Some("9am - 5pm"))])
        )
        .await,
        token("999")
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:350 "contactVariableService.replaceAll bot fields > never collides a bot_field token with a contact custom field of the same name"
#[tokio::test]
async fn contactvariableservice_replaceall_bot_fields_never_collides_a_bot_field_token_with_a_contact_custom_field_of_the_same_name()
 {
    let h = Harness::new(FakeState::default());
    let key = format_bot_field_reference("1");
    let vars = variables(
        &[(key.as_str(), "text", Some("wrong value"))],
        &[("1", Some("right value"))],
    );
    assert_eq!(replace(&h, &token("1"), &vars).await, "right value");
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:370 "contactVariableService.replaceAll gender casing > capitalises {{gender}} when it opens the text"
#[tokio::test]
async fn contactvariableservice_replaceall_gender_casing_capitalises_gender_when_it_opens_the_text()
{
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            "{{gender}} vui lòng xác nhận đơn hàng.",
            &gender_variables(Some("male"), "vi")
        )
        .await,
        "Anh vui lòng xác nhận đơn hàng."
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:379 "contactVariableService.replaceAll gender casing > lowercases {{gender}} inside a sentence"
#[tokio::test]
async fn contactvariableservice_replaceall_gender_casing_lowercases_gender_inside_a_sentence() {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            "Xin chào {{gender}}, đơn hàng đã được giao.",
            &gender_variables(Some("female"), "vi")
        )
        .await,
        "Xin chào chị, đơn hàng đã được giao."
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:388 "contactVariableService.replaceAll gender casing > capitalises {{gender}} after a sentence break and after a newline"
#[tokio::test]
async fn contactvariableservice_replaceall_gender_casing_capitalises_gender_after_a_sentence_break_and_after_a_newline()
 {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            "Cảm ơn. {{gender}} nhé!\n{{gender}} cần hỗ trợ gì thêm không?",
            &gender_variables(None, "vi")
        )
        .await,
        "Cảm ơn. Anh/Chị nhé!\nAnh/Chị cần hỗ trợ gì thêm không?"
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:397 "contactVariableService.replaceAll gender casing > keeps the Vietnamese labels for a region-tagged workspace language"
#[tokio::test]
async fn contactvariableservice_replaceall_gender_casing_keeps_the_vietnamese_labels_for_a_region_tagged_workspace_language()
 {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            "Kính gửi {{gender}}",
            &gender_variables(Some("female"), "vi-VN")
        )
        .await,
        "Kính gửi chị"
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:406 "contactVariableService.replaceAll gender casing > falls back to the English labels for other workspace languages"
#[tokio::test]
async fn contactvariableservice_replaceall_gender_casing_falls_back_to_the_english_labels_for_other_workspace_languages()
 {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            "{{gender}} — hello {{gender}}",
            &gender_variables(Some("male"), "de")
        )
        .await,
        "Male — hello male"
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:415 "contactVariableService.replaceAll gender casing > leaves other variables untouched by the sentence casing"
#[tokio::test]
async fn contactvariableservice_replaceall_gender_casing_leaves_other_variables_untouched_by_the_sentence_casing()
 {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        replace(
            &h,
            "Hi {{first_name}}, {{gender}}!",
            &gender_variables(Some("male"), "vi")
        )
        .await,
        "Hi Ada, anh!"
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:451 "contactVariableService.replaceAll gender language > renders Vietnamese for a Vietnamese workspace"
#[tokio::test]
async fn contactvariableservice_replaceall_gender_language_renders_vietnamese_for_a_vietnamese_workspace()
 {
    assert_eq!(render_gender(Some("vi"), None, None).await, "Chị");
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:455 "contactVariableService.replaceAll gender language > renders English for any non-Vietnamese workspace"
#[tokio::test]
async fn contactvariableservice_replaceall_gender_language_renders_english_for_any_non_vietnamese_workspace()
 {
    assert_eq!(render_gender(Some("en"), None, None).await, "Female");
    assert_eq!(render_gender(Some("de"), None, None).await, "Female");
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:460 "contactVariableService.replaceAll gender language > ignores the contact channel language"
#[tokio::test]
async fn contactvariableservice_replaceall_gender_language_ignores_the_contact_channel_language() {
    assert_eq!(render_gender(Some("en"), None, Some("vi")).await, "Female");
    assert_eq!(render_gender(Some("vi"), None, Some("en")).await, "Chị");
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:469 "contactVariableService.replaceAll gender language > ignores the contact locale"
#[tokio::test]
async fn contactvariableservice_replaceall_gender_language_ignores_the_contact_locale() {
    assert_eq!(
        render_gender(Some("en"), Some("vi_VN"), None).await,
        "Female"
    );
    assert_eq!(render_gender(Some("vi"), Some("en_US"), None).await, "Chị");
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:478 "contactVariableService.replaceAll gender language > falls back to English when the workspace has no language"
#[tokio::test]
async fn contactvariableservice_replaceall_gender_language_falls_back_to_english_when_the_workspace_has_no_language()
 {
    assert_eq!(render_gender(None, Some("vi_VN"), None).await, "Female");
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:486 "contactVariableService.getAll > uses a provided contact inbox object and skips the inbox query"
#[tokio::test]
async fn contactvariableservice_getall_uses_a_provided_contact_inbox_object_and_skips_the_inbox_query()
 {
    let h = Harness::new(FakeState {
        contact: Some(contact()),
        workspace: Some(workspace()),
        custom_fields: vec![ContactCustomFieldValue {
            key: "plan".into(),
            kind: "text".into(),
            value: s("gold"),
            description: "Plan".into(),
        }],
        ..Default::default()
    });
    let result = h
        .v()
        .get_all(GetAllInput {
            contact_id: "contact-1".into(),
            contact_inbox: ContactInboxArg::Model(contact_inbox()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(result.contact, contact());
    assert_eq!(result.contact_inbox, Some(contact_inbox()));
    assert_eq!(result.workspace, Some(workspace()));
    assert_eq!(h.fake.count("find_contact_inbox"), 0);
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:513 "contactVariableService.getAll > loads a contact inbox once when an id is provided"
#[tokio::test]
async fn contactvariableservice_getall_loads_a_contact_inbox_once_when_an_id_is_provided() {
    let h = Harness::new(FakeState {
        contact: Some(contact()),
        contact_inbox: Ok(Some(contact_inbox())),
        workspace: Some(workspace()),
        ..Default::default()
    });
    let result = h
        .v()
        .get_all(GetAllInput {
            contact_id: "contact-1".into(),
            contact_inbox: ContactInboxArg::Id("contact-inbox-1".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(result.contact_inbox, Some(contact_inbox()));
    assert_eq!(
        h.fake.calls_to("find_contact_inbox"),
        vec!["find_contact_inbox(contact-inbox-1)"]
    );
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:532 "contactVariableService.getAll > uses the provided workspace and skips the workspace query"
#[tokio::test]
async fn contactvariableservice_getall_uses_the_provided_workspace_and_skips_the_workspace_query() {
    let h = Harness::new(FakeState {
        contact: Some(contact()),
        ..Default::default()
    });
    let result = h
        .v()
        .get_all(GetAllInput {
            contact_id: "contact-1".into(),
            contact_inbox: ContactInboxArg::Model(contact_inbox()),
            workspace: Some(workspace()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(result.workspace, Some(workspace()));
    assert_eq!(h.fake.count("find_workspace"), 0);
}

/// Port of: packages/variables/__tests__/contact-variable.test.ts:548 "contactVariableService.getAll > builds botFieldsMap from the contact's workspace, keyed by field id"
#[tokio::test]
async fn contactvariableservice_getall_builds_botfieldsmap_from_the_contact_s_workspace_keyed_by_field_id()
 {
    let h = Harness::new(FakeState {
        contact: Some(contact()),
        bot_fields: vec![BotFieldRow {
            id: "1".into(),
            kind: "shortText".into(),
            value: s("9am - 5pm"),
        }],
        ..Default::default()
    });
    let result = h
        .v()
        .get_all(GetAllInput {
            contact_id: "contact-1".into(),
            contact_inbox: ContactInboxArg::Model(contact_inbox()),
            workspace: Some(workspace()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(h.fake.called("list_bot_fields(workspace-1)"));
    assert_eq!(
        result.bot_fields_map.unwrap().get("1"),
        Some(&BotFieldValue {
            kind: "shortText".into(),
            value: s("9am - 5pm")
        })
    );
}
