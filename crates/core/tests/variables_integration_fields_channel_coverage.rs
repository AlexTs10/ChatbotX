//! Port of `packages/variables/__tests__/integration-fields-channel-coverage.test.ts`. The channel matrix is
//! `InboxWithIntegrations::INTEGRATION_RELATIONS` (the `integration*`
//! relations of `inboxRelations`).

mod support;

use chatbotx_core::utils::channel::ChannelType;
use chatbotx_core::variables::*;
use support::variables::*;

fn contact() -> ContactModel {
    ContactModel {
        id: "contact-1".into(),
        workspace_id: "workspace-1".into(),
        timezone: s("UTC"),
        ..Default::default()
    }
}

fn contact_inbox(channel: &str) -> ContactInboxModel {
    ContactInboxModel {
        id: "contact-inbox-1".into(),
        channel: channel.into(),
        inbox_id: "inbox-1".into(),
        source_id: "source-1".into(),
        ..Default::default()
    }
}

fn harness_for(channel: &str) -> Harness {
    let mut inbox = InboxWithIntegrations {
        id: "inbox-1".into(),
        ..Default::default()
    };
    assert!(inbox.set_integration(
        channel,
        IntegrationRow {
            id: format!("integration-{channel}"),
            name: Some(format!("Name {channel}")),
            ..Default::default()
        },
    ));
    Harness::new(FakeState {
        tenant: TenantSettings {
            app_url: "https://app.example.test".into(),
            ..Default::default()
        },
        app_url: "https://app.example.test".into(),
        me_row_id: "system-field-1".into(),
        inbox: Ok(Some(inbox)),
        ..Default::default()
    })
}

