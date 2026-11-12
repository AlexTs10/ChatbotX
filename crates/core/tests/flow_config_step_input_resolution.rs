//! Port of `packages/flow-config/__tests__/step-input-resolution.test.ts`.

mod flow_config_support;
use flow_config_support::*;

const CHALLENGE_ID: &str = "11612473309626368";

#[test]
fn step_input_skip_postback_codec_round_trips_a_challenge_id() {
    let payload = fc::encode_step_input_skip_postback(CHALLENGE_ID);

    assert_eq!(
        fc::decode_step_input_skip_postback(Some(&payload)).as_deref(),
        Some(CHALLENGE_ID)
    );
}

#[test]
fn step_input_skip_postback_codec_fits_telegram_s_64_byte_callback_data_with_the_largest_id() {
    let payload = fc::encode_step_input_skip_postback("9223372036854775807");

    assert!(payload.len() <= 64);
    assert_eq!(
        fc::decode_step_input_skip_postback(Some(&payload)).as_deref(),
        Some("9223372036854775807")
    );
}

#[test]
fn step_input_skip_postback_codec_is_never_readable_as_a_flow_button_payload() {
    let payload = fc::encode_step_input_skip_postback(CHALLENGE_ID);

    assert!(!payload.contains(':'));
    assert!(fc::decode_button_payload(&payload, false).is_none());
    assert!(fc::decode_button_payload(&payload, true).is_none());
}

#[test]
fn step_input_skip_postback_codec_rejects_s() {
    let cases: [(&str, Option<&str>); 12] = [
        ("null", None),
        ("undefined", None),
        ("empty", Some("")),
        ("unrelated text", Some("hello")),
        ("a flow button payload", Some("123:456:789")),
        ("missing id", Some("STEP_INPUT_SKIP|")),
        ("non-numeric id", Some("STEP_INPUT_SKIP|abc")),
        ("id with trailing text", Some("STEP_INPUT_SKIP|123 foo")),
        ("negative id", Some("STEP_INPUT_SKIP|-1")),
        (
            "id beyond the signed bigint range",
            Some("STEP_INPUT_SKIP|9223372036854775808"),
        ),
        (
            "id with 20 digits",
            Some("STEP_INPUT_SKIP|12345678901234567890"),
        ),
        ("prefix not at the start", Some("x STEP_INPUT_SKIP|123")),
    ];
    for (label, payload) in cases {
        assert!(
            fc::decode_step_input_skip_postback(payload).is_none(),
            "{label}"
        );
    }
}

#[test]
fn step_input_resolution_payload_only_allows_reasons_that_need_no_typed_answer() {
    assert_eq!(
        fc::step_input_resolution_reasons().enum_values(),
        vec!["timeout", "user_skipped"]
    );
    assert!(
        !fc::step_input_resolution_reasons()
            .safe_parse(&json!("invalid_input_attempts"))
            .success
    );
}

#[test]
fn step_input_resolution_payload_is_accepted_by_the_metadata_union() {
    let parsed = fc::metadata_schema().safe_parse(&json!({
        "type": fc::STEP_INPUT_RESOLUTION_PAYLOAD_TYPE,
        "stepId": "1",
        "challengeId": CHALLENGE_ID,
        "reason": "timeout",
    }));

    assert!(parsed.success);
}

#[test]
fn step_input_resolution_payload_is_rejected_without_a_challenge_id_or_with_an_unknown_reason() {
    let base = json!({
        "type": fc::STEP_INPUT_RESOLUTION_PAYLOAD_TYPE,
        "stepId": "1",
        "challengeId": CHALLENGE_ID,
        "reason": "timeout",
    });

    assert!(
        !fc::metadata_schema()
            .safe_parse(&without(&base, &["challengeId"]))
            .success
    );
    assert!(
        !fc::metadata_schema()
            .safe_parse(&spread(&base, json!({ "reason": "x" })))
            .success
    );
}
