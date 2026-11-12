//! Port of `packages/utils/__tests__/connection.test.ts`. The zod object of
//! `zodToConfigFields` becomes the JSON Schema `z.toJSONSchema` emits for
//! it (`JsonSchema`, property order kept); a thrown error is its JSON-like
//! fields.

use chatbotx_core::utils::connection::{
    ACTIVE_CONNECT_SESSION_STATUSES, ACTIVE_CONNECTION_STATUSES, ConnectFailureCause,
    ConnectSessionStatus, ConnectionConfigField, ConnectionStatus, ConnectionStatusReason,
    INACTIVE_CONNECTION_STATUSES, InboxDisconnectReason, JsonSchema,
    TERMINAL_CONNECT_SESSION_STATUSES, append_connect_error, connect_failure_cause_of,
    connection_to_inbox_disconnect_reason, parse_connect_session_next_action, zod_to_config_fields,
};
use serde_json::{Value, json};

/// Port of: packages/utils/__tests__/connection.test.ts:23 "CONNECTION_TO_INBOX_DISCONNECT_REASON > maps every connection status reason"
#[test]
fn connection_to_inbox_disconnect_reason_maps_every_connection_status_reason() {
    // An exhaustive `match` maps every reason; each one has a value.
    let mut mapped: Vec<&str> = ConnectionStatusReason::ALL
        .iter()
        .map(|reason| {
            connection_to_inbox_disconnect_reason(*reason);
            reason.as_str()
        })
        .collect();
    let mut all: Vec<&str> = ConnectionStatusReason::ALL
        .iter()
        .map(|r| r.as_str())
        .collect();
    mapped.sort_unstable();
    all.sort_unstable();
    assert_eq!(mapped, all);
}

/// Port of: packages/utils/__tests__/connection.test.ts:29 "CONNECTION_TO_INBOX_DISCONNECT_REASON > maps every connection status reason to a valid inbox disconnect reason"
#[test]
fn connection_to_inbox_disconnect_reason_maps_every_connection_status_reason_to_a_valid_inbox_disconnect_reason()
 {
    for reason in ConnectionStatusReason::ALL {
        let mapped = connection_to_inbox_disconnect_reason(*reason);
        assert!(InboxDisconnectReason::ALL.contains(&mapped));
    }
}

/// Port of: packages/utils/__tests__/connection.test.ts:37 "CONNECTION_TO_INBOX_DISCONNECT_REASON > maps workspace_purge, trial_expired, and tenant_suspended 1-1 instead of collapsing them into `manual`"
#[test]
fn connection_to_inbox_disconnect_reason_maps_workspace_purge_trial_expired_and_tenant_suspended_1_1_instead_of_collapsing_them_into_manual()
 {
    assert_eq!(
        connection_to_inbox_disconnect_reason(ConnectionStatusReason::WorkspacePurge).as_str(),
        "workspace_purge"
    );
    assert_eq!(
        connection_to_inbox_disconnect_reason(ConnectionStatusReason::TrialExpired).as_str(),
        "trial_expired"
    );
    assert_eq!(
        connection_to_inbox_disconnect_reason(ConnectionStatusReason::TenantSuspended).as_str(),
        "tenant_suspended"
    );
}

/// Port of: packages/utils/__tests__/connection.test.ts:51 "connection status partitions > partition every connection status without overlap"
#[test]
fn connection_status_partitions_partition_every_connection_status_without_overlap() {
    assert!(
        !ACTIVE_CONNECTION_STATUSES
            .iter()
            .any(|s| INACTIVE_CONNECTION_STATUSES.contains(s))
    );
    let mut both: Vec<ConnectionStatus> = ACTIVE_CONNECTION_STATUSES
        .iter()
        .chain(INACTIVE_CONNECTION_STATUSES)
        .copied()
        .collect();
    let mut all = ConnectionStatus::ALL.to_vec();
    both.sort_by_key(|s| s.as_str());
    all.sort_by_key(|s| s.as_str());
    assert_eq!(both, all);
}

/// Port of: packages/utils/__tests__/connection.test.ts:62 "connection status partitions > partition every connect session status without overlap"
#[test]
fn connection_status_partitions_partition_every_connect_session_status_without_overlap() {
    assert!(
        !ACTIVE_CONNECT_SESSION_STATUSES
            .iter()
            .any(|s| TERMINAL_CONNECT_SESSION_STATUSES.contains(s))
    );
    let mut both: Vec<ConnectSessionStatus> = ACTIVE_CONNECT_SESSION_STATUSES
        .iter()
        .chain(TERMINAL_CONNECT_SESSION_STATUSES)
        .copied()
        .collect();
    let mut all = ConnectSessionStatus::ALL.to_vec();
    both.sort_by_key(|s| s.as_str());
    all.sort_by_key(|s| s.as_str());
    assert_eq!(both, all);
}

