//! `chat/utils/mini-app-buttons.ts` (Q.D1m): an "Open Mini App" flow button
//! becomes an `openWebsite` button whose link opens the Mini App for this
//! contact, with a signed `resume` token so the submit continues the flow.
//!
//! ponytail: `messenger-template-mini-app.ts` (the template's resume suffix)
//! waits for the Messenger template send step, which is not ported.

use std::collections::HashSet;

use chatbotx_core::encryption::{MINI_APP_TOKEN_TTL_MS, MiniAppClaims, MiniAppResumeClaims};
use chatbotx_core::flow_config::util::{ButtonPayload, encode_button_payload, extract_metadata};
use serde_json::{Value, json};

use crate::Deps;

/// `MiniAppLinkContext`: the flow send the Mini App is opened from.
pub struct LinkContext<'a> {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub conversation_id: i64,
    pub contact_inbox_id: i64,
    pub flow_id: &'a str,
    pub flow_version_id: Option<&'a str>,
    pub metadata: Option<&'a Value>,
    pub app_url: &'a str,
}

fn is_open_mini_app(button: &Value) -> bool {
    button.get("buttonType").and_then(Value::as_str) == Some("openMiniApp")
}

fn mini_app_id(button: &Value) -> Option<&str> {
    button
        .pointer("/beforeStep/miniAppId")
        .and_then(Value::as_str)
}

