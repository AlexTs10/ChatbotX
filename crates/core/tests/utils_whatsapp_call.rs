//! Port of `packages/utils/__tests__/whatsapp-call.test.ts`.
#![allow(clippy::bool_assert_comparison)] // P3.1 orchestrator: style lint in locked ported test

use chatbotx_core::utils::whatsapp_call::{WhatsappCallMediaToggles, transcribes_calls};

/// Port of: packages/utils/__tests__/whatsapp-call.test.ts:5 "transcribesCalls > is on only when the number both records and transcribes calls"
#[test]
fn transcribescalls_is_on_only_when_the_number_both_records_and_transcribes_calls() {
    assert_eq!(
        transcribes_calls(WhatsappCallMediaToggles {
            call_recording_enabled: true,
            call_transcription_enabled: true,
        }),
        true
    );
}

/// Port of: packages/utils/__tests__/whatsapp-call.test.ts:14 "transcribesCalls > is off when transcription is on but recording is off"
#[test]
fn transcribescalls_is_off_when_transcription_is_on_but_recording_is_off() {
    assert_eq!(
        transcribes_calls(WhatsappCallMediaToggles {
            call_recording_enabled: false,
            call_transcription_enabled: true,
        }),
        false
    );
}

/// Port of: packages/utils/__tests__/whatsapp-call.test.ts:23 "transcribesCalls > is off when transcription itself is off"
#[test]
fn transcribescalls_is_off_when_transcription_itself_is_off() {
    assert_eq!(
        transcribes_calls(WhatsappCallMediaToggles {
            call_recording_enabled: true,
            call_transcription_enabled: false,
        }),
        false
    );
}
