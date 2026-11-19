//! Port of `apps/builder/__tests__/whatsapp-call-announcement-options.test.ts`
//! and `whatsapp-call-announcement-validation-error.test.ts`
//! (`buildCallAnnouncementOptions`, `isCallAnnouncementValidationError`).
//! The integration is a migrated in-memory row (the builder builds a plain object).
use chatbotx_business::whatsapp_voip::{
    DEFAULT_CALL_ANNOUNCEMENT_PURPOSE, GraphError, build_announcements,
    is_announcement_validation_error,
};
use chatbotx_db::Db;
use chatbotx_db::repo::integration_whatsapp;
use chatbotx_db::repo::models::IntegrationWhatsapp;
use chatbotx_db::rusqlite::params;
use serde_json::{Value, json};

/// `(recording, recordingMode, transcription, transcriptionMode, language, purpose)`.
async fn integration(
    recording: bool,
    recording_mode: &str,
    transcription: bool,
    transcription_mode: &str,
    language: Option<&str>,
    purpose: Option<&str>,
) -> IntegrationWhatsapp {
    let db = Db::open_in_memory().unwrap();
    db.migrate().await.unwrap();
    let (rm, tm) = (recording_mode.to_owned(), transcription_mode.to_owned());
    let (language, purpose) = (language.map(str::to_owned), purpose.map(str::to_owned));
    db.write(move |tx| {
        tx.execute(r#"INSERT INTO "User" ("id", "email") VALUES (1, 'a@x.test')"#, [])?;
        tx.execute(r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (7, 'ws', 1)"#, [])?;
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (8, 7, 'Wa', 'whatsapp', 'wa-8')"#,
            [],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId",
                 "callRecordingEnabled", "callRecordingMode", "callTranscriptionEnabled", "callTranscriptionMode",
                 "callAnnouncementLanguage", "callRecordingPurpose")
               VALUES (9, '{}', 'p', 'w', 'b', 'Wa', 7, 8, ?1, ?2, ?3, ?4, ?5, ?6)"#,
            params![recording, rm, transcription, tm, language, purpose],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    db.read(|c| {
        Ok(integration_whatsapp::find_by_id_for_workspace(c, 9, 7)
            .unwrap()
            .unwrap())
    })
    .await
    .unwrap()
}

fn entry(purpose: &str, language: &str) -> Option<(String, String)> {
    Some((purpose.to_owned(), language.to_owned()))
}

#[tokio::test]
async fn returns_an_empty_object_when_both_toggles_are_off() {
    let i = integration(false, "metaNative", false, "metaNative", None, None).await;
    assert!(build_announcements(&i, Some("en")).is_empty());
}

#[tokio::test]
async fn omits_recording_when_the_mode_is_browser_whisper() {
    let i = integration(true, "browserWhisper", false, "metaNative", None, None).await;
    assert!(build_announcements(&i, None).recording.is_none());
}

#[tokio::test]
async fn attaches_only_recording_when_enabled_meta_native_and_transcription_off() {
    let i = integration(
        true,
        "metaNative",
        false,
        "metaNative",
        None,
        Some("Support"),
    )
    .await;
    let a = build_announcements(&i, None);
    assert_eq!(a.recording, entry("Support", "en_US"));
    assert!(a.transcription.is_none());
}

#[tokio::test]
async fn omits_transcription_when_recording_is_off() {
    let i = integration(false, "metaNative", true, "metaNative", None, None).await;
    assert!(build_announcements(&i, None).is_empty());
}

#[tokio::test]
async fn attaches_only_transcription_when_recording_is_browser_whisper() {
    let i = integration(true, "browserWhisper", true, "metaNative", None, None).await;
    let a = build_announcements(&i, None);
    assert!(a.recording.is_none());
    assert_eq!(
        a.transcription,
        entry(DEFAULT_CALL_ANNOUNCEMENT_PURPOSE, "en_US")
    );
}

#[tokio::test]
async fn attaches_both_with_one_shared_purpose_and_language() {
    let i = integration(
        true,
        "metaNative",
        true,
        "metaNative",
        Some("de"),
        Some("QA"),
    )
    .await;
    let a = build_announcements(&i, Some("fr"));
    assert_eq!(a.recording, entry("QA", "de"));
    assert_eq!(a.transcription, a.recording);
}

#[tokio::test]
async fn falls_back_to_the_contact_locale_then_en_us() {
    let i = integration(true, "metaNative", false, "metaNative", None, None).await;
    assert_eq!(
        build_announcements(&i, Some("pt-BR")).recording,
        entry(DEFAULT_CALL_ANNOUNCEMENT_PURPOSE, "pt")
    );
    assert_eq!(
        build_announcements(&i, Some("xx")).recording,
        entry(DEFAULT_CALL_ANNOUNCEMENT_PURPOSE, "en_US")
    );
}

fn graph(status: u16, code: Value) -> GraphError {
    GraphError {
        status,
        code,
        message: "x".into(),
        user_title: None,
        user_message: None,
    }
}

#[test]
fn is_true_for_a_meta_4xx_and_false_for_a_5xx() {
    assert!(is_announcement_validation_error(&graph(400, json!(100))));
    assert!(!is_announcement_validation_error(&graph(500, json!(1))));
}

#[test]
fn matches_a_documented_code_delivered_as_a_string() {
    assert!(!is_announcement_validation_error(&graph(
        400,
        json!("138006")
    )));
    assert!(!is_announcement_validation_error(&graph(
        400,
        json!(138_006)
    )));
}

#[test]
fn never_retries_the_local_purpose_too_long_error() {
    assert!(!is_announcement_validation_error(&graph(
        400,
        json!("whatsappCallAnnouncementPurposeTooLong")
    )));
}

#[test]
fn is_false_for_auth_and_rate_limit_statuses() {
    for status in [401, 403, 429] {
        assert!(!is_announcement_validation_error(&graph(
            status,
            Value::Null
        )));
    }
}
