//! Port of `packages/business/__tests__/whatsapp-voip-sdp.test.ts` (`pinAnswerDtlsSetup`).
use chatbotx_business::whatsapp_voip::pin_answer_dtls_setup;

#[test]
fn rewrites_an_actpass_setup_line_to_active_keeping_crlf() {
    assert_eq!(
        pin_answer_dtls_setup("v=0\r\na=setup:actpass\r\nm=audio\r\n"),
        "v=0\r\na=setup:active\r\nm=audio\r\n"
    );
}

#[test]
fn rewrites_every_actpass_line_at_session_and_media_level() {
    let sdp = "a=setup:actpass\nm=audio 9\na=setup:actpass\n";
    assert_eq!(
        pin_answer_dtls_setup(sdp),
        "a=setup:active\nm=audio 9\na=setup:active\n"
    );
}

#[test]
fn rewrites_an_actpass_line_with_no_trailing_newline() {
    assert_eq!(
        pin_answer_dtls_setup("v=0\na=setup:actpass"),
        "v=0\na=setup:active"
    );
}

#[test]
fn leaves_a_valid_answer_role_untouched() {
    let sdp = "a=setup:active\r\na=setup:passive\r\n";
    assert_eq!(pin_answer_dtls_setup(sdp), sdp);
}

#[test]
fn never_touches_actpass_text_that_is_not_a_whole_setup_line() {
    let sdp = "a=x-note:a=setup:actpass\na=setup:actpass-ish\n";
    assert_eq!(pin_answer_dtls_setup(sdp), sdp);
}
