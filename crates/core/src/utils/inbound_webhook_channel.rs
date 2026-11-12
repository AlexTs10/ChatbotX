//! `packages/utils/src/inbound-webhook-channel.ts`.

use super::channel::ChannelType;

string_enum! {
    /// `inboxIdentifierKinds`.
    pub enum InboxIdentifierKind {
        PageId = "pageId",
        IgId = "igId",
        PhoneNumberId = "phoneNumberId",
        DisplayPhoneNumber = "displayPhoneNumber",
        BotId = "botId",
        OaId = "oaId",
        OpenId = "openId",
        ThreadsUserId = "threadsUserId",
    }
}

string_enum! {
    /// `inboxLookupStrategies`.
    pub enum InboxLookupStrategy {
        InboxSourceId = "inboxSourceId",
        ConnectionSourceId = "connectionSourceId",
        WhatsappDisplayPhone = "whatsappDisplayPhone",
    }
}

string_enum! {
    /// `contactIdentityKinds` (the option order is the lookup order).
    pub enum ContactIdentityKind {
        ChannelUserId = "channelUserId",
        Phone = "phone",
        Email = "email",
        FullName = "fullName",
    }
}

/// `CONTACT_LOOKUP_ORDER`.
pub const CONTACT_LOOKUP_ORDER: &[ContactIdentityKind] = ContactIdentityKind::ALL;

string_enum! {
    /// `channelUserIdKinds`.
    pub enum ChannelUserIdKind {
        Psid = "psid",
        Igsid = "igsid",
        TelegramChatId = "telegramChatId",
        ZaloUserId = "zaloUserId",
        TiktokCustomerOpenId = "tiktokCustomerOpenId",
        ThreadsUsername = "threadsUsername",
        WhatsappId = "whatsappId",
        ApiSourceId = "apiSourceId",
    }
}

/// One inbox identifier of a channel spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboxIdentifier {
    pub kind: InboxIdentifierKind,
    pub strategy: InboxLookupStrategy,
    pub example_path: &'static str,
}

/// The channel's own contact id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelUserId {
    pub kind: ChannelUserIdKind,
    pub example_path: &'static str,
}

/// `InboundWebhookChannelSpec`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboundWebhookChannelSpec {
    pub inbox_identifiers: &'static [InboxIdentifier],
    pub channel_user_id: Option<ChannelUserId>,
    pub creation_identities: &'static [ContactIdentityKind],
}

/// An `InboxIdentifier` literal (a struct literal, so `&[...]` is promoted
/// to `'static`).
macro_rules! ident {
    ($kind:expr, $strategy:expr, $path:expr $(,)?) => {
        InboxIdentifier {
            kind: $kind,
            strategy: $strategy,
            example_path: $path,
        }
    };
}

const fn user(kind: ChannelUserIdKind, example_path: &'static str) -> Option<ChannelUserId> {
    Some(ChannelUserId { kind, example_path })
}

