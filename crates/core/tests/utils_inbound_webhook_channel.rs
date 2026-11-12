//! Port of `packages/utils/__tests__/inbound-webhook-channel.test.ts`.

use std::collections::HashSet;

use chatbotx_core::utils::channel::ChannelType;
use chatbotx_core::utils::inbound_webhook_channel::{
    CONTACT_LOOKUP_ORDER, ChannelUserIdKind, ContactIdentityKind, InboundWebhookChannelSpec,
    InboxIdentifierKind, InboxLookupStrategy, get_inbox_identifier_strategy,
    inbound_webhook_channel_spec, missing_creation_identity_kinds, payload_resolvable_channels,
};
use chatbotx_core::utils::json_path::canonicalize_json_path;

fn kinds(names: &[&str]) -> Vec<ContactIdentityKind> {
    names
        .iter()
        .map(|n| ContactIdentityKind::parse(n).unwrap())
        .collect()
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:13 "INBOUND_WEBHOOK_CHANNEL_SPECS > has an entry for every channel type"
#[test]
fn inbound_webhook_channel_specs_has_an_entry_for_every_channel_type() {
    // A total `match` has an entry for every channel.
    for channel in ChannelType::ALL {
        let _ = inbound_webhook_channel_spec(*channel);
    }
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:19 "INBOUND_WEBHOOK_CHANNEL_SPECS > lets a contact be created by channelUserId only when the channel has one"
#[test]
fn inbound_webhook_channel_specs_lets_a_contact_be_created_by_channeluserid_only_when_the_channel_has_one()
 {
    for channel in ChannelType::ALL {
        let spec = inbound_webhook_channel_spec(*channel);
        if spec
            .creation_identities
            .contains(&ContactIdentityKind::ChannelUserId)
        {
            assert!(spec.channel_user_id.is_some(), "{channel}");
        }
    }
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:28 "INBOUND_WEBHOOK_CHANNEL_SPECS > gives every example path in canonical $ form"
#[test]
fn inbound_webhook_channel_specs_gives_every_example_path_in_canonical_form() {
    for channel in ChannelType::ALL {
        let spec = inbound_webhook_channel_spec(*channel);
        let mut paths: Vec<&str> = spec
            .inbox_identifiers
            .iter()
            .map(|i| i.example_path)
            .collect();
        if let Some(user) = spec.channel_user_id {
            paths.push(user.example_path);
        }
        for path in paths {
            assert_eq!(canonicalize_json_path(path).as_deref(), Some(path));
        }
    }
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:41 "INBOUND_WEBHOOK_CHANNEL_SPECS > never lists the same inbox identifier kind twice for a channel"
#[test]
fn inbound_webhook_channel_specs_never_lists_the_same_inbox_identifier_kind_twice_for_a_channel() {
    for channel in ChannelType::ALL {
        let kinds: Vec<InboxIdentifierKind> = inbound_webhook_channel_spec(*channel)
            .inbox_identifiers
            .iter()
            .map(|i| i.kind)
            .collect();
        assert_eq!(kinds.iter().collect::<HashSet<_>>().len(), kinds.len());
    }
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:50 "INBOUND_WEBHOOK_CHANNEL_SPECS > creates a WhatsApp contact from its phone, while wa_id can still find one"
#[test]
fn inbound_webhook_channel_specs_creates_a_whatsapp_contact_from_its_phone_while_wa_id_can_still_find_one()
 {
    let spec = inbound_webhook_channel_spec(ChannelType::Whatsapp);
    assert_eq!(
        spec.channel_user_id.map(|u| u.kind),
        Some(ChannelUserIdKind::WhatsappId)
    );
    assert_eq!(spec.creation_identities, &[ContactIdentityKind::Phone]);
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:57 "INBOUND_WEBHOOK_CHANNEL_SPECS > leaves omnichannel unusable"
#[test]
fn inbound_webhook_channel_specs_leaves_omnichannel_unusable() {
    assert_eq!(
        inbound_webhook_channel_spec(ChannelType::Omnichannel),
        InboundWebhookChannelSpec {
            inbox_identifiers: &[],
            channel_user_id: None,
            creation_identities: &[],
        }
    );
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:67 "CONTACT_LOOKUP_ORDER > searches by channel user id, phone, email, then full name"
#[test]
fn contact_lookup_order_searches_by_channel_user_id_phone_email_then_full_name() {
    assert_eq!(
        CONTACT_LOOKUP_ORDER.to_vec(),
        kinds(&["channelUserId", "phone", "email", "fullName"])
    );
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:78 "PAYLOAD_RESOLVABLE_CHANNELS > holds the channels with an external inbox id, never the fixed-inbox ones"
#[test]
fn payload_resolvable_channels_holds_the_channels_with_an_external_inbox_id_never_the_fixed_inbox_ones()
 {
    let names: Vec<&str> = payload_resolvable_channels()
        .iter()
        .map(|c| c.as_str())
        .collect();
    assert_eq!(
        names,
        vec![
            "messenger",
            "whatsapp",
            "zalo",
            "telegram",
            "instagram",
            "threads",
            "tiktok"
        ]
    );
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:92 "getInboxIdentifierStrategy > returns the strategy a channel declares for a kind"
#[test]
fn getinboxidentifierstrategy_returns_the_strategy_a_channel_declares_for_a_kind() {
    assert_eq!(
        get_inbox_identifier_strategy(
            ChannelType::Whatsapp,
            InboxIdentifierKind::DisplayPhoneNumber
        ),
        Some(InboxLookupStrategy::WhatsappDisplayPhone)
    );
    assert_eq!(
        get_inbox_identifier_strategy(ChannelType::Tiktok, InboxIdentifierKind::OpenId),
        Some(InboxLookupStrategy::ConnectionSourceId)
    );
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:101 "getInboxIdentifierStrategy > returns undefined for a kind the channel does not declare"
#[test]
fn getinboxidentifierstrategy_returns_undefined_for_a_kind_the_channel_does_not_declare() {
    assert_eq!(
        get_inbox_identifier_strategy(ChannelType::Messenger, InboxIdentifierKind::IgId),
        None
    );
    assert_eq!(
        get_inbox_identifier_strategy(ChannelType::Webchat, InboxIdentifierKind::PageId),
        None
    );
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:108 "missingCreationIdentityKinds > messenger with ["channelUserId"] is missing []"
#[test]
fn missingcreationidentitykinds_messenger_with_channeluserid_is_missing() {
    assert_eq!(
        missing_creation_identity_kinds(
            ChannelType::parse("messenger").unwrap(),
            &kinds(&["channelUserId"])
        ),
        kinds(&[])
    );
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:108 "missingCreationIdentityKinds > messenger with ["phone", "email", "fullName"] is missing ["channelUserId"]"
#[test]
fn missingcreationidentitykinds_messenger_with_phone_email_fullname_is_missing_channeluserid() {
    assert_eq!(
        missing_creation_identity_kinds(
            ChannelType::parse("messenger").unwrap(),
            &kinds(&["phone", "email", "fullName"])
        ),
        kinds(&["channelUserId"])
    );
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:108 "missingCreationIdentityKinds > whatsapp with ["phone"] is missing []"
#[test]
fn missingcreationidentitykinds_whatsapp_with_phone_is_missing() {
    assert_eq!(
        missing_creation_identity_kinds(
            ChannelType::parse("whatsapp").unwrap(),
            &kinds(&["phone"])
        ),
        kinds(&[])
    );
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:108 "missingCreationIdentityKinds > whatsapp with ["channelUserId"] is missing ["phone"]"
#[test]
fn missingcreationidentitykinds_whatsapp_with_channeluserid_is_missing_phone() {
    assert_eq!(
        missing_creation_identity_kinds(
            ChannelType::parse("whatsapp").unwrap(),
            &kinds(&["channelUserId"])
        ),
        kinds(&["phone"])
    );
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:108 "missingCreationIdentityKinds > smtp with ["phone"] is missing ["email"]"
#[test]
fn missingcreationidentitykinds_smtp_with_phone_is_missing_email() {
    assert_eq!(
        missing_creation_identity_kinds(ChannelType::parse("smtp").unwrap(), &kinds(&["phone"])),
        kinds(&["email"])
    );
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:108 "missingCreationIdentityKinds > webchat with ["email"] is missing []"
#[test]
fn missingcreationidentitykinds_webchat_with_email_is_missing() {
    assert_eq!(
        missing_creation_identity_kinds(ChannelType::parse("webchat").unwrap(), &kinds(&["email"])),
        kinds(&[])
    );
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:108 "missingCreationIdentityKinds > webchat with ["fullName"] is missing ["phone", "email"]"
#[test]
fn missingcreationidentitykinds_webchat_with_fullname_is_missing_phone_email() {
    assert_eq!(
        missing_creation_identity_kinds(
            ChannelType::parse("webchat").unwrap(),
            &kinds(&["fullName"])
        ),
        kinds(&["phone", "email"])
    );
}

/// Port of: packages/utils/__tests__/inbound-webhook-channel.test.ts:108 "missingCreationIdentityKinds > omnichannel with ["fullName"] is missing []"
#[test]
fn missingcreationidentitykinds_omnichannel_with_fullname_is_missing() {
    assert_eq!(
        missing_creation_identity_kinds(
            ChannelType::parse("omnichannel").unwrap(),
            &kinds(&["fullName"])
        ),
        kinds(&[])
    );
}