/// Port of: packages/utils/__tests__/connection.test.ts:78 "connectSessionNextActionSchema > rejects an unknown next-action type"
#[test]
fn connectsessionnextactionschema_rejects_an_unknown_next_action_type() {
    assert!(parse_connect_session_next_action(&json!({ "type": "redirect" })).is_err());
}

/// Port of: packages/utils/__tests__/connection.test.ts:86 "appendConnectError > adds the code to a relative return URL and keeps its query and hash"
#[test]
fn appendconnecterror_adds_the_code_to_a_relative_return_url_and_keeps_its_query_and_hash() {
    assert_eq!(
        append_connect_error("/space/1/settings?session=9#top", "internal_error"),
        "/space/1/settings?session=9&connect_error=internal_error#top"
    );
}

/// Port of: packages/utils/__tests__/connection.test.ts:92 "appendConnectError > replaces an existing connect_error value"
#[test]
fn appendconnecterror_replaces_an_existing_connect_error_value() {
    assert_eq!(
        append_connect_error("/a?connect_error=x&b=1", "provider_unavailable"),
        "/a?connect_error=provider_unavailable&b=1"
    );
}

/// Port of: packages/utils/__tests__/connection.test.ts:98 "appendConnectError > keeps an absolute return URL absolute"
#[test]
fn appendconnecterror_keeps_an_absolute_return_url_absolute() {
    assert_eq!(
        append_connect_error("https://app.example.com/a?b=1", "internal_error"),
        "https://app.example.com/a?b=1&connect_error=internal_error"
    );
}

/// Port of: packages/utils/__tests__/connection.test.ts:104 "appendConnectError > never emits a protocol-relative or backslash redirect for /..//evil.example"
#[test]
fn appendconnecterror_never_emits_a_protocol_relative_or_backslash_redirect_for_evil_example() {
    let out = append_connect_error("/..//evil.example", "internal_error");

    assert!(out.starts_with('/'));
    assert!(!out.starts_with("//"));
    assert!(!out.contains('\\'));
    assert!(out.contains("connect_error=internal_error"));
}

/// Port of: packages/utils/__tests__/connection.test.ts:104 "appendConnectError > never emits a protocol-relative or backslash redirect for /.//evil.example/x"
#[test]
fn appendconnecterror_never_emits_a_protocol_relative_or_backslash_redirect_for_evil_example_x() {
    let out = append_connect_error("/.//evil.example/x", "internal_error");

    assert!(out.starts_with('/'));
    assert!(!out.starts_with("//"));
    assert!(!out.contains('\\'));
    assert!(out.contains("connect_error=internal_error"));
}

/// Port of: packages/utils/__tests__/connection.test.ts:104 "appendConnectError > never emits a protocol-relative or backslash redirect for /%2e%2e//evil.example"
#[test]
fn appendconnecterror_never_emits_a_protocol_relative_or_backslash_redirect_for_2e_2e_evil_example()
{
    let out = append_connect_error("/%2e%2e//evil.example", "internal_error");

    assert!(out.starts_with('/'));
    assert!(!out.starts_with("//"));
    assert!(!out.contains('\\'));
    assert!(out.contains("connect_error=internal_error"));
}

/// Port of: packages/utils/__tests__/connection.test.ts:104 "appendConnectError > never emits a protocol-relative or backslash redirect for //evil.example"
#[test]
fn appendconnecterror_never_emits_a_protocol_relative_or_backslash_redirect_for_evil_example_2() {
    let out = append_connect_error("//evil.example", "internal_error");

    assert!(out.starts_with('/'));
    assert!(!out.starts_with("//"));
    assert!(!out.contains('\\'));
    assert!(out.contains("connect_error=internal_error"));
}

/// Port of: packages/utils/__tests__/connection.test.ts:104 "appendConnectError > never emits a protocol-relative or backslash redirect for /\evil.example"
#[test]
fn appendconnecterror_never_emits_a_protocol_relative_or_backslash_redirect_for_evil_example_3() {
    let out = append_connect_error("/\\evil.example", "internal_error");

    assert!(out.starts_with('/'));
    assert!(!out.starts_with("//"));
    assert!(!out.contains('\\'));
    assert!(out.contains("connect_error=internal_error"));
}

/// Port of: packages/utils/__tests__/connection.test.ts:121 "connectFailureCauseOf > reads an allow-listed cause from failureCause or data.cause"
#[test]
fn connectfailurecauseof_reads_an_allow_listed_cause_from_failurecause_or_data_cause() {
    assert_eq!(
        connect_failure_cause_of(&json!({ "failureCause": "permission_denied" })),
        Some(ConnectFailureCause::PermissionDenied)
    );
    assert_eq!(
        connect_failure_cause_of(&json!({ "failureCause": "project_not_approved" })),
        Some(ConnectFailureCause::ProjectNotApproved)
    );
    assert_eq!(
        connect_failure_cause_of(&json!({ "data": { "cause": "api_not_enabled" } })),
        Some(ConnectFailureCause::ApiNotEnabled)
    );
}