/// `INBOUND_WEBHOOK_CHANNEL_SPECS[channel]`.
pub fn inbound_webhook_channel_spec(channel: ChannelType) -> InboundWebhookChannelSpec {
    use ChannelUserIdKind as U;
    use ContactIdentityKind as K;
    use InboxIdentifierKind as I;
    use InboxLookupStrategy as S;
    const BY_USER_ID: &[ContactIdentityKind] = &[ContactIdentityKind::ChannelUserId];
    match channel {
        ChannelType::Omnichannel => InboundWebhookChannelSpec {
            inbox_identifiers: &[],
            channel_user_id: None,
            creation_identities: &[],
        },
        ChannelType::Webchat => InboundWebhookChannelSpec {
            inbox_identifiers: &[],
            channel_user_id: None,
            creation_identities: &[K::Phone, K::Email],
        },
        ChannelType::Smtp => InboundWebhookChannelSpec {
            inbox_identifiers: &[],
            channel_user_id: None,
            creation_identities: &[K::Email],
        },
        ChannelType::Api => InboundWebhookChannelSpec {
            inbox_identifiers: &[],
            channel_user_id: user(U::ApiSourceId, "$.contact.id"),
            creation_identities: BY_USER_ID,
        },
        ChannelType::Messenger => InboundWebhookChannelSpec {
            inbox_identifiers: &[ident!(I::PageId, S::InboxSourceId, "$.entry[0].id")],
            channel_user_id: user(U::Psid, "$.sender.id"),
            creation_identities: BY_USER_ID,
        },
        ChannelType::Instagram => InboundWebhookChannelSpec {
            inbox_identifiers: &[ident!(I::IgId, S::InboxSourceId, "$.entry[0].id")],
            channel_user_id: user(U::Igsid, "$.sender.id"),
            creation_identities: BY_USER_ID,
        },
        ChannelType::Whatsapp => InboundWebhookChannelSpec {
            inbox_identifiers: &[
                ident!(
                    I::PhoneNumberId,
                    S::InboxSourceId,
                    "$.metadata.phone_number_id",
                ),
                ident!(
                    I::DisplayPhoneNumber,
                    S::WhatsappDisplayPhone,
                    "$.metadata.display_phone_number",
                ),
            ],
            channel_user_id: user(U::WhatsappId, "$.contacts[0].wa_id"),
            creation_identities: &[K::Phone],
        },
        ChannelType::Telegram => InboundWebhookChannelSpec {
            inbox_identifiers: &[ident!(I::BotId, S::InboxSourceId, "$.bot.id")],
            channel_user_id: user(U::TelegramChatId, "$.chat.id"),
            creation_identities: BY_USER_ID,
        },
        ChannelType::Zalo => InboundWebhookChannelSpec {
            inbox_identifiers: &[ident!(I::OaId, S::InboxSourceId, "$.oa_id")],
            channel_user_id: user(U::ZaloUserId, "$.user_id"),
            creation_identities: BY_USER_ID,
        },
        ChannelType::Tiktok => InboundWebhookChannelSpec {
            inbox_identifiers: &[ident!(I::OpenId, S::ConnectionSourceId, "$.open_id")],
            channel_user_id: user(U::TiktokCustomerOpenId, "$.customer.open_id"),
            creation_identities: BY_USER_ID,
        },
        ChannelType::Threads => InboundWebhookChannelSpec {
            inbox_identifiers: &[ident!(I::ThreadsUserId, S::InboxSourceId, "$.user_id")],
            channel_user_id: user(U::ThreadsUsername, "$.user.username"),
            creation_identities: BY_USER_ID,
        },
    }
}

/// `GENERIC_CHANNEL_USER_ID_EXAMPLE_PATH`.
pub const GENERIC_CHANNEL_USER_ID_EXAMPLE_PATH: &str = "$.contact.id";

/// `PAYLOAD_RESOLVABLE_CHANNELS`.
pub fn payload_resolvable_channels() -> Vec<ChannelType> {
    ChannelType::ALL
        .iter()
        .copied()
        .filter(|c| {
            !inbound_webhook_channel_spec(*c)
                .inbox_identifiers
                .is_empty()
        })
        .collect()
}

/// `missingCreationIdentityKinds`.
pub fn missing_creation_identity_kinds(
    channel: ChannelType,
    configured: &[ContactIdentityKind],
) -> Vec<ContactIdentityKind> {
    let creation = inbound_webhook_channel_spec(channel).creation_identities;
    if creation.iter().any(|k| configured.contains(k)) {
        Vec::new()
    } else {
        creation.to_vec()
    }
}

/// `getInboxIdentifierStrategy`.
pub fn get_inbox_identifier_strategy(
    channel: ChannelType,
    kind: InboxIdentifierKind,
) -> Option<InboxLookupStrategy> {
    inbound_webhook_channel_spec(channel)
        .inbox_identifiers
        .iter()
        .find(|i| i.kind == kind)
        .map(|i| i.strategy)
}
