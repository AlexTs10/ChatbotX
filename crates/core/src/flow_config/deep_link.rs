//! Port of `packages/flow-config/src/deep-link.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const DEEP_LINK_APP_IDS: &[&str] = &["zaloGroup", "zalo", "messenger"];

/// `DeepLinkApp`.
#[derive(Debug, Clone)]
pub struct DeepLinkApp {
    pub id: &'static str,
    pub hosts: &'static [&'static str],
    pub path_pattern: Option<&'static str>,
    pub app_name: &'static str,
    pub channel: Option<&'static str>,
    pub android: DeepLinkAndroid,
    pub ios: Option<DeepLinkIos>,
}

/// `DeepLinkApp.android`.
#[derive(Debug, Clone)]
pub struct DeepLinkAndroid {
    pub package: &'static str,
}

/// `DeepLinkApp.ios`.
#[derive(Debug, Clone)]
pub struct DeepLinkIos {
    pub app_store_id: &'static str,
    pub scheme: Option<fn(&url::Url) -> Option<String>>,
}

/// `AppLinkTargets`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppLinkTargets {
    pub android_package: String,
    pub android_intent_url: String,
    pub ios_url: Option<String>,
    pub ios_app_store_id: Option<String>,
    pub web_url: String,
}

fn messenger_thread_id(url: &url::Url) -> Option<String> {
    let segments: Vec<&str> = url.path().split('/').filter(|s| !s.is_empty()).collect();
    (segments.len() == 1).then(|| segments[0].to_owned())
}

fn messenger_scheme(url: &url::Url) -> Option<String> {
    messenger_thread_id(url).map(|page| format!("fb-messenger://user-thread/{page}"))
}

/// `DEEP_LINK_APPS`.
pub fn deep_link_apps() -> &'static [DeepLinkApp] {
    static V: LazyLock<Vec<DeepLinkApp>> = LazyLock::new(|| {
        vec![
            DeepLinkApp {
                id: "zaloGroup",
                hosts: &["zalo.me", "chat.zalo.me"],
                path_pattern: Some("/g/"),
                app_name: "Zalo",
                channel: Some("zalo"),
                android: DeepLinkAndroid {
                    package: "com.zing.zalo",
                },
                ios: Some(DeepLinkIos {
                    app_store_id: "579523206",
                    scheme: None,
                }),
            },
            DeepLinkApp {
                id: "zalo",
                hosts: &["zalo.me", "chat.zalo.me"],
                path_pattern: None,
                app_name: "Zalo",
                channel: Some("zalo"),
                android: DeepLinkAndroid {
                    package: "com.zing.zalo",
                },
                ios: Some(DeepLinkIos {
                    app_store_id: "579523206",
                    scheme: None,
                }),
            },
            DeepLinkApp {
                id: "messenger",
                hosts: &["m.me"],
                path_pattern: None,
                app_name: "Messenger",
                channel: None,
                android: DeepLinkAndroid {
                    package: "com.facebook.orca",
                },
                ios: Some(DeepLinkIos {
                    app_store_id: "454638411",
                    scheme: Some(messenger_scheme),
                }),
            },
        ]
    });
    &V
}

fn normalize_deep_link_host(host: &str) -> String {
    let lower = host.to_lowercase();
    lower
        .strip_prefix("www.")
        .map(str::to_owned)
        .unwrap_or(lower)
}

/// `matchDeepLinkApp(url)`: the app rule for an http(s) URL (`path_pattern`
/// is a path prefix: the only pattern used is `^/g/`).
pub fn match_deep_link_app(url: &str) -> Option<&'static DeepLinkApp> {
    let parsed = url::Url::parse(url).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    let host = normalize_deep_link_host(parsed.host_str()?);
    deep_link_apps().iter().find(|app| {
        app.hosts.contains(&host.as_str())
            && app
                .path_pattern
                .is_none_or(|prefix| parsed.path().starts_with(prefix))
    })
}

/// `buildAppLinkTargets(url, app)`.
pub fn build_app_link_targets(url: &str, app: &DeepLinkApp) -> AppLinkTargets {
    let parsed = url::Url::parse(url).ok();
    let (scheme, host, path, search) = match &parsed {
        Some(u) => {
            let host = match u.port() {
                Some(port) => format!("{}:{port}", u.host_str().unwrap_or("")),
                None => u.host_str().unwrap_or("").to_owned(),
            };
            (
                u.scheme().to_owned(),
                normalize_deep_link_host(&host),
                u.path().to_owned(),
                u.query().map(|q| format!("?{q}")).unwrap_or_default(),
            )
        }
        None => (String::new(), String::new(), String::new(), String::new()),
    };
    let ios_url = match (&app.ios, &parsed) {
        (
            Some(DeepLinkIos {
                scheme: Some(scheme),
                ..
            }),
            Some(u),
        ) => scheme(u),
        _ => None,
    };
    AppLinkTargets {
        android_package: app.android.package.to_owned(),
        android_intent_url: format!(
            "intent://{host}{path}{search}#Intent;scheme={scheme};package={};S.browser_fallback_url={};end",
            app.android.package,
            crate::js::encode_uri_component(url)
        ),
        ios_app_store_id: ios_url
            .as_ref()
            .and(app.ios.as_ref())
            .map(|ios| ios.app_store_id.to_owned()),
        ios_url,
        web_url: url.to_owned(),
    }
}

/// `resolveDeepLinkNavigation(targets, userAgent)`: (href, auto).
pub fn resolve_deep_link_navigation(targets: &AppLinkTargets, user_agent: &str) -> (String, bool) {
    if user_agent.to_lowercase().contains("android") {
        (targets.android_intent_url.clone(), true)
    } else {
        (targets.web_url.clone(), false)
    }
}