/// `readStepButtons` (own and cards') plus the quick replies, mutably.
fn button_lists<'a>(
    step: &'a mut Value,
    quick_replies: Option<&'a mut Value>,
) -> Vec<&'a mut Vec<Value>> {
    let mut lists = Vec::new();
    let Value::Object(step) = step else {
        return lists;
    };
    for (key, value) in step.iter_mut() {
        match (key.as_str(), value) {
            ("buttons", Value::Array(buttons)) => lists.push(buttons),
            ("cards", Value::Array(cards)) => {
                for card in cards {
                    if let Some(Value::Array(buttons)) = card.get_mut("buttons") {
                        lists.push(buttons);
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(Value::Array(replies)) = quick_replies {
        lists.push(replies);
    }
    lists
}

/// The Mini App ids the step's Open Mini App buttons name.
pub fn mini_app_ids(step: &Value, quick_replies: Option<&Value>) -> Vec<String> {
    let own = step
        .get("buttons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let cards = step
        .get("cards")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|card| {
            card.get("buttons")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        });
    let replies = quick_replies
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    own.chain(cards)
        .chain(replies)
        .filter(|b| is_open_mini_app(b))
        .filter_map(|b| mini_app_id(b).map(str::to_owned))
        .collect()
}

/// The `buttonPayload` of the resume token: built like the payload
/// `convertButtonsToTemplate` puts on a tapped button.
pub fn resume_button_payload(ctx: &LinkContext<'_>, button_id: &str) -> String {
    encode_button_payload(&ButtonPayload {
        flow_id: ctx.flow_id.to_owned(),
        flow_version_id: ctx.flow_version_id.map(str::to_owned),
        button_id: Some(button_id.to_owned()),
        broadcast_id: extract_metadata("broadcastId", ctx.metadata),
        sequence_step_id: extract_metadata("sequenceStepId", ctx.metadata),
        contact_inbox_id: Some(ctx.contact_inbox_id.to_string()),
        comment_automation_id: extract_metadata("commentAutomationId", ctx.metadata),
        ..ButtonPayload::default()
    })
}

/// `toMiniAppLinkButton` / `resolveButtons`: every Open Mini App button in
/// `step` and `quick_replies` becomes a link button, or is dropped when its
/// Mini App is not in `sendable`. `sign_resume(button_id, mini_app_id)`
/// signs the resume token (`None` drops the button).
pub fn apply_mini_app_links(
    ctx: &LinkContext<'_>,
    step: &mut Value,
    quick_replies: Option<&mut Value>,
    sendable: &HashSet<String>,
    contact_token: &str,
    sign_resume: &mut dyn FnMut(&str, &str) -> Option<String>,
) {
    for buttons in button_lists(step, quick_replies) {
        let mut out = Vec::with_capacity(buttons.len());
        for button in buttons.drain(..) {
            if !is_open_mini_app(&button) {
                out.push(button);
                continue;
            }
            let button_id = button
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let target = mini_app_id(&button).unwrap_or("").to_owned();
            if !sendable.contains(&target) {
                tracing::warn!(
                    workspace_id = ctx.workspace_id,
                    flow_id = ctx.flow_id,
                    button_id,
                    mini_app_id = target,
                    "Dropping an Open Mini App button whose Mini App cannot be sent"
                );
                continue;
            }
            let Some(resume) = sign_resume(&button_id, &target) else {
                continue;
            };
            let Ok(mut url) = url::Url::parse(ctx.app_url).and_then(|base| base.join("/mini-apps"))
            else {
                continue;
            };
            url.query_pairs_mut()
                .append_pair("id", &target)
                .append_pair("token", contact_token)
                .append_pair("resume", &resume);
            out.push(json!({
                "id": button_id,
                "label": button.get("label").cloned().unwrap_or(Value::Null),
                "buttonType": "openWebsite",
                "beforeStep": {
                    "id": chatbotx_core::utils::id::create_id(),
                    "stepType": "openWebsite",
                    "url": url.to_string(),
                    "browserSize": 100,
                },
                "steps": button.get("steps").cloned().unwrap_or_else(|| json!([])),
            }));
        }
        *buttons = out;
    }
}

/// `findSendableMiniAppIds`: the ids, among `ids`, of enabled Mini Apps of
/// this workspace (a copied flow keeps another workspace's ids).
async fn sendable_ids(deps: &Deps, workspace_id: i64, ids: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for id in ids.iter().collect::<HashSet<_>>() {
        let Ok(parsed) = id.parse::<i64>() else {
            continue;
        };
        if let Ok(Some(app)) = chatbotx_business::mini_app::find_unscoped(&deps.db, parsed).await
            && app.workspace_id == workspace_id
            && app.enabled
        {
            out.insert(id.clone());
        }
    }
    out
}

/// `resolveMiniAppLinks`: does nothing (no lookup, no signing) when no
/// button opens a Mini App. Without `ENCRYPTION_KEY` no link can be signed,
/// so the buttons are dropped.
pub async fn resolve_mini_app_links(
    deps: &Deps,
    ctx: &LinkContext<'_>,
    step: &mut Value,
    quick_replies: Option<&mut Value>,
) {
    let ids = mini_app_ids(step, quick_replies.as_deref());
    if ids.is_empty() {
        return;
    }
    let sendable = sendable_ids(deps, ctx.workspace_id, &ids).await;
    let encryption = deps.variables.as_ref().map(|v| &v.encryption);
    let contact_token = encryption.and_then(|e| {
        let claims = MiniAppClaims {
            workspace_id: ctx.workspace_id.to_string(),
            contact_id: ctx.contact_id.to_string(),
        };
        e.sign_mini_app_token(&claims, MINI_APP_TOKEN_TTL_MS)
            .inspect_err(|err| tracing::error!(err = %err, "Unable to sign the Mini App token"))
            .ok()
    });
    let sendable = if contact_token.is_some() {
        sendable
    } else {
        HashSet::new()
    };
    let mut sign = |button_id: &str, mini_app_id: &str| {
        let claims = MiniAppResumeClaims {
            workspace_id: ctx.workspace_id.to_string(),
            contact_id: ctx.contact_id.to_string(),
            mini_app_id: mini_app_id.to_owned(),
            conversation_id: ctx.conversation_id.to_string(),
            contact_inbox_id: ctx.contact_inbox_id.to_string(),
            button_payload: resume_button_payload(ctx, button_id),
        };
        encryption?
            .sign_mini_app_resume_token(&claims, MINI_APP_TOKEN_TTL_MS)
            .inspect_err(
                |err| tracing::error!(err = %err, "Unable to sign the Mini App resume token"),
            )
            .ok()
    };
    apply_mini_app_links(
        ctx,
        step,
        quick_replies,
        &sendable,
        contact_token.as_deref().unwrap_or(""),
        &mut sign,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTX: LinkContext<'static> = LinkContext {
        workspace_id: 1,
        contact_id: 10,
        conversation_id: 20,
        contact_inbox_id: 30,
        flow_id: "40",
        flow_version_id: Some("50"),
        metadata: None,
        app_url: "https://app.example.com",
    };

    fn mini_app_button(id: &str, mini_app_id: &str) -> Value {
        json!({ "id": id, "label": "Open form", "buttonType": "openMiniApp",
            "beforeStep": { "id": format!("{id}1"), "stepType": "openMiniApp", "miniAppId": mini_app_id }, "steps": [] })
    }

    fn website_button() -> Value {
        json!({ "id": "200", "label": "Go", "buttonType": "openWebsite",
            "beforeStep": { "id": "201", "stepType": "openWebsite", "url": "https://example.com", "browserSize": 100 }, "steps": [] })
    }

    fn run(
        step: &mut Value,
        quick_replies: Option<&mut Value>,
        sendable: &[&str],
    ) -> Vec<(String, String)> {
        let sendable = sendable.iter().map(|s| (*s).to_owned()).collect();
        let mut calls = Vec::new();
        apply_mini_app_links(
            &CTX,
            step,
            quick_replies,
            &sendable,
            "contact-token",
            &mut |b, m| {
                calls.push((b.to_owned(), m.to_owned()));
                Some("resume-token".into())
            },
        );
        calls
    }

    #[test]
    fn turns_an_open_mini_app_button_into_a_link_to_the_mini_app() {
        let mut step = json!({ "id": "300", "stepType": "sendText", "text": "Hi", "buttons": [mini_app_button("100", "7")] });
        run(&mut step, None, &["7"]);
        let button = &step["buttons"][0];
        assert_eq!(button["id"], "100");
        assert_eq!(button["buttonType"], "openWebsite");
        let url = url::Url::parse(button["beforeStep"]["url"].as_str().unwrap()).unwrap();
        assert_eq!(
            format!("{}{}", url.origin().ascii_serialization(), url.path()),
            "https://app.example.com/mini-apps"
        );
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["id"], "7");
        assert_eq!(query["token"], "contact-token");
        assert_eq!(query["resume"], "resume-token");
    }

    #[test]
    fn signs_the_resume_token_for_this_contact_conversation_and_button() {
        assert!(resume_button_payload(&CTX, "100").contains("40:50:100"));
        let mut step = json!({ "buttons": [mini_app_button("100", "7")] });
        assert_eq!(
            run(&mut step, None, &["7"]),
            [("100".to_owned(), "7".to_owned())]
        );
    }

    #[test]
    fn signs_every_button_cards_and_quick_replies_included() {
        let mut step = json!({ "id": "300", "stepType": "sendCarousel", "cards": [
            { "title": "A", "buttons": [mini_app_button("101", "7")] },
            { "title": "B", "buttons": [mini_app_button("102", "7")] } ] });
        let mut replies = json!([mini_app_button("103", "7")]);
        assert_eq!(mini_app_ids(&step, Some(&replies)).len(), 3);
        assert_eq!(run(&mut step, Some(&mut replies), &["7"]).len(), 3);
        assert_eq!(step["cards"][0]["buttons"][0]["buttonType"], "openWebsite");
        assert_eq!(step["cards"][1]["buttons"][0]["buttonType"], "openWebsite");
        assert_eq!(replies[0]["buttonType"], "openWebsite");
    }

    #[test]
    fn does_nothing_when_no_button_opens_a_mini_app() {
        let step = json!({ "buttons": [website_button()] });
        assert!(mini_app_ids(&step, Some(&json!([website_button()]))).is_empty());
    }

    #[test]
    fn drops_a_button_whose_mini_app_cannot_be_sent() {
        let mut step = json!({ "buttons": [mini_app_button("100", "7"), website_button()] });
        assert!(run(&mut step, None, &[]).is_empty());
        assert_eq!(step["buttons"], json!([website_button()]));
    }
}
