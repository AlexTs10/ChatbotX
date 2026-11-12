//! Port of `packages/flow-config/__tests__/send-view-catalog-step.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use fc::SendViewCatalogLengthLimits;

fn step(overrides: Value) -> Value {
    spread(
        &json!({ "id": "1", "stepType": "sendViewCatalog", "text": "Browse our catalog" }),
        overrides,
    )
}

fn issue_codes(result: &SafeParse) -> Vec<String> {
    result.messages()
}

#[test]
fn sendviewcatalogstepdefaultfn_builds_an_empty_draft_with_a_fresh_id() {
    let first = fc::send_view_catalog_step_default_fn(&json!({}));
    let second = fc::send_view_catalog_step_default_fn(&json!({}));

    assert_match_object(
        &first,
        &json!({ "stepType": "sendViewCatalog", "text": "", "footer": "", "thumbnailProductId": "" }),
    );
    assert_ne!(first["id"], second["id"]);
}

#[test]
fn sendviewcatalogstepdefaultfn_lets_callers_override_fields_but_never_the_identity() {
    let built = fc::send_view_catalog_step_default_fn(&json!({ "text": "Hi", "footer": "Bye" }));

    assert_match_object(&built, &json!({ "text": "Hi", "footer": "Bye" }));
    assert_eq!(built["stepType"], "sendViewCatalog");
}

#[test]
fn sendviewcatalogstepschema_accepts_a_body_alone_footer_and_thumbnail_are_optional() {
    let result = fc::send_view_catalog_step_schema().safe_parse(&step(json!({})));

    assert!(result.success);
    assert_match_object(
        &result.data.unwrap(),
        &json!({ "footer": "", "thumbnailProductId": "" }),
    );
}

#[test]
fn sendviewcatalogstepschema_trims_body_footer_and_thumbnail_id() {
    let result = fc::send_view_catalog_step_schema().safe_parse(&step(json!({
        "text": "  Hello  ",
        "footer": "  Bye  ",
        "thumbnailProductId": "  sku-1  ",
    })));

    assert_match_object(
        &result.data.unwrap(),
        &json!({ "text": "Hello", "footer": "Bye", "thumbnailProductId": "sku-1" }),
    );
}

#[test]
fn sendviewcatalogstepschema_rejects_a_blank_body_j() {
    for text in ["", "   ", "\n"] {
        assert!(
            !fc::send_view_catalog_step_schema()
                .safe_parse(&step(json!({ "text": text })))
                .success,
            "{text:?}"
        );
    }
}

#[test]
fn sendviewcatalogstepschema_keeps_newlines_inside_the_footer() {
    let result = fc::send_view_catalog_step_schema()
        .safe_parse(&step(json!({ "footer": "line one\nline two" })));

    assert_eq!(result.data.unwrap()["footer"], "line one\nline two");
}

#[test]
fn sendviewcatalogstepschema_caps_body_and_footer_at_the_widest_supported_limit() {
    let body = fc::send_view_catalog_step_schema().safe_parse(&step(
        json!({ "text": rep("a", fc::SEND_VIEW_CATALOG_TEXT_MAX + 1) }),
    ));
    let footer = fc::send_view_catalog_step_schema().safe_parse(&step(
        json!({ "footer": rep("a", fc::SEND_VIEW_CATALOG_FOOTER_MAX + 1) }),
    ));

    assert_eq!(issue_codes(&body), vec!["sendViewCatalogBodyTooLong"]);
    assert_eq!(issue_codes(&footer), vec!["sendViewCatalogFooterTooLong"]);
}

#[test]
fn sendviewcatalogstepschema_accepts_exactly_the_limit() {
    assert!(
        fc::send_view_catalog_step_schema()
            .safe_parse(&step(json!({
                "text": rep("a", fc::SEND_VIEW_CATALOG_TEXT_MAX),
                "footer": rep("a", fc::SEND_VIEW_CATALOG_FOOTER_MAX),
            })))
            .success
    );
}

