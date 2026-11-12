//! Port of `packages/utils/__tests__/live-media.test.ts`. `vi.stubEnv` is an
//! injected env map.

use chatbotx_core::env::EmptyEnv;
use chatbotx_core::utils::live_media::{
    is_lazy_avatar_channel, is_live_media_deferrable_url, is_live_media_lazy_channel,
    is_url_on_hosts,
};

/// Port of: packages/utils/__tests__/live-media.test.ts:14 "live media laziness flags > defaults to the channels whose provider can serve their media again"
#[test]
fn live_media_laziness_flags_defaults_to_the_channels_whose_provider_can_serve_their_media_again() {
    assert!(is_live_media_lazy_channel(&EmptyEnv, "messenger"));
    assert!(is_live_media_lazy_channel(&EmptyEnv, "instagram"));
    assert!(!is_live_media_lazy_channel(&EmptyEnv, "whatsapp"));
    assert!(!is_live_media_lazy_channel(&EmptyEnv, "telegram"));
    assert!(!is_live_media_lazy_channel(&EmptyEnv, "zalo"));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:22 "live media laziness flags > applies the same default to avatars"
#[test]
fn live_media_laziness_flags_applies_the_same_default_to_avatars() {
    assert!(is_lazy_avatar_channel(&EmptyEnv, "messenger"));
    assert!(is_lazy_avatar_channel(&EmptyEnv, "instagram"));
    assert!(!is_lazy_avatar_channel(&EmptyEnv, "zalo"));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:28 "live media laziness flags > is not affected by a misconfigured image compression setting"
#[test]
fn live_media_laziness_flags_is_not_affected_by_a_misconfigured_image_compression_setting() {
    let env = [("STORED_IMAGE_MEDIA_QUALITY", "0")];
    assert!(is_live_media_lazy_channel(&env, "messenger"));
    assert!(is_lazy_avatar_channel(&env, "instagram"));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:36 "live media laziness flags > an empty list turns lazy ingestion off for every channel"
#[test]
fn live_media_laziness_flags_an_empty_list_turns_lazy_ingestion_off_for_every_channel() {
    let env = [
        ("LIVE_MEDIA_LAZY_CHANNELS", ""),
        ("LIVE_AVATAR_LAZY_CHANNELS", ""),
    ];
    assert!(!is_live_media_lazy_channel(&env, "messenger"));
    assert!(!is_lazy_avatar_channel(&env, "instagram"));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:44 "live media laziness flags > parses a comma separated list, ignoring spaces and case"
#[test]
fn live_media_laziness_flags_parses_a_comma_separated_list_ignoring_spaces_and_case() {
    let env = [("LIVE_MEDIA_LAZY_CHANNELS", " Messenger , telegram ")];
    assert!(is_live_media_lazy_channel(&env, "messenger"));
    assert!(is_live_media_lazy_channel(&env, "telegram"));
    assert!(!is_live_media_lazy_channel(&env, "instagram"));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:52 "live media laziness flags > the media and avatar flags are independent"
#[test]
fn live_media_laziness_flags_the_media_and_avatar_flags_are_independent() {
    let env = [("LIVE_AVATAR_LAZY_CHANNELS", "")];
    assert!(is_live_media_lazy_channel(&env, "messenger"));
    assert!(!is_lazy_avatar_channel(&env, "messenger"));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:63 "isUrlOnHosts > accepts https://scontent.xx.fbcdn.net/v/t1/a.jpg"
#[test]
fn isurlonhosts_accepts_https_scontent_xx_fbcdn_net_v_t1_a_jpg() {
    assert!(is_url_on_hosts(
        "https://scontent.xx.fbcdn.net/v/t1/a.jpg",
        Some(&["fbcdn.net", "fbsbx.com"][..])
    ));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:63 "isUrlOnHosts > accepts https://fbcdn.net/a.jpg"
#[test]
fn isurlonhosts_accepts_https_fbcdn_net_a_jpg() {
    assert!(is_url_on_hosts(
        "https://fbcdn.net/a.jpg",
        Some(&["fbcdn.net", "fbsbx.com"][..])
    ));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:63 "isUrlOnHosts > accepts https://lookaside.fbsbx.com/ig_messaging_cdn/?asset_id=1"
#[test]
fn isurlonhosts_accepts_https_lookaside_fbsbx_com_ig_messaging_cdn_asset_id_1() {
    assert!(is_url_on_hosts(
        "https://lookaside.fbsbx.com/ig_messaging_cdn/?asset_id=1",
        Some(&["fbcdn.net", "fbsbx.com"][..])
    ));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:63 "isUrlOnHosts > accepts HTTPS://SCONTENT.FBCDN.NET/a.jpg"
#[test]
fn isurlonhosts_accepts_https_scontent_fbcdn_net_a_jpg() {
    assert!(is_url_on_hosts(
        "HTTPS://SCONTENT.FBCDN.NET/a.jpg",
        Some(&["fbcdn.net", "fbsbx.com"][..])
    ));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:72 "isUrlOnHosts > rejects http://scontent.xx.fbcdn.net/a.jpg"
#[test]
fn isurlonhosts_rejects_http_scontent_xx_fbcdn_net_a_jpg() {
    assert!(!is_url_on_hosts(
        "http://scontent.xx.fbcdn.net/a.jpg",
        Some(&["fbcdn.net", "fbsbx.com"][..])
    ));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:72 "isUrlOnHosts > rejects https://fbcdn.net.evil.example/a.jpg"
#[test]
fn isurlonhosts_rejects_https_fbcdn_net_evil_example_a_jpg() {
    assert!(!is_url_on_hosts(
        "https://fbcdn.net.evil.example/a.jpg",
        Some(&["fbcdn.net", "fbsbx.com"][..])
    ));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:72 "isUrlOnHosts > rejects https://evilfbcdn.net/a.jpg"
#[test]
fn isurlonhosts_rejects_https_evilfbcdn_net_a_jpg() {
    assert!(!is_url_on_hosts(
        "https://evilfbcdn.net/a.jpg",
        Some(&["fbcdn.net", "fbsbx.com"][..])
    ));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:72 "isUrlOnHosts > rejects https://fbcdn.net@evil.example/a.jpg"
#[test]
fn isurlonhosts_rejects_https_fbcdn_net_evil_example_a_jpg_2() {
    assert!(!is_url_on_hosts(
        "https://fbcdn.net@evil.example/a.jpg",
        Some(&["fbcdn.net", "fbsbx.com"][..])
    ));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:72 "isUrlOnHosts > rejects https://media.giphy.com/media/x/giphy.gif"
#[test]
fn isurlonhosts_rejects_https_media_giphy_com_media_x_giphy_gif() {
    assert!(!is_url_on_hosts(
        "https://media.giphy.com/media/x/giphy.gif",
        Some(&["fbcdn.net", "fbsbx.com"][..])
    ));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:72 "isUrlOnHosts > rejects not a url"
#[test]
fn isurlonhosts_rejects_not_a_url() {
    assert!(!is_url_on_hosts(
        "not a url",
        Some(&["fbcdn.net", "fbsbx.com"][..])
    ));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:72 "isUrlOnHosts > rejects "
#[test]
fn isurlonhosts_rejects() {
    assert!(!is_url_on_hosts("", Some(&["fbcdn.net", "fbsbx.com"][..])));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:84 "isUrlOnHosts > rejects everything when there are no hosts"
#[test]
fn isurlonhosts_rejects_everything_when_there_are_no_hosts() {
    assert!(!is_url_on_hosts(
        "https://scontent.xx.fbcdn.net/a.jpg",
        None
    ));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:96 "isLiveMediaDeferrableUrl > defers a Meta CDN URL on a lazy channel"
#[test]
fn islivemediadeferrableurl_defers_a_meta_cdn_url_on_a_lazy_channel() {
    assert!(is_live_media_deferrable_url(
        &EmptyEnv,
        "messenger",
        "https://scontent.xx.fbcdn.net/a.jpg"
    ));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:105 "isLiveMediaDeferrableUrl > keeps a third-party URL (a GIF host, a shared link) on the download path"
#[test]
fn islivemediadeferrableurl_keeps_a_third_party_url_a_gif_host_a_shared_link_on_the_download_path()
{
    assert!(!is_live_media_deferrable_url(
        &EmptyEnv,
        "messenger",
        "https://media.giphy.com/x.gif"
    ));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:111 "isLiveMediaDeferrableUrl > keeps a non-lazy channel on the download path"
#[test]
fn islivemediadeferrableurl_keeps_a_non_lazy_channel_on_the_download_path() {
    assert!(!is_live_media_deferrable_url(
        &EmptyEnv,
        "telegram",
        "https://scontent.xx.fbcdn.net/a.jpg"
    ));
}

/// Port of: packages/utils/__tests__/live-media.test.ts:120 "isLiveMediaDeferrableUrl > keeps everything on the download path when the flag is emptied"
#[test]
fn islivemediadeferrableurl_keeps_everything_on_the_download_path_when_the_flag_is_emptied() {
    let env = [("LIVE_MEDIA_LAZY_CHANNELS", "")];
    assert!(!is_live_media_deferrable_url(
        &env,
        "messenger",
        "https://scontent.xx.fbcdn.net/a.jpg"
    ));
}
