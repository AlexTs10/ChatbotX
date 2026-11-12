//! `packages/utils/src/channel.ts`: the channel registry and the channel
//! subsets shared code keys off.

string_enum! {
    /// `channelTypes`.
    pub enum ChannelType {
        Omnichannel = "omnichannel",
        Webchat = "webchat",
        Messenger = "messenger",
        Whatsapp = "whatsapp",
        Zalo = "zalo",
        Smtp = "smtp",
        Telegram = "telegram",
        Instagram = "instagram",
        Threads = "threads",
        Tiktok = "tiktok",
        Api = "api",
    }
}

string_enum! {
    /// `adsEligibleChannelTypes`.
    pub enum AdsEligibleChannelType {
        Whatsapp = "whatsapp",
        Messenger = "messenger",
        Instagram = "instagram",
    }
}

string_enum! {
    /// `adReferralChannelTypes`.
    pub enum AdReferralChannelType {
        Messenger = "messenger",
        Instagram = "instagram",
    }
}

/// `DEFAULT_ADS_CONVERSION_CHANNEL`.
pub const DEFAULT_ADS_CONVERSION_CHANNEL: AdsEligibleChannelType = AdsEligibleChannelType::Whatsapp;

/// `ChannelCapability`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelCapability {
    pub creatable: bool,
    pub manageable: bool,
    pub requires_credential: bool,
    pub order: u32,
    pub callable: bool,
}

const fn cap(
    creatable: bool,
    manageable: bool,
    requires_credential: bool,
    order: u32,
    callable: bool,
) -> ChannelCapability {
    ChannelCapability {
        creatable,
        manageable,
        requires_credential,
        order,
        callable,
    }
}

/// `CHANNEL_CAPABILITIES[channel]`.
pub fn channel_capabilities(channel: ChannelType) -> ChannelCapability {
    match channel {
        ChannelType::Whatsapp => cap(true, true, true, 1, true),
        ChannelType::Messenger => cap(true, true, true, 2, false),
        ChannelType::Instagram => cap(true, true, true, 3, false),
        ChannelType::Threads => cap(true, true, true, 4, false),
        ChannelType::Tiktok => cap(true, true, true, 5, false),
        ChannelType::Telegram => cap(true, true, false, 6, false),
        ChannelType::Zalo => cap(true, true, true, 7, false),
        ChannelType::Webchat => cap(true, true, false, 8, false),
        ChannelType::Smtp => cap(false, true, false, 9, false),
        ChannelType::Api => cap(true, true, false, 10, false),
        ChannelType::Omnichannel => cap(false, false, false, 11, false),
    }
}

fn ordered(filter: impl Fn(ChannelCapability) -> bool) -> Vec<ChannelType> {
    let mut channels: Vec<ChannelType> = ChannelType::ALL
        .iter()
        .copied()
        .filter(|c| filter(channel_capabilities(*c)))
        .collect();
    channels.sort_by_key(|c| channel_capabilities(*c).order);
    channels
}

/// `CREATABLE_CHANNELS`, in display order.
pub fn creatable_channels() -> Vec<ChannelType> {
    ordered(|c| c.creatable)
}

/// `MANAGEABLE_CHANNELS`, in display order.
pub fn manageable_channels() -> Vec<ChannelType> {
    ordered(|c| c.manageable)
}

/// `CALL_CAPABLE_CHANNELS`, in display order.
pub fn call_capable_channels() -> Vec<ChannelType> {
    ordered(|c| c.callable)
}

string_enum! {
    /// `coexistChannels`.
    pub enum CoexistChannel {
        Whatsapp = "whatsapp",
        Messenger = "messenger",
        Instagram = "instagram",
    }
}

/// `COEXIST_CHANNELS`.
pub fn coexist_channels() -> &'static [CoexistChannel] {
    CoexistChannel::ALL
}

/// `isCoexistChannel`.
pub fn is_coexist_channel(channel: Option<&str>) -> bool {
    channel.and_then(CoexistChannel::parse).is_some()
}

string_enum! {
    /// `contactScanChannels`.
    pub enum ContactScanChannel {
        Messenger = "messenger",
        Instagram = "instagram",
    }
}

/// `isContactScanChannel`.
pub fn is_contact_scan_channel(channel: Option<&str>) -> bool {
    channel.and_then(ContactScanChannel::parse).is_some()
}

string_enum! {
    /// `threadControlChannels`.
    pub enum ThreadControlChannel {
        Whatsapp = "whatsapp",
        Messenger = "messenger",
    }
}

/// `THREAD_CONTROL_CHANNELS`.
pub fn thread_control_channels() -> &'static [ThreadControlChannel] {
    ThreadControlChannel::ALL
}

/// `isThreadControlChannel`.
pub fn is_thread_control_channel(channel: Option<&str>) -> bool {
    channel.and_then(ThreadControlChannel::parse).is_some()
}

/// `AiHandoverChannelPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiHandoverChannelPolicy {
    pub takeover_message_window_ms: i64,
    pub hand_to_ai_active_within_days: u32,
    pub message_max_length: usize,
}

const MS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

/// `AI_HANDOVER_CHANNEL_POLICIES[channel]` (only Messenger has one).
pub fn ai_handover_channel_policy(
    channel: ThreadControlChannel,
) -> Option<AiHandoverChannelPolicy> {
    match channel {
        ThreadControlChannel::Messenger => Some(AiHandoverChannelPolicy {
            takeover_message_window_ms: 7 * MS_PER_DAY,
            hand_to_ai_active_within_days: 30,
            message_max_length: 2000,
        }),
        ThreadControlChannel::Whatsapp => None,
    }
}

/// `parseAiHandoverChannel`.
pub fn parse_ai_handover_channel(channel: Option<&str>) -> Option<ThreadControlChannel> {
    channel
        .and_then(ThreadControlChannel::parse)
        .filter(|c| ai_handover_channel_policy(*c).is_some())
}

/// `ARCHIVE_RELEASE_CHANNELS`.
pub fn archive_release_channels() -> &'static [ThreadControlChannel] {
    &[ThreadControlChannel::Whatsapp]
}

/// `supportsArchiveRelease`.
pub fn supports_archive_release(channel: Option<&str>) -> bool {
    channel == Some("whatsapp")
}