fn channels() -> Vec<&'static str> {
    let mut channels: Vec<&str> = InboxWithIntegrations::INTEGRATION_RELATIONS
        .iter()
        .map(|(c, _)| *c)
        .collect();
    channels.sort();
    channels
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:130 "the derived channel matrix is non-empty and only contains real channels"
#[test]
fn the_derived_channel_matrix_is_non_empty_and_only_contains_real_channels() {
    assert!(!channels().is_empty());
    for channel in channels() {
        assert!(
            ChannelType::parse(channel).is_some(),
            "{channel} is not a ChannelType"
        );
    }
    for (channel, relation) in InboxWithIntegrations::INTEGRATION_RELATIONS {
        let suffix = &relation["integration".len()..];
        assert_eq!(
            format!("{}{}", suffix[..1].to_lowercase(), &suffix[1..]),
            *channel
        );
    }
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:142 "page_user_name resolves the instagram integration name"
#[tokio::test]
async fn page_user_name_resolves_the_instagram_integration_name() {
    let h = harness_for("instagram");
    let value = h
        .v()
        .get_integration_field(
            &contact(),
            SystemFieldType::PageUserName,
            Some(&contact_inbox("instagram")),
            None,
        )
        .await
        .unwrap();
    assert_eq!(value, s("Name instagram"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:142 "page_user_name resolves the messenger integration name"
#[tokio::test]
async fn page_user_name_resolves_the_messenger_integration_name() {
    let h = harness_for("messenger");
    let value = h
        .v()
        .get_integration_field(
            &contact(),
            SystemFieldType::PageUserName,
            Some(&contact_inbox("messenger")),
            None,
        )
        .await
        .unwrap();
    assert_eq!(value, s("Name messenger"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:142 "page_user_name resolves the smtp integration name"
#[tokio::test]
async fn page_user_name_resolves_the_smtp_integration_name() {
    let h = harness_for("smtp");
    let value = h
        .v()
        .get_integration_field(
            &contact(),
            SystemFieldType::PageUserName,
            Some(&contact_inbox("smtp")),
            None,
        )
        .await
        .unwrap();
    assert_eq!(value, s("Name smtp"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:142 "page_user_name resolves the telegram integration name"
#[tokio::test]
async fn page_user_name_resolves_the_telegram_integration_name() {
    let h = harness_for("telegram");
    let value = h
        .v()
        .get_integration_field(
            &contact(),
            SystemFieldType::PageUserName,
            Some(&contact_inbox("telegram")),
            None,
        )
        .await
        .unwrap();
    assert_eq!(value, s("Name telegram"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:142 "page_user_name resolves the threads integration name"
#[tokio::test]
async fn page_user_name_resolves_the_threads_integration_name() {
    let h = harness_for("threads");
    let value = h
        .v()
        .get_integration_field(
            &contact(),
            SystemFieldType::PageUserName,
            Some(&contact_inbox("threads")),
            None,
        )
        .await
        .unwrap();
    assert_eq!(value, s("Name threads"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:142 "page_user_name resolves the tiktok integration name"
#[tokio::test]
async fn page_user_name_resolves_the_tiktok_integration_name() {
    let h = harness_for("tiktok");
    let value = h
        .v()
        .get_integration_field(
            &contact(),
            SystemFieldType::PageUserName,
            Some(&contact_inbox("tiktok")),
            None,
        )
        .await
        .unwrap();
    assert_eq!(value, s("Name tiktok"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:142 "page_user_name resolves the webchat integration name"
#[tokio::test]
async fn page_user_name_resolves_the_webchat_integration_name() {
    let h = harness_for("webchat");
    let value = h
        .v()
        .get_integration_field(
            &contact(),
            SystemFieldType::PageUserName,
            Some(&contact_inbox("webchat")),
            None,
        )
        .await
        .unwrap();
    assert_eq!(value, s("Name webchat"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:142 "page_user_name resolves the whatsapp integration name"
#[tokio::test]
async fn page_user_name_resolves_the_whatsapp_integration_name() {
    let h = harness_for("whatsapp");
    let value = h
        .v()
        .get_integration_field(
            &contact(),
            SystemFieldType::PageUserName,
            Some(&contact_inbox("whatsapp")),
            None,
        )
        .await
        .unwrap();
    assert_eq!(value, s("Name whatsapp"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:142 "page_user_name resolves the zalo integration name"
#[tokio::test]
async fn page_user_name_resolves_the_zalo_integration_name() {
    let h = harness_for("zalo");
    let value = h
        .v()
        .get_integration_field(
            &contact(),
            SystemFieldType::PageUserName,
            Some(&contact_inbox("zalo")),
            None,
        )
        .await
        .unwrap();
    assert_eq!(value, s("Name zalo"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:158 "getChannelIntegrationId resolves the instagram integration id for the me link"
#[tokio::test]
async fn getchannelintegrationid_resolves_the_instagram_integration_id_for_the_me_link() {
    let h = harness_for("instagram");
    let link = h
        .v()
        .get_integration_field(&contact(), SystemFieldType::Me, Some(&contact_inbox("instagram")), None)
        .await
        .unwrap()
        .expect("getChannelIntegrationId has no \"instagram\" case, so the me link cannot be built for it");
    let url = url::Url::parse(&link).unwrap();
    let ib = url
        .query_pairs()
        .find(|(k, _)| k == "ib")
        .map(|(_, v)| v.into_owned());
    assert_eq!(ib, s("integration-instagram"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:158 "getChannelIntegrationId resolves the messenger integration id for the me link"
#[tokio::test]
async fn getchannelintegrationid_resolves_the_messenger_integration_id_for_the_me_link() {
    let h = harness_for("messenger");
    let link = h
        .v()
        .get_integration_field(&contact(), SystemFieldType::Me, Some(&contact_inbox("messenger")), None)
        .await
        .unwrap()
        .expect("getChannelIntegrationId has no \"messenger\" case, so the me link cannot be built for it");
    let url = url::Url::parse(&link).unwrap();
    let ib = url
        .query_pairs()
        .find(|(k, _)| k == "ib")
        .map(|(_, v)| v.into_owned());
    assert_eq!(ib, s("integration-messenger"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:158 "getChannelIntegrationId resolves the smtp integration id for the me link"
#[tokio::test]
async fn getchannelintegrationid_resolves_the_smtp_integration_id_for_the_me_link() {
    let h = harness_for("smtp");
    let link = h
        .v()
        .get_integration_field(
            &contact(),
            SystemFieldType::Me,
            Some(&contact_inbox("smtp")),
            None,
        )
        .await
        .unwrap()
        .expect(
            "getChannelIntegrationId has no \"smtp\" case, so the me link cannot be built for it",
        );
    let url = url::Url::parse(&link).unwrap();
    let ib = url
        .query_pairs()
        .find(|(k, _)| k == "ib")
        .map(|(_, v)| v.into_owned());
    assert_eq!(ib, s("integration-smtp"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:158 "getChannelIntegrationId resolves the telegram integration id for the me link"
#[tokio::test]
async fn getchannelintegrationid_resolves_the_telegram_integration_id_for_the_me_link() {
    let h = harness_for("telegram");
    let link = h
        .v()
        .get_integration_field(&contact(), SystemFieldType::Me, Some(&contact_inbox("telegram")), None)
        .await
        .unwrap()
        .expect("getChannelIntegrationId has no \"telegram\" case, so the me link cannot be built for it");
    let url = url::Url::parse(&link).unwrap();
    let ib = url
        .query_pairs()
        .find(|(k, _)| k == "ib")
        .map(|(_, v)| v.into_owned());
    assert_eq!(ib, s("integration-telegram"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:158 "getChannelIntegrationId resolves the threads integration id for the me link"
#[tokio::test]
async fn getchannelintegrationid_resolves_the_threads_integration_id_for_the_me_link() {
    let h = harness_for("threads");
    let link = h
        .v()
        .get_integration_field(&contact(), SystemFieldType::Me, Some(&contact_inbox("threads")), None)
        .await
        .unwrap()
        .expect("getChannelIntegrationId has no \"threads\" case, so the me link cannot be built for it");
    let url = url::Url::parse(&link).unwrap();
    let ib = url
        .query_pairs()
        .find(|(k, _)| k == "ib")
        .map(|(_, v)| v.into_owned());
    assert_eq!(ib, s("integration-threads"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:158 "getChannelIntegrationId resolves the tiktok integration id for the me link"
#[tokio::test]
async fn getchannelintegrationid_resolves_the_tiktok_integration_id_for_the_me_link() {
    let h = harness_for("tiktok");
    let link = h
        .v()
        .get_integration_field(
            &contact(),
            SystemFieldType::Me,
            Some(&contact_inbox("tiktok")),
            None,
        )
        .await
        .unwrap()
        .expect(
            "getChannelIntegrationId has no \"tiktok\" case, so the me link cannot be built for it",
        );
    let url = url::Url::parse(&link).unwrap();
    let ib = url
        .query_pairs()
        .find(|(k, _)| k == "ib")
        .map(|(_, v)| v.into_owned());
    assert_eq!(ib, s("integration-tiktok"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:158 "getChannelIntegrationId resolves the webchat integration id for the me link"
#[tokio::test]
async fn getchannelintegrationid_resolves_the_webchat_integration_id_for_the_me_link() {
    let h = harness_for("webchat");
    let link = h
        .v()
        .get_integration_field(&contact(), SystemFieldType::Me, Some(&contact_inbox("webchat")), None)
        .await
        .unwrap()
        .expect("getChannelIntegrationId has no \"webchat\" case, so the me link cannot be built for it");
    let url = url::Url::parse(&link).unwrap();
    let ib = url
        .query_pairs()
        .find(|(k, _)| k == "ib")
        .map(|(_, v)| v.into_owned());
    assert_eq!(ib, s("integration-webchat"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:158 "getChannelIntegrationId resolves the whatsapp integration id for the me link"
#[tokio::test]
async fn getchannelintegrationid_resolves_the_whatsapp_integration_id_for_the_me_link() {
    let h = harness_for("whatsapp");
    let link = h
        .v()
        .get_integration_field(&contact(), SystemFieldType::Me, Some(&contact_inbox("whatsapp")), None)
        .await
        .unwrap()
        .expect("getChannelIntegrationId has no \"whatsapp\" case, so the me link cannot be built for it");
    let url = url::Url::parse(&link).unwrap();
    let ib = url
        .query_pairs()
        .find(|(k, _)| k == "ib")
        .map(|(_, v)| v.into_owned());
    assert_eq!(ib, s("integration-whatsapp"));
}

/// Port of: packages/variables/__tests__/integration-fields-channel-coverage.test.ts:158 "getChannelIntegrationId resolves the zalo integration id for the me link"
#[tokio::test]
async fn getchannelintegrationid_resolves_the_zalo_integration_id_for_the_me_link() {
    let h = harness_for("zalo");
    let link = h
        .v()
        .get_integration_field(
            &contact(),
            SystemFieldType::Me,
            Some(&contact_inbox("zalo")),
            None,
        )
        .await
        .unwrap()
        .expect(
            "getChannelIntegrationId has no \"zalo\" case, so the me link cannot be built for it",
        );
    let url = url::Url::parse(&link).unwrap();
    let ib = url
        .query_pairs()
        .find(|(k, _)| k == "ib")
        .map(|(_, v)| v.into_owned());
    assert_eq!(ib, s("integration-zalo"));
}
