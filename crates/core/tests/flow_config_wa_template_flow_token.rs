//! Port of `packages/flow-config/__tests__/wa-template-flow-token.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use fc::ButtonPayload;

#[test]
fn whatsapp_template_flow_token_round_trips_broadcast_origin() {
    let token = json!({ "origin": "b", "broadcastId": "11612473309626368", "buttonIndex": 1 });
    let encoded = fc::encode_template_flow_token(&token);

    assert_eq!(fc::decode_template_flow_token(Some(&encoded)), Some(token));
}

#[test]
fn whatsapp_template_flow_token_round_trips_flow_step_origin_with_version_and_carousel_card() {
    let token = json!({
        "origin": "s",
        "flowId": "11612473309626368",
        "flowVersionId": "11612473309626369",
        "stepId": "11612473309626370",
        "buttonIndex": 0,
        "cardIndex": 2,
    });
    let encoded = fc::encode_template_flow_token(&token);

    assert_eq!(fc::decode_template_flow_token(Some(&encoded)), Some(token));
}

#[test]
fn whatsapp_template_flow_token_rejects_malformed_or_truncated_tokens() {
    for token in [
        "",
        "watf:b",
        "watf:s:1::2",
        "watf:b:not-a-bigint:0",
        "watf:b:9999999999999999999:0",
    ] {
        assert!(
            fc::decode_template_flow_token(Some(token)).is_none(),
            "{token}"
        );
    }
}

#[test]
fn whatsapp_template_flow_token_rejects_tokens_with_trailing_segments() {
    assert!(fc::decode_template_flow_token(Some("watf:b:11612473309626368:0::extra")).is_none());
    assert!(
        fc::decode_template_flow_token(Some(
            "watf:s:11612473309626368:11612473309626369:11612473309626370:0::extra"
        ))
        .is_none()
    );
}

#[test]
fn whatsapp_template_flow_token_rejects_empty_index_segments() {
    assert!(fc::decode_template_flow_token(Some("watf:b:11612473309626368:")).is_none());
    assert!(
        fc::decode_template_flow_token(Some(
            "watf:s:11612473309626368:11612473309626369:11612473309626370:"
        ))
        .is_none()
    );
}

#[test]
fn whatsapp_template_flow_token_does_not_collide_with_existing_button_payload_tokens() {
    let button_payload = fc::encode_button_payload(&ButtonPayload {
        flow_id: "11612473309626368".into(),
        flow_version_id: Some("11612473309626369".into()),
        button_id: Some("11612473309626370".into()),
        ..ButtonPayload::default()
    });
    let template_payload = fc::encode_template_flow_token(&json!({
        "origin": "s",
        "flowId": "11612473309626368",
        "flowVersionId": "11612473309626369",
        "stepId": "11612473309626371",
        "buttonIndex": 0,
    }));

    assert!(!fc::is_template_flow_token(Some(&button_payload)));
    assert!(fc::decode_template_flow_token(Some(&button_payload)).is_none());
    assert!(fc::decode_button_payload(&template_payload, false).is_none());
}
