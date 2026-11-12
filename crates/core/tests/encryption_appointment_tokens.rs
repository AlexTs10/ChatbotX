//! Port of `packages/encryption/__tests__/appointment-tokens.test.ts`.
//! Tokens are signed with an `Encryption` on the real clock; TTL arguments
//! are explicit (the TS default TTL is the named constant).

mod support;

use chatbotx_core::encryption::{
    APPOINTMENT_INBOX_LINK_TTL_MS, APPOINTMENT_TOKEN_TTL_MS, APPOINTMENT_WEBVIEW_TOKEN_TTL_MS,
    AppointmentClaims, AppointmentTokenSource, AppointmentWebviewClaims, AppointmentWebviewMode,
    build_appointment_cancel_postback, parse_appointment_cancel_postback,
};
use regex::Regex;
use serde_json::json;

fn url_safe(token: &str) -> bool {
    Regex::new(r"^[A-Za-z0-9\-_]+$").unwrap().is_match(token)
}

fn s(value: &str) -> Option<String> {
    Some(value.to_owned())
}

fn flow_claims() -> AppointmentWebviewClaims {
    AppointmentWebviewClaims {
        source: AppointmentTokenSource::Flow,
        mode: AppointmentWebviewMode::Book,
        workspace_id: "workspace-1".to_owned(),
        calendar_id: "calendar-1".to_owned(),
        contact_id: "contact-1".to_owned(),
        conversation_id: "conversation-1".to_owned(),
        contact_inbox_id: "contact-inbox-1".to_owned(),
        channel: s("messenger"),
        flow_id: s("flow-1"),
        flow_version_id: s("flow-version-1"),
        step_id: s("step-1"),
        ..AppointmentWebviewClaims::default()
    }
}

