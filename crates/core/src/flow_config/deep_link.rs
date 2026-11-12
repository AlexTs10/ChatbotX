//! Port of `packages/flow-config/src/deep-link.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const DEEP_LINK_APP_IDS: &[&str] = &["zaloGroup", "zalo", "messenger"];

// HAND-PORT DEEP_LINK_APPS (object): [{"id":"zaloGroup","hosts":["zalo.me","chat.zalo.me"],"pathPattern":{},"appName":"Zalo","channel":"zalo","android":{"package":"com.zing.zalo"},"ios":{"appStoreId":"579523206"}},{"id":"zalo","hosts":["zalo.me","chat.zalo.me"],"appName":"Zalo","channel":"zalo","android":{"package":"com.zing.zalo"},"ios":{"appStoreId":"579523206"}},{"id":"messenger","hosts":["m.me"],"appName":"Messenger","android":{"package":"com.facebook.orca"},"ios":{"appStoreId":"454638411","scheme":"<fn scheme>"}}]

// HAND-PORT matchDeepLinkApp:
// url=>{let parsed;try{parsed=new URL(url)}catch{return}if(parsed.protocol!=="http:"&&parsed.protocol!=="https:"){return}const hostname=normalizeDeepLinkHost(parsed.hostname);return DEEP_LINK_APPS.find(app=>app.hosts.includes(hostname)&&(app.pathPattern?app.pathPattern.test(parsed.pathname):true))}

// HAND-PORT resolveDeepLinkNavigation:
// (targets,userAgent)=>ANDROID_UA.test(userAgent)?{href:targets.androidIntentUrl,auto:true}:{href:targets.webUrl,auto:false}

// HAND-PORT buildAppLinkTargets:
// (url,app)=>{const parsed=new URL(url);const scheme=parsed.protocol.slice(0,-1);const host=normalizeDeepLinkHost(parsed.host);const schemeless=`${host}${parsed.pathname}${parsed.search}`;const iosUrl=app.ios?.scheme?.(parsed);return{androidPackage:app.android.package,androidIntentUrl:`intent://${schemeless}#Intent;scheme=${scheme};package=${app.android.package};S.browser_fallback_url=${encodeURIComponent(url)};end`,iosUrl,iosAppStoreId:iosUrl?app.ios?.appStoreId:void 0,webUrl:url}}

// ---- stage 1 API (hand-ported in stage 2) ----

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

pub fn build_app_link_targets(url: &str, app: &DeepLinkApp) -> AppLinkTargets {
    todo!("P3.4")
}

pub fn deep_link_apps() -> &'static [DeepLinkApp] {
    todo!("P3.4")
}

pub fn match_deep_link_app(url: &str) -> Option<&'static DeepLinkApp> {
    todo!("P3.4")
}
