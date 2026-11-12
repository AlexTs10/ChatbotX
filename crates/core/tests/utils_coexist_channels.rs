//! Port of `packages/utils/__tests__/coexist-channels.test.ts`.

use chatbotx_core::utils::channel::{CoexistChannel, coexist_channels, is_coexist_channel};

/// Port of: packages/utils/__tests__/coexist-channels.test.ts:9 "isCoexistChannel > returns true for coexist channel whatsapp"
#[test]
fn iscoexistchannel_returns_true_for_coexist_channel_whatsapp() {
    assert!(coexist_channels().iter().any(|c| c.as_str() == "whatsapp"));
    assert!(is_coexist_channel(Some("whatsapp")));
}

/// Port of: packages/utils/__tests__/coexist-channels.test.ts:9 "isCoexistChannel > returns true for coexist channel messenger"
#[test]
fn iscoexistchannel_returns_true_for_coexist_channel_messenger() {
    assert!(coexist_channels().iter().any(|c| c.as_str() == "messenger"));
    assert!(is_coexist_channel(Some("messenger")));
}

/// Port of: packages/utils/__tests__/coexist-channels.test.ts:9 "isCoexistChannel > returns true for coexist channel instagram"
#[test]
fn iscoexistchannel_returns_true_for_coexist_channel_instagram() {
    assert!(coexist_channels().iter().any(|c| c.as_str() == "instagram"));
    assert!(is_coexist_channel(Some("instagram")));
}

/// Port of: packages/utils/__tests__/coexist-channels.test.ts:15 "isCoexistChannel > returns false for a non-coexist channel"
#[test]
fn iscoexistchannel_returns_false_for_a_non_coexist_channel() {
    assert!(!is_coexist_channel(Some("webchat")));
    assert!(!is_coexist_channel(Some("telegram")));
}

/// Port of: packages/utils/__tests__/coexist-channels.test.ts:20 "isCoexistChannel > returns false for null and undefined"
#[test]
fn iscoexistchannel_returns_false_for_null_and_undefined() {
    assert!(!is_coexist_channel(None));
    assert!(!is_coexist_channel(None));
}

/// Port of: packages/utils/__tests__/coexist-channels.test.ts:25 "isCoexistChannel > returns false for an unknown string"
#[test]
fn iscoexistchannel_returns_false_for_an_unknown_string() {
    assert!(!is_coexist_channel(Some("not-a-channel")));
}

/// Port of: packages/utils/__tests__/coexist-channels.test.ts:31 "COEXIST_CHANNELS > matches coexistChannels.options exactly"
#[test]
fn coexist_channels_matches_coexistchannels_options_exactly() {
    assert_eq!(coexist_channels(), CoexistChannel::ALL);
    let names: Vec<&str> = coexist_channels().iter().map(|c| c.as_str()).collect();
    assert_eq!(names, vec!["whatsapp", "messenger", "instagram"]);
}
