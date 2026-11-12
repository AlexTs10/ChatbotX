//! Port of `packages/utils/__tests__/thread-control-channels.test.ts`.

use chatbotx_core::utils::channel::{
    ChannelType, archive_release_channels, is_thread_control_channel, supports_archive_release,
    thread_control_channels,
};

/// Port of: packages/utils/__tests__/thread-control-channels.test.ts:11 "isThreadControlChannel > returns true for routing-capable channel whatsapp"
#[test]
fn isthreadcontrolchannel_returns_true_for_routing_capable_channel_whatsapp() {
    assert!(
        thread_control_channels()
            .iter()
            .any(|c| c.as_str() == "whatsapp")
    );
    assert!(is_thread_control_channel(Some("whatsapp")));
}

/// Port of: packages/utils/__tests__/thread-control-channels.test.ts:11 "isThreadControlChannel > returns true for routing-capable channel messenger"
#[test]
fn isthreadcontrolchannel_returns_true_for_routing_capable_channel_messenger() {
    assert!(
        thread_control_channels()
            .iter()
            .any(|c| c.as_str() == "messenger")
    );
    assert!(is_thread_control_channel(Some("messenger")));
}

/// Port of: packages/utils/__tests__/thread-control-channels.test.ts:17 "isThreadControlChannel > returns false for a channel without routing"
#[test]
fn isthreadcontrolchannel_returns_false_for_a_channel_without_routing() {
    assert!(!is_thread_control_channel(Some("webchat")));
}

/// Port of: packages/utils/__tests__/thread-control-channels.test.ts:21 "isThreadControlChannel > returns false for null, undefined and unknown strings"
#[test]
fn isthreadcontrolchannel_returns_false_for_null_undefined_and_unknown_strings() {
    assert!(!is_thread_control_channel(None));
    assert!(!is_thread_control_channel(None));
    assert!(!is_thread_control_channel(Some("not-a-channel")));
}

/// Port of: packages/utils/__tests__/thread-control-channels.test.ts:29 "THREAD_CONTROL_CHANNELS > is a subset of channelTypes"
#[test]
fn thread_control_channels_is_a_subset_of_channeltypes() {
    for channel in thread_control_channels() {
        assert!(ChannelType::parse(channel.as_str()).is_some());
    }
}

/// Port of: packages/utils/__tests__/thread-control-channels.test.ts:37 "supportsArchiveRelease > whatsapp releases on archive, messenger does not"
#[test]
fn supportsarchiverelease_whatsapp_releases_on_archive_messenger_does_not() {
    assert!(supports_archive_release(Some("whatsapp")));
    assert!(!supports_archive_release(Some("messenger")));
}

/// Port of: packages/utils/__tests__/thread-control-channels.test.ts:42 "supportsArchiveRelease > is false for null, undefined and unknown channels"
#[test]
fn supportsarchiverelease_is_false_for_null_undefined_and_unknown_channels() {
    assert!(!supports_archive_release(None));
    assert!(!supports_archive_release(None));
    assert!(!supports_archive_release(Some("webchat")));
}

/// Port of: packages/utils/__tests__/thread-control-channels.test.ts:48 "supportsArchiveRelease > is a subset of the routing channels"
#[test]
fn supportsarchiverelease_is_a_subset_of_the_routing_channels() {
    for channel in archive_release_channels() {
        assert!(thread_control_channels().contains(channel));
    }
}
