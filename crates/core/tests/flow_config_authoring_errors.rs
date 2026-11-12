//! Port of `packages/flow-config/__tests__/authoring/errors.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use fc::{FlowAuthoringError, FlowAuthoringException};

fn get_invalid_spec_error() -> z::ZodError {
    let result = z::object(vec![(
        "items",
        z::array(z::object(vec![("name", z::string().min(3))])),
    )])
    .safe_parse(&json!({ "items": [{ "name": "" }] }));
    result.error.expect("expected schema parsing to fail")
}

#[test]
fn closestnames_returns_an_exact_match() {
    assert_eq!(
        fc::closest_names("welcome", &["welcome", "other"]),
        vec!["welcome"]
    );
}

#[test]
fn closestnames_returns_a_typo_match() {
    assert_eq!(
        fc::closest_names("welcom_promo", &["welcome_promo"]),
        vec!["welcome_promo"]
    );
}

#[test]
fn closestnames_returns_a_prefix_match() {
    assert_eq!(
        fc::closest_names("wel", &["welcome_promo"]),
        vec!["welcome_promo"]
    );
}

#[test]
fn closestnames_omits_unrelated_names() {
    assert!(fc::closest_names("welcome", &["archive", "assignment"]).is_empty());
}

#[test]
fn closestnames_caps_results_at_three_candidates() {
    assert_eq!(
        fc::closest_names("alpha", &["alpha", "alpha1", "alpha2", "alpha3"]),
        vec!["alpha", "alpha1", "alpha2"]
    );
}

fn code_and_path(errors: &[FlowAuthoringError]) -> Vec<(String, String)> {
    errors
        .iter()
        .map(|e| (e.code.clone(), e.path.clone()))
        .collect()
}

#[test]
fn zoderrortoflowauthoringerrors_uses_a_supplied_path_mapper() {
    let mapper = |issue_path: &[PathSegment]| {
        Some(
            issue_path
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("/"),
        )
    };
    let errors = fc::zod_error_to_flow_authoring_errors(
        &get_invalid_spec_error(),
        "invalidStep",
        Some(&mapper),
    );

    assert_eq!(
        code_and_path(&errors),
        vec![("invalidStep".into(), "items/0/name".into())]
    );
}

#[test]
fn zoderrortoflowauthoringerrors_uses_the_issue_path_without_a_mapper() {
    let errors =
        fc::zod_error_to_flow_authoring_errors(&get_invalid_spec_error(), "invalidSpec", None);

    assert_eq!(
        code_and_path(&errors),
        vec![("invalidSpec".into(), "items[0].name".into())]
    );
}

#[test]
fn zoderrortoflowauthoringerrors_falls_back_to_the_issue_path_when_the_mapper_has_no_mapping() {
    let mapper = |_: &[PathSegment]| None;
    let errors = fc::zod_error_to_flow_authoring_errors(
        &get_invalid_spec_error(),
        "invalidStep",
        Some(&mapper),
    );

    assert_eq!(
        code_and_path(&errors),
        vec![("invalidStep".into(), "items[0].name".into())]
    );
}

#[test]
fn flowauthoringexception_formats_channel_capabilities_without_exposing_validation_codes() {
    let error = FlowAuthoringException::new(vec![
        FlowAuthoringError {
            capability: Some(json!({
                "block": "sendCard",
                "channel": "instagram",
                "code": "unsupportedBlock",
                "policyVersion": 1,
            })),
            code: "unsupportedBlock".into(),
            message: "unsupportedBlock".into(),
            path: "steps[0]".into(),
            ..FlowAuthoringError::default()
        },
        FlowAuthoringError {
            capability: Some(json!({
                "actual": 4,
                "allowed": 3,
                "block": "sendText",
                "channel": "tiktok",
                "code": "constraintExceeded",
                "policyVersion": 1,
                "unit": "buttons",
            })),
            code: "constraintExceeded".into(),
            message: "constraintExceeded".into(),
            path: "steps[1]".into(),
            ..FlowAuthoringError::default()
        },
    ]);

    assert_eq!(
        error.message(),
        "steps[0]: The instagram channel does not support sendCard.; steps[1]: sendText exceeds the tiktok maximum of 3 buttons (received 4)."
    );
}

#[test]
fn formatzodpathsegment_formats_object_keys_and_array_indices() {
    assert_eq!(
        fc::format_zod_path_segment("steps", &PathSegment::Index(0)),
        "steps[0]"
    );
    assert_eq!(
        fc::format_zod_path_segment("steps[0]", &PathSegment::from("templateName")),
        "steps[0].templateName"
    );
}
