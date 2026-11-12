//! Port of `packages/utils/__tests__/call-capable-channels.test.ts`.

use chatbotx_core::utils::channel::{ChannelType, call_capable_channels, channel_capabilities};

/// Port of: packages/utils/__tests__/call-capable-channels.test.ts:9 "CALL_CAPABLE_CHANNELS > lists exactly the channels flagged callable in the registry"
#[test]
fn call_capable_channels_lists_exactly_the_channels_flagged_callable_in_the_registry() {
    let expected: Vec<ChannelType> = ChannelType::ALL
        .iter()
        .copied()
        .filter(|channel| channel_capabilities(*channel).callable)
        .collect();
    assert_eq!(call_capable_channels(), expected);
}

/// Port of: packages/utils/__tests__/call-capable-channels.test.ts:17 "CALL_CAPABLE_CHANNELS > holds whatsapp today — the only channel with a calling API"
#[test]
fn call_capable_channels_holds_whatsapp_today_the_only_channel_with_a_calling_api() {
    assert_eq!(call_capable_channels(), vec![ChannelType::Whatsapp]);
    assert_eq!(call_capable_channels()[0].as_str(), "whatsapp");
}

/// Port of: packages/utils/__tests__/call-capable-channels.test.ts:21 "CALL_CAPABLE_CHANNELS > never lists a channel that cannot be connected at all"
#[test]
fn call_capable_channels_never_lists_a_channel_that_cannot_be_connected_at_all() {
    for channel in call_capable_channels() {
        assert!(channel_capabilities(channel).manageable);
    }
}
