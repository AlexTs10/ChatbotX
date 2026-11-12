//! Port of `packages/variables/__tests__/last-call.test.ts`.

mod support;

use chatbotx_core::variables::*;
use support::variables::*;

/// Port of: packages/variables/__tests__/last-call.test.ts:35 "last call helpers > getContactLastCallRecording > returns a presigned recording URL, never the public storage URL"
#[tokio::test]
async fn last_call_helpers_getcontactlastcallrecording_returns_a_presigned_recording_url_never_the_public_storage_url()
 {
    let h = Harness::new(FakeState {
        recorded_call: Some(CallRecord {
            recording_path: s("space/ws-1/calls/call-1.ogg"),
            ..Default::default()
        }),
        signed_url: Ok("https://storage.example.com/space/ws-1/calls/call-1.ogg?X-Signed=1".into()),
        ..Default::default()
    });
    let result = h
        .v()
        .get_contact_last_call_recording("contact-1")
        .await
        .unwrap();
    assert!(
        h.fake
            .called("recording_signed_url(space/ws-1/calls/call-1.ogg)")
    );
    assert_eq!(
        result,
        s("https://storage.example.com/space/ws-1/calls/call-1.ogg?X-Signed=1")
    );
    assert_ne!(result, s("space/ws-1/calls/call-1.ogg"));
}

/// Port of: packages/variables/__tests__/last-call.test.ts:55 "last call helpers > getContactLastCallRecording > a presign failure degrades to null instead of aborting the whole render"
#[tokio::test]
async fn last_call_helpers_getcontactlastcallrecording_a_presign_failure_degrades_to_null_instead_of_aborting_the_whole_render()
 {
    let h = Harness::new(FakeState {
        recorded_call: Some(CallRecord {
            id: "call-1".into(),
            recording_path: s("space/ws-1/calls/call-1.ogg"),
            transcript: None,
        }),
        signed_url: Err("storage down".into()),
        ..Default::default()
    });
    assert_eq!(
        h.v()
            .get_contact_last_call_recording("contact-1")
            .await
            .unwrap(),
        None
    );
}

/// Port of: packages/variables/__tests__/last-call.test.ts:66 "last call helpers > getContactLastCallRecording > returns null when the contact has no recorded call"
#[tokio::test]
async fn last_call_helpers_getcontactlastcallrecording_returns_null_when_the_contact_has_no_recorded_call()
 {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        h.v()
            .get_contact_last_call_recording("contact-1")
            .await
            .unwrap(),
        None
    );
    assert_eq!(h.fake.count("recording_signed_url"), 0);
}

/// Port of: packages/variables/__tests__/last-call.test.ts:75 "last call helpers > getContactLastCallRecording > returns null when the call has no recording path"
#[tokio::test]
async fn last_call_helpers_getcontactlastcallrecording_returns_null_when_the_call_has_no_recording_path()
 {
    let h = Harness::new(FakeState {
        recorded_call: Some(CallRecord {
            recording_path: None,
            ..Default::default()
        }),
        ..Default::default()
    });
    assert_eq!(
        h.v()
            .get_contact_last_call_recording("contact-1")
            .await
            .unwrap(),
        None
    );
    assert_eq!(h.fake.count("recording_signed_url"), 0);
}

/// Port of: packages/variables/__tests__/last-call.test.ts:89 "last call helpers > getContactLastCallTranscript > returns the transcript text of the latest transcribed call"
#[tokio::test]
async fn last_call_helpers_getcontactlastcalltranscript_returns_the_transcript_text_of_the_latest_transcribed_call()
 {
    let h = Harness::new(FakeState {
        transcribed_call: Some(CallRecord {
            transcript: s("hello from the call"),
            ..Default::default()
        }),
        ..Default::default()
    });
    assert_eq!(
        h.v()
            .get_contact_last_call_transcript("contact-1")
            .await
            .unwrap(),
        s("hello from the call")
    );
}

/// Port of: packages/variables/__tests__/last-call.test.ts:99 "last call helpers > getContactLastCallTranscript > returns null when there is no transcribed call"
#[tokio::test]
async fn last_call_helpers_getcontactlastcalltranscript_returns_null_when_there_is_no_transcribed_call()
 {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        h.v()
            .get_contact_last_call_transcript("contact-1")
            .await
            .unwrap(),
        None
    );
}
