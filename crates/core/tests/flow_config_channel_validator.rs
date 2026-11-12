//! Port of `packages/flow-config/__tests__/channel-validator.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use fc::StepValidator;

fn base() -> Schema {
    z::object(vec![("kind", z::literal(json!("base")))])
}

fn whatsapp() -> Schema {
    z::object(vec![("kind", z::literal(json!("whatsapp")))])
}

#[test]
fn resolvestepvalidator_returns_the_schema_itself_when_a_step_validates_the_same_everywhere() {
    let base = base();
    let validator = StepValidator::Schema(base.clone());
    assert!(fc::resolve_step_validator(&validator, "whatsapp").same(&base));
}

#[test]
fn resolvestepvalidator_prefers_the_channel_s_own_schema_over_the_base() {
    let (base, whatsapp) = (base(), whatsapp());
    let validator = StepValidator::per_channel(base.clone(), vec![("whatsapp", whatsapp.clone())]);

    assert!(fc::resolve_step_validator(&validator, "whatsapp").same(&whatsapp));
}

#[test]
fn resolvestepvalidator_falls_back_to_the_base_for_a_channel_that_declares_no_rule() {
    let (base, whatsapp) = (base(), whatsapp());
    let validator = StepValidator::per_channel(base.clone(), vec![("whatsapp", whatsapp)]);

    assert!(fc::resolve_step_validator(&validator, "telegram").same(&base));
}

#[test]
fn resolvestepvalidator_falls_back_to_the_base_for_an_unknown_channel() {
    // `chooseChannelStepSchema.channel` is a plain string, so a stale or empty
    // value must resolve rather than throw.
    let base = base();
    let validator = StepValidator::per_channel(base.clone(), vec![]);

    assert!(fc::resolve_step_validator(&validator, "").same(&base));
}
