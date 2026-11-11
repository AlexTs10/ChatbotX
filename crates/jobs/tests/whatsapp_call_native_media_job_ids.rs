//! Port of `packages/worker-config/__tests__/whatsapp-call-native-media-job-ids.test.ts`.

mod support;

use chatbotx_jobs::queues::whatsapp_call::{
    NativeRecordingFetchData, NativeTranscriptFetchData, WhatsappCallJobAction,
    WhatsappCallJobData, whatsapp_call_native_recording_fetch_job_id,
    whatsapp_call_native_transcript_fetch_job_id, whatsapp_call_recording_ready_job_id,
};

/// Port of: packages/worker-config/__tests__/whatsapp-call-native-media-job-ids.test.ts:11 "IntegrationJobAction native media fetch actions > registers the two new job actions"
#[test]
fn registers_the_two_new_job_actions() {
    assert_eq!(
        WhatsappCallJobAction::NativeRecordingFetch.as_str(),
        "whatsappCallNativeRecordingFetch"
    );
    assert_eq!(
        WhatsappCallJobAction::NativeTranscriptFetch.as_str(),
        "whatsappCallNativeTranscriptFetch"
    );
}

/// Port of: packages/worker-config/__tests__/whatsapp-call-native-media-job-ids.test.ts:22 "whatsappCallNativeRecordingFetch payload shape > carries only slim media references, never sdp or audio bytes"
#[test]
fn carries_only_slim_media_references_never_sdp_or_audio_bytes() {
    let payload = WhatsappCallJobData::NativeRecordingFetch(NativeRecordingFetchData {
        whatsapp_call_id: "call-1".to_owned(),
        wacid: "wacid-1".to_owned(),
        workspace_id: "ws-1".to_owned(),
        audio_media_id: "media-1".to_owned(),
        audio_url: "https://graph.facebook.com/v1/media-1".to_owned(),
        mime_type: "audio/ogg; codecs=opus".to_owned(),
    });

    let value = payload.to_value();
    assert_eq!(value["type"], "whatsappCallNativeRecordingFetch");
    let data = &value["data"];
    let keys = support::sorted_keys(data);
    for absent in ["sdp", "audio", "bytes", "buffer"] {
        assert!(!keys.iter().any(|k| k == absent), "{absent} present");
    }
    let mut expected = vec![
        "audioMediaId",
        "audioUrl",
        "mimeType",
        "wacid",
        "whatsappCallId",
        "workspaceId",
    ];
    expected.sort();
    assert_eq!(keys, expected);
}

/// Port of: packages/worker-config/__tests__/whatsapp-call-native-media-job-ids.test.ts:53 "whatsappCallNativeTranscriptFetch payload shape > carries only slim media references, never inlined transcript segments"
#[test]
fn carries_only_slim_media_references_never_inlined_transcript() {
    let payload = WhatsappCallJobData::NativeTranscriptFetch(NativeTranscriptFetchData {
        whatsapp_call_id: "call-1".to_owned(),
        wacid: "wacid-1".to_owned(),
        workspace_id: "ws-1".to_owned(),
        document_media_id: "doc-1".to_owned(),
        document_url: "https://graph.facebook.com/v1/doc-1".to_owned(),
    });

    let value = payload.to_value();
    assert_eq!(value["type"], "whatsappCallNativeTranscriptFetch");
    let keys = support::sorted_keys(&value["data"]);
    for absent in ["segments", "transcript", "sdp"] {
        assert!(!keys.iter().any(|k| k == absent), "{absent} present");
    }
    let mut expected = vec![
        "documentMediaId",
        "documentUrl",
        "wacid",
        "whatsappCallId",
        "workspaceId",
    ];
    expected.sort();
    assert_eq!(keys, expected);
}

/// Port of: packages/worker-config/__tests__/whatsapp-call-native-media-job-ids.test.ts:81 "native media fetch job id helpers > whatsappCallNativeRecordingFetchJobId is stable and keyed by wacid"
#[test]
fn whatsappcallnativerecordingfetchjobid_is_stable_and_keyed_by() {
    assert_eq!(
        whatsapp_call_native_recording_fetch_job_id("wacid-1"),
        "native-rec-fetch-wacid-1"
    );
    assert_eq!(
        whatsapp_call_native_recording_fetch_job_id("wacid-1"),
        whatsapp_call_native_recording_fetch_job_id("wacid-1")
    );
    assert_ne!(
        whatsapp_call_native_recording_fetch_job_id("wacid-1"),
        whatsapp_call_native_recording_fetch_job_id("wacid-2")
    );
}

/// Port of: packages/worker-config/__tests__/whatsapp-call-native-media-job-ids.test.ts:93 "native media fetch job id helpers > whatsappCallNativeTranscriptFetchJobId is stable and keyed by wacid"
#[test]
fn whatsappcallnativetranscriptfetchjobid_is_stable_and_keyed() {
    assert_eq!(
        whatsapp_call_native_transcript_fetch_job_id("wacid-1"),
        "native-transcript-fetch-wacid-1"
    );
    assert_eq!(
        whatsapp_call_native_transcript_fetch_job_id("wacid-1"),
        whatsapp_call_native_transcript_fetch_job_id("wacid-1")
    );
    assert_ne!(
        whatsapp_call_native_transcript_fetch_job_id("wacid-1"),
        whatsapp_call_native_transcript_fetch_job_id("wacid-2")
    );
}

/// Port of: packages/worker-config/__tests__/whatsapp-call-native-media-job-ids.test.ts:105 "native media fetch job id helpers > the two new job ids never collide with each other for the same wacid"
#[test]
fn the_two_new_job_ids_never_collide_with_each_other_for_the() {
    assert_ne!(
        whatsapp_call_native_recording_fetch_job_id("wacid-1"),
        whatsapp_call_native_transcript_fetch_job_id("wacid-1")
    );
}

/// Port of: packages/worker-config/__tests__/whatsapp-call-native-media-job-ids.test.ts:111 "native media fetch job id helpers > the two new job ids never collide with the existing whatsappCallRecordingReady job id"
#[test]
fn the_two_new_job_ids_never_collide_with_the_existing() {
    assert_ne!(
        whatsapp_call_native_recording_fetch_job_id("call-1"),
        whatsapp_call_recording_ready_job_id("call-1")
    );
    assert_ne!(
        whatsapp_call_native_transcript_fetch_job_id("call-1"),
        whatsapp_call_recording_ready_job_id("call-1")
    );
}
