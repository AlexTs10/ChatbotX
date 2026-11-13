//! Port of `packages/database/__tests__/thread-control-sdk-parity.test.ts` (P3.2b; lives in
//! chatbotx-core because it compares the db partial with `flow_config`). Ported: the flow
//! step's pass targets and note limit. Adapted: there is no Rust SDK, so the
//! `@chatbotx.io/sdk` values are pinned as a copy of `packages/sdk/src/lib/shared/message.ts`
//! (`threadControlRoles`, `threadControlActions`).

use chatbotx_core::flow_config::steps::{
    THREAD_CONTROL_STEP_METADATA_MAX_LENGTH, thread_control_step_target_roles,
};
use chatbotx_db::repo::partials::thread_control::{
    THREAD_CONTROL_ACTIONS, THREAD_CONTROL_METADATA_MAX_LENGTH, THREAD_CONTROL_ROLES,
};

/// `threadControlRoles.options` of `@chatbotx.io/sdk`.
const SDK_ROLES: &[&str] = &[
    "ai_agent",
    "ctwa",
    "customer_service",
    "escalation",
    "marketing",
    "utility",
];

/// `threadControlActions.options` of `@chatbotx.io/sdk`.
const SDK_ACTIONS: &[&str] = &["take", "release", "pass"];

#[test]
fn thread_control_sdk_parity_keeps_the_sdk_role_values_identical_to_the_persisted_ones() {
    assert_eq!(SDK_ROLES, THREAD_CONTROL_ROLES);
}

#[test]
fn thread_control_sdk_parity_keeps_the_sdk_action_values_identical_to_the_persisted_ones() {
    assert_eq!(SDK_ACTIONS, THREAD_CONTROL_ACTIONS);
}

#[test]
fn thread_control_sdk_parity_keeps_the_flow_step_s_pass_targets_identical_to_the_persisted_roles() {
    assert_eq!(
        thread_control_step_target_roles().enum_values(),
        THREAD_CONTROL_ROLES
    );
}

#[test]
fn thread_control_sdk_parity_keeps_the_flow_step_s_note_limit_identical_to_the_persisted_one() {
    assert_eq!(
        THREAD_CONTROL_STEP_METADATA_MAX_LENGTH,
        THREAD_CONTROL_METADATA_MAX_LENGTH
    );
}
