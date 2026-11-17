//! Messenger page menus shared by the API (settings save) and the worker
//! (user menu steps): `buildBrandingUrl(appUrl, "messenger", ...)`,
//! `moveBrandingMenuLast` and `messengerMenusToCallToActions`.

use chatbotx_core::flow_config::util::{ButtonPayload, encode_button_payload};
use serde_json::{Value, json};

use crate::keys::Edition;

/// `buildBrandingUrl(appUrl, "messenger", isCommunity)`.
pub fn branding_url(app_url: &str) -> String {
    branding_url_for(app_url, "messenger")
}

/// `buildBrandingUrl(appUrl, channel, isCommunity)`.
pub fn branding_url_for(app_url: &str, channel: &str) -> String {
    let edition = if Edition::from_env().is_community() {
        "selfhosted"
    } else {
        "cloud"
    };
    match url::Url::parse(app_url) {
        Ok(mut url) => {
            let pairs: Vec<(String, String)> = url
                .query_pairs()
                .filter(|(k, _)| k != "ref" && k != "channel")
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            url.query_pairs_mut()
                .clear()
                .extend_pairs(&pairs)
                .append_pair("ref", edition)
                .append_pair("channel", channel);
            url.to_string()
        }
        Err(_) => app_url.to_owned(),
    }
}

/// `moveBrandingMenuLast` + `messengerMenusToCallToActions`.
pub fn call_to_actions(menus: &Value, branding_url: &str) -> Vec<Value> {
    let mut menus: Vec<&Value> = menus.as_array().into_iter().flatten().collect();
    let is_branding = |m: &&Value| {
        m.get("type").and_then(Value::as_str) == Some("url")
            && m.get("url").and_then(Value::as_str) == Some(branding_url)
    };
    if let Some(index) = menus.iter().position(is_branding) {
        let branding = menus.remove(index);
        menus.push(branding);
    }
    menus
        .into_iter()
        .filter_map(|menu| {
            let label = menu.get("label").cloned().unwrap_or(Value::Null);
            match menu.get("type").and_then(Value::as_str) {
                Some("flow") => Some(json!({ "type": "postback", "title": label,
                    "payload": encode_button_payload(&ButtonPayload {
                        flow_id: menu.get("flowId").and_then(Value::as_str).unwrap_or("").to_owned(),
                        ..ButtonPayload::default()
                    }) })),
                Some("url") => Some(json!({ "type": "web_url", "title": label, "url": menu.get("url") })),
                _ => None,
            }
        })
        .collect()
}