#[test]
fn sendviewcatalogstepschema_counts_utf_16_units_the_unit_whatsapp_s_constructors_enforce() {
    // An astral emoji is one code point but two UTF-16 units, so 513 of them
    // is 513 code points yet 1026 units: it must not slip past the cap.
    let emoji = "😀";
    let at_limit = rep(emoji, fc::SEND_VIEW_CATALOG_TEXT_MAX / 2);
    let over_limit = rep(emoji, fc::SEND_VIEW_CATALOG_TEXT_MAX / 2 + 1);

    assert!(
        fc::send_view_catalog_step_schema()
            .safe_parse(&step(json!({ "text": at_limit })))
            .success
    );
    assert_eq!(
        issue_codes(
            &fc::send_view_catalog_step_schema().safe_parse(&step(json!({ "text": over_limit })))
        ),
        vec!["sendViewCatalogBodyTooLong"]
    );
}

#[test]
fn sendviewcatalogdetailsformschema_validates_just_the_dialog_s_fields_with_the_step_s_own_limits()
{
    assert!(
        fc::send_view_catalog_details_form_schema()
            .safe_parse(&json!({ "footer": "Free shipping", "thumbnailProductId": "sku-1" }))
            .success
    );
    assert_eq!(
        issue_codes(
            &fc::send_view_catalog_details_form_schema().safe_parse(&json!({
                "footer": rep("a", fc::SEND_VIEW_CATALOG_FOOTER_MAX + 1),
                "thumbnailProductId": "",
            }))
        ),
        vec!["sendViewCatalogFooterTooLong"]
    );
}

#[test]
fn sendviewcatalogdetailsformschema_does_not_require_the_body_which_lives_outside_the_dialog() {
    assert!(
        fc::send_view_catalog_details_form_schema()
            .safe_parse(&json!({}))
            .success
    );
}

#[test]
fn resolvesendviewcataloglengthlimits_whatsapp_declares_1024_body_60_footer() {
    assert_eq!(
        fc::resolve_send_view_catalog_length_limits(Some("whatsapp")),
        SendViewCatalogLengthLimits {
            text: 1024,
            footer: 60
        }
    );
}

#[test]
fn resolvesendviewcataloglengthlimits_falls_back_to_the_widest_supported_limits_for_s() {
    for channel in [Some("omnichannel"), Some("unknown"), None] {
        assert_eq!(
            fc::resolve_send_view_catalog_length_limits(channel),
            SendViewCatalogLengthLimits {
                text: fc::SEND_VIEW_CATALOG_TEXT_MAX,
                footer: fc::SEND_VIEW_CATALOG_FOOTER_MAX
            },
            "{channel:?}"
        );
    }
}

fn node_step(overrides: Value) -> SafeParse {
    fc::send_message_node_schema()
        .field("data")
        .field("details")
        .field("steps")
        .element()
        .safe_parse(&step(overrides))
}

#[test]
fn sendmessage_node_accepts_a_node_carrying_the_step() {
    assert!(node_step(json!({})).success);
}

#[test]
fn sendmessage_node_rejects_an_over_limit_body_on_a_raw_graph_so_nothing_is_clamped_silently_at_send_time()
 {
    assert_eq!(
        issue_codes(&node_step(
            json!({ "text": rep("a", fc::SEND_VIEW_CATALOG_TEXT_MAX + 1) })
        )),
        vec!["sendViewCatalogBodyTooLong"]
    );
}

#[test]
fn channel_policy_only_whatsapp_and_omnichannel_deliver_the_step_today() {
    let support_by_channel: Vec<(&str, &str)> = fc::channel_flow_policies()
        .iter()
        .map(|(channel, policy)| (*channel, policy.steps["sendViewCatalog"]))
        .collect();
    let support = fc::step_support();
    let (full, unsupported) = (
        support["full"].as_str().unwrap(),
        support["unsupported"].as_str().unwrap(),
    );

    assert_eq!(
        support_by_channel,
        vec![
            ("api", unsupported),
            ("instagram", unsupported),
            ("messenger", unsupported),
            ("omnichannel", full),
            ("smtp", unsupported),
            ("telegram", unsupported),
            ("threads", unsupported),
            ("tiktok", unsupported),
            ("webchat", unsupported),
            ("whatsapp", full),
            ("zalo", unsupported),
        ]
    );
}

#[test]
fn channel_policy_the_externally_reported_policy_version_was_bumped_for_the_new_support() {
    assert_eq!(fc::CHANNEL_POLICY_VERSION, 3);
}
