//! Port of `packages/encryption/__tests__/user-data-webview-token.test.ts`.

mod support;

use chatbotx_core::encryption::{
    APPOINTMENT_WEBVIEW_TOKEN_TTL_MS, AppointmentTokenSource, AppointmentWebviewClaims,
    AppointmentWebviewMode, USER_DATA_WEBVIEW_TOKEN_TTL_MS, UserDataReplyFormat,
    UserDataWebviewClaims,
};
use regex::Regex;

fn base_payload() -> UserDataWebviewClaims {
    UserDataWebviewClaims {
        workspace_id: "workspace-1".to_owned(),
        conversation_id: "conversation-1".to_owned(),
        contact_inbox_id: "contact-inbox-1".to_owned(),
        contact_id: "contact-1".to_owned(),
        channel: "messenger".to_owned(),
        flow_id: "flow-1".to_owned(),
        flow_version_id: Some("flow-version-1".to_owned()),
        step_id: "step-1".to_owned(),
        node_id: "node-1".to_owned(),
        challenge_id: "challenge-1".to_owned(),
        output_field_id: "field-1".to_owned(),
        reply_format: UserDataReplyFormat::Date,
    }
}

/// Port of: packages/encryption/__tests__/user-data-webview-token.test.ts:31 "user data webview token > round-trips a signed token"
#[test]
fn user_data_webview_token_round_trips_a_signed_token() {
    let enc = support::encryption();
    let token = enc
        .sign_user_data_webview_token(&base_payload(), USER_DATA_WEBVIEW_TOKEN_TTL_MS)
        .unwrap();

    assert!(Regex::new(r"^[A-Za-z0-9\-_]+$").unwrap().is_match(&token));
    assert_eq!(
        enc.verify_user_data_webview_token(&token).unwrap().claims,
        base_payload()
    );
}

/// Port of: packages/encryption/__tests__/user-data-webview-token.test.ts:40 "user data webview token > round-trips without the optional flowVersionId"
#[test]
fn user_data_webview_token_round_trips_without_the_optional_flowversionid() {
    let enc = support::encryption();
    let payload = UserDataWebviewClaims {
        flow_version_id: None,
        ..base_payload()
    };
    let token = enc
        .sign_user_data_webview_token(&payload, USER_DATA_WEBVIEW_TOKEN_TTL_MS)
        .unwrap();

    assert_eq!(
        enc.verify_user_data_webview_token(&token).unwrap().claims,
        payload
    );
}

/// Port of: packages/encryption/__tests__/user-data-webview-token.test.ts:49 "user data webview token > round-trips the datetime replyFormat"
#[test]
fn user_data_webview_token_round_trips_the_datetime_replyformat() {
    let enc = support::encryption();
    let token = enc
        .sign_user_data_webview_token(
            &UserDataWebviewClaims {
                reply_format: UserDataReplyFormat::Datetime,
                ..base_payload()
            },
            USER_DATA_WEBVIEW_TOKEN_TTL_MS,
        )
        .unwrap();

    assert_eq!(
        enc.verify_user_data_webview_token(&token)
            .unwrap()
            .claims
            .reply_format,
        UserDataReplyFormat::Datetime
    );
}

/// Port of: packages/encryption/__tests__/user-data-webview-token.test.ts:60 "user data webview token > rejects expired tokens"
#[test]
fn user_data_webview_token_rejects_expired_tokens() {
    let enc = support::encryption();
    let token = enc
        .sign_user_data_webview_token(&base_payload(), -1)
        .unwrap();
    let err = enc.verify_user_data_webview_token(&token).unwrap_err();
    assert!(err.to_string().contains("expired"), "{err}");
}

/// Port of: packages/encryption/__tests__/user-data-webview-token.test.ts:66 "user data webview token > rejects malformed tokens"
#[test]
fn user_data_webview_token_rejects_malformed_tokens() {
    let enc = support::encryption();
    assert!(enc.verify_user_data_webview_token("not-a-token").is_err());
}

/// Port of: packages/encryption/__tests__/user-data-webview-token.test.ts:70 "user data webview token > rejects tokens signed for a different AAD (type mismatch)"
#[test]
fn user_data_webview_token_rejects_tokens_signed_for_a_different_aad_type_mismatch() {
    let enc = support::encryption();
    let appointment_token = enc
        .sign_appointment_webview_token(
            &AppointmentWebviewClaims {
                source: AppointmentTokenSource::Flow,
                mode: AppointmentWebviewMode::Book,
                workspace_id: "workspace-1".to_owned(),
                calendar_id: "calendar-1".to_owned(),
                contact_id: "contact-1".to_owned(),
                conversation_id: "conversation-1".to_owned(),
                contact_inbox_id: "contact-inbox-1".to_owned(),
                channel: Some("messenger".to_owned()),
                flow_id: Some("flow-1".to_owned()),
                flow_version_id: Some("flow-version-1".to_owned()),
                step_id: Some("step-1".to_owned()),
                ..AppointmentWebviewClaims::default()
            },
            APPOINTMENT_WEBVIEW_TOKEN_TTL_MS,
        )
        .unwrap();

    let err = enc
        .verify_user_data_webview_token(&appointment_token)
        .unwrap_err();
    assert!(err.to_string().contains("type mismatch"), "{err}");
}

/// Port of: packages/encryption/__tests__/user-data-webview-token.test.ts:90 "user data webview token > does not allow a user-data webview token to be verified as an appointment webview token"
#[test]
fn user_data_webview_token_does_not_allow_a_user_data_webview_token_to_be_verified_as_an_appointment_webview_token()
 {
    let enc = support::encryption();
    let token = enc
        .sign_user_data_webview_token(&base_payload(), USER_DATA_WEBVIEW_TOKEN_TTL_MS)
        .unwrap();

    let err = enc.verify_appointment_webview_token(&token).unwrap_err();
    assert!(err.to_string().contains("type mismatch"), "{err}");
}
