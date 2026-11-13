//! Port of `packages/database/__tests__/contact-partial.test.ts` (P3.2b, ported; one test
//! per `test.each` row).

use chatbotx_db::repo::partials::contact::{PROFILE_SNAPSHOT_CHANNELS, supports_profile_snapshot};

#[test]
fn supports_profile_snapshot_accepts_every_channel_in_the_capability_list() {
    for channel in PROFILE_SNAPSHOT_CHANNELS {
        assert!(supports_profile_snapshot(channel), "{channel}");
    }
}

#[test]
fn supports_profile_snapshot_rejects_messenger() {
    assert!(!supports_profile_snapshot("messenger"));
}

#[test]
fn supports_profile_snapshot_rejects_whatsapp() {
    assert!(!supports_profile_snapshot("whatsapp"));
}

#[test]
fn supports_profile_snapshot_rejects_telegram() {
    assert!(!supports_profile_snapshot("telegram"));
}

#[test]
fn supports_profile_snapshot_rejects_webchat() {
    assert!(!supports_profile_snapshot("webchat"));
}

#[test]
fn supports_profile_snapshot_rejects_the_empty_string() {
    assert!(!supports_profile_snapshot(""));
}
