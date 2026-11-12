//! Port of `packages/flow-config/__tests__/deep-link.test.ts`.

mod flow_config_support;
use flow_config_support::*;

use fc::{DeepLinkApp, DeepLinkIos};

fn id(url: &str) -> Option<&'static str> {
    fc::match_deep_link_app(url).map(|app| app.id)
}

#[test]
fn matchdeeplinkapp_a_zalo_group_invite_resolves_to_the_group_rule_not_the_generic_one() {
    // Table order is the only thing separating these two, and getting it wrong
    // silently relabels every group invite.
    assert_eq!(id("https://zalo.me/g/owfqvp123"), Some("zaloGroup"));
    assert_eq!(id("https://www.zalo.me/g/owfqvp123"), Some("zaloGroup"));
    assert_eq!(id("https://zalo.me/pagename"), Some("zalo"));
}

#[test]
fn matchdeeplinkapp_the_table_holds_zalo_and_m_me_and_nothing_else() {
    // Every other destination keeps going straight to where it always went.
    let ids: Vec<&str> = fc::deep_link_apps().iter().map(|app| app.id).collect();
    assert_eq!(ids, vec!["zaloGroup", "zalo", "messenger"]);
    assert_eq!(id("https://m.me/9679565075442614"), Some("messenger"));
    assert_eq!(id("https://www.facebook.com/share/g/1FhUw8ij8B/"), None);
    assert_eq!(id("https://instagram.com/someone"), None);
    assert_eq!(id("https://t.me/somechannel"), None);
    assert_eq!(id("https://vt.tiktok.com/ZS123/"), None);
    assert_eq!(id("https://chat.whatsapp.com/ABC"), None);
}

#[test]
fn matchdeeplinkapp_m_me_carries_no_channel_so_it_is_not_exempt_on_messenger() {
    // The self-channel exemption assumes a link opened from inside its own app
    // already works. For `m.me` in Messenger's iOS webview that is precisely
    // the assumption that fails, so declaring a channel here would exempt the
    // one case the rule exists for.
    assert_eq!(
        fc::match_deep_link_app("https://m.me/9679565075442614")
            .unwrap()
            .channel,
        None
    );
}

#[test]
fn matchdeeplinkapp_host_matching_is_by_whole_label_not_by_suffix() {
    assert_eq!(id("https://notzalo.me/g/abc"), None);
    assert_eq!(id("https://zalo.me.evil.test/g/abc"), None);
}

#[test]
fn matchdeeplinkapp_unknown_and_non_http_destinations_do_not_match() {
    for url in [
        "https://example.com/page",
        "javascript:alert(1)",
        "data:text/html,<p>x</p>",
        "not a url",
        "",
    ] {
        assert_eq!(id(url), None, "{url}");
    }
}

#[test]
fn matchdeeplinkapp_every_rule_declares_an_app_name_and_an_android_package() {
    for app in fc::deep_link_apps() {
        assert!(!app.app_name.is_empty());
        assert!(!app.android.package.is_empty());
    }
}

const URL: &str = "https://zalo.me/g/owfqvp123?ref=1";

fn app() -> &'static DeepLinkApp {
    fc::match_deep_link_app(URL).unwrap()
}

fn encode_uri_component(text: &str) -> String {
    chatbotx_core::js::encode_uri_component(text)
}

#[test]
fn buildapplinktargets_keeps_the_original_https_url_as_the_web_target() {
    assert_eq!(fc::build_app_link_targets(URL, app()).web_url, URL);
}

#[test]
fn buildapplinktargets_builds_an_android_intent_url_with_an_encoded_browser_fallback() {
    let targets = fc::build_app_link_targets(URL, app());

    assert_eq!(
        targets.android_intent_url,
        format!(
            "intent://zalo.me/g/owfqvp123?ref=1#Intent;scheme=https;package=com.zing.zalo;S.browser_fallback_url={};end",
            encode_uri_component(URL)
        )
    );
}

#[test]
fn buildapplinktargets_normalises_the_intent_host_the_same_way_the_rule_was_matched() {
    let www_url = "https://www.zalo.me/g/owfqvp123";
    let www_app = fc::match_deep_link_app(www_url).unwrap();

    let targets = fc::build_app_link_targets(www_url, www_app);

    assert!(
        targets
            .android_intent_url
            .contains("intent://zalo.me/g/owfqvp123")
    );
    // The fallback is the real web address, so it keeps the host as written.
    assert!(targets.android_intent_url.contains(&format!(
        "S.browser_fallback_url={}",
        encode_uri_component(www_url)
    )));
}

#[test]
fn buildapplinktargets_emits_no_ios_target_for_a_rule_with_no_verified_scheme() {
    // `al:ios:url` must be a custom scheme, never an https URL.
    assert!(app().ios.as_ref().and_then(|ios| ios.scheme).is_none());

    let targets = fc::build_app_link_targets(URL, app());
    assert!(targets.ios_url.is_none());
    // Pointless without its required other half, so it is withheld too.
    assert!(targets.ios_app_store_id.is_none());
}

#[test]
fn buildapplinktargets_maps_an_m_me_thread_to_messenger_s_own_scheme() {
    let m_me = "https://m.me/9679565075442614";
    let messenger = fc::match_deep_link_app(m_me).unwrap();

    let targets = fc::build_app_link_targets(m_me, messenger);

    assert_eq!(
        targets.ios_url.as_deref(),
        Some("fb-messenger://user-thread/9679565075442614")
    );
    assert_eq!(targets.ios_app_store_id.as_deref(), Some("454638411"));
}

#[test]
fn buildapplinktargets_leaves_an_m_me_recurring_notification_link_on_https() {
    let rn = "https://m.me/rn/OriginalCoastClothing?topic=deals";
    let messenger = fc::match_deep_link_app(rn).unwrap();

    let targets = fc::build_app_link_targets(rn, messenger);

    assert!(targets.ios_url.is_none());
    assert!(targets.ios_app_store_id.is_none());
}

#[test]
fn buildapplinktargets_carries_the_ios_pair_once_a_rule_declares_a_scheme() {
    let custom = DeepLinkApp {
        ios: Some(DeepLinkIos {
            app_store_id: "579523206",
            scheme: Some(|u: &url::Url| Some(format!("zalo:/{}", u.path()))),
        }),
        ..app().clone()
    };
    let targets = fc::build_app_link_targets(URL, &custom);

    assert_eq!(targets.ios_url.as_deref(), Some("zalo://g/owfqvp123"));
    assert_eq!(targets.ios_app_store_id.as_deref(), Some("579523206"));
}