/// Port of: packages/utils/__tests__/connection.test.ts:133 "connectFailureCauseOf > ignores anything else (0)"
#[test]
fn connectfailurecauseof_ignores_anything_else_0() {
    assert_eq!(connect_failure_cause_of(&Value::Null), None);
}

/// Port of: packages/utils/__tests__/connection.test.ts:133 "connectFailureCauseOf > ignores anything else (1)"
#[test]
fn connectfailurecauseof_ignores_anything_else_1() {
    assert_eq!(connect_failure_cause_of(&Value::Null), None);
}

/// Port of: packages/utils/__tests__/connection.test.ts:133 "connectFailureCauseOf > ignores anything else (2)"
#[test]
fn connectfailurecauseof_ignores_anything_else_2() {
    assert_eq!(connect_failure_cause_of(&json!("x")), None);
}

/// Port of: packages/utils/__tests__/connection.test.ts:133 "connectFailureCauseOf > ignores anything else (3)"
#[test]
fn connectfailurecauseof_ignores_anything_else_3() {
    assert_eq!(
        connect_failure_cause_of(&json!({ "name": "Error", "message": "boom" })),
        None
    );
}

/// Port of: packages/utils/__tests__/connection.test.ts:133 "connectFailureCauseOf > ignores anything else (4)"
#[test]
fn connectfailurecauseof_ignores_anything_else_4() {
    assert_eq!(
        connect_failure_cause_of(&json!({ "failureCause": "<script>" })),
        None
    );
}

/// Port of: packages/utils/__tests__/connection.test.ts:133 "connectFailureCauseOf > ignores anything else (5)"
#[test]
fn connectfailurecauseof_ignores_anything_else_5() {
    assert_eq!(
        connect_failure_cause_of(&json!({ "data": { "cause": "internal_error" } })),
        None
    );
}

/// Port of: packages/utils/__tests__/connection.test.ts:133 "connectFailureCauseOf > ignores anything else (6)"
#[test]
fn connectfailurecauseof_ignores_anything_else_6() {
    assert_eq!(connect_failure_cause_of(&json!({ "data": null })), None);
}

/// Port of: packages/utils/__tests__/connection.test.ts:147 "zodToConfigFields > preserves descriptions, optional defaults, patterns, and bigint-string format"
#[test]
fn zodtoconfigfields_preserves_descriptions_optional_defaults_patterns_and_bigint_string_format() {
    // z.toJSONSchema(z.object({ name, brandColor, flowId })).
    let schema = JsonSchema {
        kind: Some("object".to_owned()),
        properties: Some(vec![
            (
                "name".to_owned(),
                JsonSchema {
                    description: Some("Display name.".to_owned()),
                    ..JsonSchema::of_type("string")
                },
            ),
            (
                "brandColor".to_owned(),
                JsonSchema {
                    default: Some(json!("#007bff")),
                    description: Some("Brand color.".to_owned()),
                    pattern: Some("^#[0-9A-Fa-f]{6}$".to_owned()),
                    ..JsonSchema::of_type("string")
                },
            ),
            (
                "flowId".to_owned(),
                JsonSchema {
                    description: Some("Flow ID.".to_owned()),
                    pattern: Some("^\\d+$".to_owned()),
                    ..JsonSchema::of_type("string")
                },
            ),
        ]),
        required: Some(vec![
            "name".to_owned(),
            "brandColor".to_owned(),
            "flowId".to_owned(),
        ]),
        ..JsonSchema::default()
    };

    let fields = zod_to_config_fields(&schema).unwrap();

    assert_eq!(
        fields,
        vec![
            ConnectionConfigField {
                name: "name".to_owned(),
                kind: "string".to_owned(),
                required: true,
                description: Some("Display name.".to_owned()),
                ..ConnectionConfigField::default()
            },
            ConnectionConfigField {
                name: "brandColor".to_owned(),
                kind: "string".to_owned(),
                required: false,
                pattern: Some("^#[0-9A-Fa-f]{6}$".to_owned()),
                description: Some("Brand color.".to_owned()),
                ..ConnectionConfigField::default()
            },
            ConnectionConfigField {
                name: "flowId".to_owned(),
                kind: "string".to_owned(),
                required: true,
                format: Some("bigint-string".to_owned()),
                pattern: Some("^\\d+$".to_owned()),
                description: Some("Flow ID.".to_owned()),
                ..ConnectionConfigField::default()
            },
        ]
    );
}
