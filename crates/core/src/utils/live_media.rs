//! `packages/utils/src/live-media.ts`: which channels keep the provider's
//! media / avatar URL instead of downloading it.

use std::collections::HashSet;

use crate::env::Env;

const DEFAULT_LAZY_CHANNELS: &str = "messenger,instagram";

fn parse_channels(value: &str) -> HashSet<String> {
    value
        .split(',')
        .map(|c| crate::js::trim(c).to_lowercase())
        .filter(|c| !c.is_empty())
        .collect()
}

fn lazy(env: &dyn Env, name: &str, channel: &str) -> bool {
    let value = env
        .var(name)
        .unwrap_or_else(|| DEFAULT_LAZY_CHANNELS.to_owned());
    parse_channels(&value).contains(&channel.to_lowercase())
}

/// `isLiveMediaLazyChannel` (`LIVE_MEDIA_LAZY_CHANNELS`, read per call).
pub fn is_live_media_lazy_channel(env: &dyn Env, channel: &str) -> bool {
    lazy(env, "LIVE_MEDIA_LAZY_CHANNELS", channel)
}

/// `isLazyAvatarChannel` (`LIVE_AVATAR_LAZY_CHANNELS`).
pub fn is_lazy_avatar_channel(env: &dyn Env, channel: &str) -> bool {
    lazy(env, "LIVE_AVATAR_LAZY_CHANNELS", channel)
}

/// `META_MEDIA_HOSTS`.
pub const META_MEDIA_HOSTS: &[&str] =
    &["fbcdn.net", "fbsbx.com", "cdninstagram.com", "facebook.com"];

/// `isUrlOnHosts`: https, and the host (or a subdomain) is listed.
pub fn is_url_on_hosts(url: &str, hosts: Option<&[&str]>) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let Some(hostname) = parsed.host_str() else {
        return false;
    };
    parsed.scheme() == "https"
        && hosts
            .unwrap_or(&[])
            .iter()
            .any(|host| hostname == *host || hostname.ends_with(&format!(".{host}")))
}

/// `isLiveMediaDeferrableUrl`: a lazy channel and a Meta CDN URL.
pub fn is_live_media_deferrable_url(env: &dyn Env, channel: &str, url: &str) -> bool {
    is_live_media_lazy_channel(env, channel) && is_url_on_hosts(url, Some(META_MEDIA_HOSTS))
}
