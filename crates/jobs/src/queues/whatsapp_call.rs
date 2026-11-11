//! WhatsApp call native media fetch jobs on the `integration` queue
//! (`IntegrationJobAction.whatsappCallNative*Fetch` and their job ids).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The two native media fetch actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WhatsappCallJobAction {
    NativeRecordingFetch,
    NativeTranscriptFetch,
}

impl WhatsappCallJobAction {
    /// `"whatsappCallNativeRecordingFetch"`, `"whatsappCallNativeTranscriptFetch"`.
    pub fn as_str(self) -> &'static str {
        todo!()
    }
}

/// `whatsappCallNativeRecordingFetch` data: slim media references only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRecordingFetchData {
    pub whatsapp_call_id: String,
    pub wacid: String,
    pub workspace_id: String,
    pub audio_media_id: String,
    pub audio_url: String,
    pub mime_type: String,
}

/// `whatsappCallNativeTranscriptFetch` data: slim media references only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeTranscriptFetchData {
    pub whatsapp_call_id: String,
    pub wacid: String,
    pub workspace_id: String,
    pub document_media_id: String,
    pub document_url: String,
}

/// `{ type, data }` of a native media fetch job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WhatsappCallJobData {
    #[serde(rename = "whatsappCallNativeRecordingFetch")]
    NativeRecordingFetch(NativeRecordingFetchData),
    #[serde(rename = "whatsappCallNativeTranscriptFetch")]
    NativeTranscriptFetch(NativeTranscriptFetchData),
}

impl WhatsappCallJobData {
    /// The JSON written to the queue.
    pub fn to_value(&self) -> Value {
        todo!()
    }
}

/// `whatsappCallNativeRecordingFetchJobId(wacid)` = `native-rec-fetch-<wacid>`.
pub fn whatsapp_call_native_recording_fetch_job_id(wacid: &str) -> String {
    todo!()
}

/// `whatsappCallNativeTranscriptFetchJobId(wacid)` = `native-transcript-fetch-<wacid>`.
pub fn whatsapp_call_native_transcript_fetch_job_id(wacid: &str) -> String {
    todo!()
}

/// `whatsappCallRecordingReadyJobId(whatsappCallId)` (the existing job id).
pub fn whatsapp_call_recording_ready_job_id(whatsapp_call_id: &str) -> String {
    todo!()
}
