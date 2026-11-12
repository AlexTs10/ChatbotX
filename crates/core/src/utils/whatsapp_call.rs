//! `packages/utils/src/whatsapp-call.ts`.

/// The two per-number call media toggles on `IntegrationWhatsapp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhatsappCallMediaToggles {
    pub call_recording_enabled: bool,
    pub call_transcription_enabled: bool,
}

/// `transcribesCalls`: transcription only runs with recording on.
pub fn transcribes_calls(toggles: WhatsappCallMediaToggles) -> bool {
    toggles.call_recording_enabled && toggles.call_transcription_enabled
}