fn inbox_claims() -> AppointmentWebviewClaims {
    AppointmentWebviewClaims {
        source: AppointmentTokenSource::Inbox,
        mode: AppointmentWebviewMode::Book,
        workspace_id: "workspace-1".to_owned(),
        calendar_id: "calendar-1".to_owned(),
        contact_id: "contact-1".to_owned(),
        conversation_id: "conversation-1".to_owned(),
        contact_inbox_id: "contact-inbox-1".to_owned(),
        ..AppointmentWebviewClaims::default()
    }
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:25 "appointment tokens > round-trips a webview token"
#[test]
fn appointment_tokens_round_trips_a_webview_token() {
    let enc = support::encryption();
    let token = enc
        .sign_appointment_webview_token(
            &AppointmentWebviewClaims {
                node_id: s("node-1"),
                selected_date_custom_field_id: s("field-1"),
                ..flow_claims()
            },
            APPOINTMENT_WEBVIEW_TOKEN_TTL_MS,
        )
        .unwrap();

    assert!(url_safe(&token));
    let payload = enc.verify_appointment_webview_token(&token).unwrap().claims;
    assert_eq!(payload.mode, AppointmentWebviewMode::Book);
    assert_eq!(payload.workspace_id, "workspace-1");
    assert_eq!(payload.calendar_id, "calendar-1");
    assert_eq!(payload.contact_id, "contact-1");
    assert_eq!(payload.conversation_id, "conversation-1");
    assert_eq!(payload.contact_inbox_id, "contact-inbox-1");
    assert_eq!(payload.channel, s("messenger"));
    assert_eq!(payload.flow_id, s("flow-1"));
    assert_eq!(payload.flow_version_id, s("flow-version-1"));
    assert_eq!(payload.step_id, s("step-1"));
    assert_eq!(payload.node_id, s("node-1"));
    assert_eq!(payload.selected_date_custom_field_id, s("field-1"));
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:59 "appointment tokens > round-trips an availability range webview token"
#[test]
fn appointment_tokens_round_trips_an_availability_range_webview_token() {
    let enc = support::encryption();
    let token = enc
        .sign_appointment_webview_token(
            &AppointmentWebviewClaims {
                mode: AppointmentWebviewMode::SelectAvailabilityRange,
                node_id: s("node-1"),
                start_date_custom_field_id: s("start-field"),
                end_date_custom_field_id: s("end-field"),
                result_custom_field_id: s("result-field"),
                result_used_by_ai: Some(true),
                ..flow_claims()
            },
            APPOINTMENT_WEBVIEW_TOKEN_TTL_MS,
        )
        .unwrap();

    assert!(url_safe(&token));
    let payload = enc.verify_appointment_webview_token(&token).unwrap().claims;
    assert_eq!(
        payload.mode,
        AppointmentWebviewMode::SelectAvailabilityRange
    );
    assert_eq!(payload.start_date_custom_field_id, s("start-field"));
    assert_eq!(payload.end_date_custom_field_id, s("end-field"));
    assert_eq!(payload.result_custom_field_id, s("result-field"));
    assert_eq!(payload.result_used_by_ai, Some(true));
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:89 "appointment tokens > rejects expired tokens"
#[test]
fn appointment_tokens_rejects_expired_tokens() {
    let enc = support::encryption();
    let token = enc
        .sign_appointment_webview_token(&flow_claims(), -1)
        .unwrap();

    let err = enc.verify_appointment_webview_token(&token).unwrap_err();
    assert!(err.to_string().contains("expired"), "{err}");
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:112 "appointment tokens > treats tokens signed before the source field as flow tokens"
#[test]
fn appointment_tokens_treats_tokens_signed_before_the_source_field_as_flow_tokens() {
    let enc = support::encryption();
    let token = enc
        .sign_appointment_token(
            &json!({
                "mode": "book",
                "workspaceId": "workspace-1",
                "calendarId": "calendar-1",
                "contactId": "contact-1",
                "conversationId": "conversation-1",
                "contactInboxId": "contact-inbox-1",
                "channel": "messenger",
                "flowId": "flow-1",
                "flowVersionId": "flow-version-1",
                "stepId": "step-1",
                "nodeId": "node-1",
                "expiresAt": support::now_ms() + 60_000,
            }),
            "appointment-webview-token",
        )
        .unwrap();

    let payload = enc.verify_appointment_webview_token(&token).unwrap().claims;
    assert_eq!(payload.source, AppointmentTokenSource::Flow);
    assert_eq!(payload.mode, AppointmentWebviewMode::Book);
    assert_eq!(payload.flow_id, s("flow-1"));
    assert_eq!(payload.node_id, s("node-1"));
}

fn legacy_flow_token_without_source(mode: &str, expected: AppointmentWebviewMode) {
    let enc = support::encryption();
    let token = enc
        .sign_appointment_token(
            &json!({
                "mode": mode,
                "workspaceId": "workspace-1",
                "calendarId": "calendar-1",
                "contactId": "contact-1",
                "conversationId": "conversation-1",
                "contactInboxId": "contact-inbox-1",
                "channel": "messenger",
                "flowId": "flow-1",
                "flowVersionId": "flow-version-1",
                "stepId": "step-1",
                "nodeId": "node-1",
                "expiresAt": support::now_ms() + 60_000,
            }),
            "appointment-webview-token",
        )
        .unwrap();

    let payload = enc.verify_appointment_webview_token(&token).unwrap().claims;
    assert_eq!(payload.source, AppointmentTokenSource::Flow);
    assert_eq!(payload.mode, expected);
    assert_eq!(payload.step_id, s("step-1"));
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:139 "appointment tokens > keeps verifying legacy selectAvailability flow tokens without source"
#[test]
fn appointment_tokens_keeps_verifying_legacy_selectavailability_flow_tokens_without_source() {
    legacy_flow_token_without_source(
        "selectAvailability",
        AppointmentWebviewMode::SelectAvailability,
    );
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:139 "appointment tokens > keeps verifying legacy selectAvailabilityRange flow tokens without source"
#[test]
fn appointment_tokens_keeps_verifying_legacy_selectavailabilityrange_flow_tokens_without_source() {
    legacy_flow_token_without_source(
        "selectAvailabilityRange",
        AppointmentWebviewMode::SelectAvailabilityRange,
    );
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:168 "appointment tokens > round-trips an inbox token without flow fields, channel, or mode"
#[test]
fn appointment_tokens_round_trips_an_inbox_token_without_flow_fields_channel_or_mode() {
    let enc = support::encryption();
    let token = enc
        .sign_appointment_token(
            &json!({
                "source": "inbox",
                "workspaceId": "workspace-1",
                "calendarId": "calendar-1",
                "contactId": "contact-1",
                "conversationId": "conversation-1",
                "contactInboxId": "contact-inbox-1",
                "expiresAt": support::now_ms() + 60_000,
            }),
            "appointment-webview-token",
        )
        .unwrap();

    let payload = enc.verify_appointment_webview_token(&token).unwrap().claims;
    assert_eq!(payload.source, AppointmentTokenSource::Inbox);
    assert_eq!(payload.mode, AppointmentWebviewMode::Book);
    assert_eq!(payload.contact_inbox_id, "contact-inbox-1");
    assert_eq!(payload.channel, None);
    assert_eq!(payload.flow_id, None);
    let serialized = serde_json::to_value(&payload).unwrap();
    assert!(serialized.get("channel").is_none());
    assert!(serialized.get("flowId").is_none());
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:192 "appointment tokens > rejects an inbox token for the availability range mode"
#[test]
fn appointment_tokens_rejects_an_inbox_token_for_the_availability_range_mode() {
    let enc = support::encryption();
    let token = enc
        .sign_appointment_webview_token(
            &AppointmentWebviewClaims {
                mode: AppointmentWebviewMode::SelectAvailabilityRange,
                ..inbox_claims()
            },
            APPOINTMENT_WEBVIEW_TOKEN_TTL_MS,
        )
        .unwrap();

    assert!(enc.verify_appointment_webview_token(&token).is_err());
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:206 "appointment tokens > rejects an inbox token that carries a flow node"
#[test]
fn appointment_tokens_rejects_an_inbox_token_that_carries_a_flow_node() {
    let enc = support::encryption();
    let token = enc
        .sign_appointment_webview_token(
            &AppointmentWebviewClaims {
                node_id: s("node-1"),
                ..inbox_claims()
            },
            APPOINTMENT_WEBVIEW_TOKEN_TTL_MS,
        )
        .unwrap();

    assert!(enc.verify_appointment_webview_token(&token).is_err());
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:221 "appointment tokens > rejects a flow token without stepId"
#[test]
fn appointment_tokens_rejects_a_flow_token_without_stepid() {
    let enc = support::encryption();
    let token = enc
        .sign_appointment_webview_token(
            &AppointmentWebviewClaims {
                step_id: None,
                ..flow_claims()
            },
            APPOINTMENT_WEBVIEW_TOKEN_TTL_MS,
        )
        .unwrap();

    assert!(enc.verify_appointment_webview_token(&token).is_err());
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:221 "appointment tokens > rejects a flow token without channel"
#[test]
fn appointment_tokens_rejects_a_flow_token_without_channel() {
    let enc = support::encryption();
    let token = enc
        .sign_appointment_webview_token(
            &AppointmentWebviewClaims {
                channel: None,
                ..flow_claims()
            },
            APPOINTMENT_WEBVIEW_TOKEN_TTL_MS,
        )
        .unwrap();

    assert!(enc.verify_appointment_webview_token(&token).is_err());
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:244 "appointment tokens > inbox booking links expire after 24 hours"
#[test]
fn appointment_tokens_inbox_booking_links_expire_after_24_hours() {
    assert_eq!(APPOINTMENT_INBOX_LINK_TTL_MS, 24 * 60 * 60 * 1000);
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:248 "appointment tokens > returns the expiry that is written into the signed token"
#[test]
fn appointment_tokens_returns_the_expiry_that_is_written_into_the_signed_token() {
    let enc = support::encryption();
    let (token, expires_at) = enc
        .sign_appointment_webview_token_with_expiry(&inbox_claims(), APPOINTMENT_INBOX_LINK_TTL_MS)
        .unwrap();

    let payload = enc.verify_appointment_webview_token(&token).unwrap();
    assert_eq!(expires_at, payload.expires_at);
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:266 "appointment tokens > does not allow schedule tokens to be used as cancel tokens"
#[test]
fn appointment_tokens_does_not_allow_schedule_tokens_to_be_used_as_cancel_tokens() {
    let enc = support::encryption();
    let token = enc
        .sign_appointment_schedule_token(
            &AppointmentClaims {
                appointment_id: "appointment-1".to_owned(),
                workspace_id: "workspace-1".to_owned(),
                contact_id: "contact-1".to_owned(),
                ..AppointmentClaims::default()
            },
            APPOINTMENT_TOKEN_TTL_MS,
        )
        .unwrap();

    let err = enc.verify_appointment_cancel_token(&token).unwrap_err();
    assert!(err.to_string().contains("type mismatch"), "{err}");
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:278 "appointment tokens > round-trips a cancel token"
#[test]
fn appointment_tokens_round_trips_a_cancel_token() {
    let enc = support::encryption();
    let claims = AppointmentClaims {
        appointment_id: "appointment-1".to_owned(),
        workspace_id: "workspace-1".to_owned(),
        contact_id: "contact-1".to_owned(),
        conversation_id: s("conversation-1"),
        contact_inbox_id: s("contact-inbox-1"),
        flow_version_id: s("flow-version-1"),
    };
    let token = enc
        .sign_appointment_cancel_token(&claims, APPOINTMENT_TOKEN_TTL_MS)
        .unwrap();

    assert_eq!(
        enc.verify_appointment_cancel_token(&token).unwrap().claims,
        claims
    );
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:298 "appointment tokens > wraps and parses appointment cancel postbacks"
#[test]
fn appointment_tokens_wraps_and_parses_appointment_cancel_postbacks() {
    let postback = build_appointment_cancel_postback("cancel-token");

    assert_eq!(postback, "appointment_cancel:cancel-token");
    assert_eq!(
        parse_appointment_cancel_postback(Some(&postback)).as_deref(),
        Some("cancel-token")
    );
    assert_eq!(
        parse_appointment_cancel_postback(Some("flow-payload")),
        None
    );
}

/// Port of: packages/encryption/__tests__/appointment-tokens.test.ts:306 "appointment tokens > rejects malformed tokens"
#[test]
fn appointment_tokens_rejects_malformed_tokens() {
    let enc = support::encryption();
    assert!(
        enc.verify_appointment_schedule_token("not-a-token")
            .is_err()
    );
}
