//! The Messenger user steps of `integration/handlers/messenger-user-menu.ts`
//! (Q.I1b): `setMessengerUserPersistentMenu`, `enableMessengerComposer`,
//! `disableMessengerComposer` (Graph `me/custom_user_settings`) and
//! `setMessengerPersona`.
//!
//! ponytail: `updateMessengerContactData` needs the Messenger profile
//! lookup (not ported, specs/Q.I1b.md "Left"); it stays an implicit
//! success, like the original's silent no-op on a Graph error.

use chatbotx_business::keys::Edition;
use chatbotx_core::flow_config::util::{ButtonPayload, encode_button_payload};
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::models::ContactInbox;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::JobError;
use serde_json::{Value, json};

use super::StepCtx;
use crate::channel::{Channel, ChannelCtx};
use crate::flow::StepStatus;
use crate::{db_fail, fail};

/// The Messenger user steps; `None` = not one of them.
pub(super) async fn run(
    ctx: &StepCtx<'_, '_>,
    step_type: &str,
) -> Result<Option<StepStatus>, JobError> {
    match step_type {
        "setMessengerUserPersistentMenu"
        | "enableMessengerComposer"
        | "disableMessengerComposer"
        | "setMessengerPersona" => {}
        _ => return Ok(None),
    }
    let Some(user) = resolve_user(ctx).await? else {
        return Ok(Some(StepStatus::Success));
    };
    match step_type {
        "setMessengerPersona" => set_persona(ctx, &user).await?,
        "setMessengerUserPersistentMenu" => {
            let menus = match ctx.str("userPersistentMenuId").filter(|id| !id.is_empty()) {
                Some(id) => user_menu(ctx, id).await?,
                None => user.persistent_menus.clone(),
            };
            let ctas = call_to_actions(&menus, &branding_url(&ctx.run.deps.app_url));
            apply(&user, Some(ctas), None, &ctx.run.deps.app_url).await?;
        }
        "enableMessengerComposer" => apply(&user, None, Some(false), &ctx.run.deps.app_url).await?,
        _ => apply(&user, None, Some(true), &ctx.run.deps.app_url).await?,
    }
    Ok(Some(StepStatus::Success))
}

/// `ResolvedMessengerUserContext`.
struct MessengerUser {
    channel: std::sync::Arc<dyn Channel>,
    ctx: ChannelCtx,
    psid: String,
    contact_inbox: ContactInbox,
    persistent_menus: Value,
    personas: Value,
}

/// `resolveMessengerUserContext`: the run's contact inbox when it is on
/// Messenger, else the contact's most recently active Messenger one.
async fn resolve_user(ctx: &StepCtx<'_, '_>) -> Result<Option<MessengerUser>, JobError> {
    let deps = ctx.run.deps;
    let contact_inbox = if ctx.run.contact_inbox.channel == "messenger" {
        Some(ctx.run.contact_inbox.clone())
    } else {
        let (ws, contact) = (ctx.workspace_id(), ctx.contact_id());
        deps.db
            .read(move |conn| {
                Ok(conn
                    .query_row(
                        &format!(
                            r#"{} WHERE "contactId" = ?1 AND "channel" = 'messenger'
                                 AND "inboxId" IN (SELECT "id" FROM "Inbox" WHERE "workspaceId" = ?2)
                               ORDER BY COALESCE("lastMessageAt", 0) DESC, "id" ASC LIMIT 1"#,
                            ContactInbox::select_sql()
                        ),
                        params![contact, ws],
                        ContactInbox::from_row,
                    )
                    .optional()?)
            })
            .await
            .map_err(db_fail)?
    };
    let Some(contact_inbox) = contact_inbox.filter(|ci| !ci.source_id.is_empty()) else {
        return Ok(None);
    };
    let (channel, channel_ctx) = deps
        .channels
        .resolve(
            &deps.db,
            deps.realtime.as_ref(),
            ctx.workspace_id(),
            &contact_inbox,
        )
        .await
        .map_err(|e| fail(e.to_string()))?;
    let integration_id = channel_ctx.integration_id;
    let (persistent_menus, personas) = deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "persistentMenus", "personas" FROM "IntegrationMessenger" WHERE "id" = ?1"#,
                    params![integration_id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?
        .unwrap_or_default();
    let parse = |text: &str| serde_json::from_str(text).unwrap_or_else(|_| json!([]));
    Ok(Some(MessengerUser {
        channel,
        ctx: channel_ctx,
        psid: contact_inbox.source_id.clone(),
        contact_inbox,
        persistent_menus: parse(&persistent_menus),
        personas: parse(&personas),
    }))
}

/// `findUserPersistentMenuById(...)?.menus ?? []`.
async fn user_menu(ctx: &StepCtx<'_, '_>, id: &str) -> Result<Value, JobError> {
    let (ws, id) = (ctx.workspace_id(), id.parse::<i64>().unwrap_or(-1));
    let menus = ctx
        .run
        .deps
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "menus" FROM "UserPersistentMenu" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![id, ws],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?;
    Ok(menus
        .and_then(|m| serde_json::from_str(&m).ok())
        .unwrap_or_else(|| json!([])))
}

/// `buildBrandingUrl(appUrl, "messenger", isCommunity)`.
pub fn branding_url(app_url: &str) -> String {
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
                .append_pair("channel", "messenger");
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

/// What `applyUserComposerAndMenu` writes.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuWrite {
    /// Disabling the composer with no menu item: Facebook refuses it.
    Skip,
    /// No composer lock and no item: clear the user's custom menu.
    Delete,
    Set(Value),
}

/// `applyUserComposerAndMenu` after reading the current settings:
/// composer = given ?? current user level ?? false; items = given ?? user
/// level ?? page level ?? the workspace's page menu.
pub fn plan_menu_write(
    current: &Value,
    call_to_actions: Option<Vec<Value>>,
    composer_input_disabled: Option<bool>,
    workspace_call_to_actions: impl FnOnce() -> Vec<Value>,
) -> MenuWrite {
    let level_ctas = |level: &str| {
        current
            .pointer(&format!("/{level}/call_to_actions"))
            .and_then(Value::as_array)
            .cloned()
    };
    let composer = composer_input_disabled
        .or_else(|| {
            current
                .pointer("/userLevel/composer_input_disabled")
                .and_then(Value::as_bool)
        })
        .unwrap_or(false);
    let ctas = call_to_actions
        .or_else(|| level_ctas("userLevel"))
        .or_else(|| level_ctas("pageLevel"))
        .unwrap_or_else(workspace_call_to_actions);
    match (composer, ctas.is_empty()) {
        (true, true) => MenuWrite::Skip,
        (false, true) => MenuWrite::Delete,
        _ => MenuWrite::Set(json!([{
            "locale": "default",
            "composer_input_disabled": composer,
            "call_to_actions": ctas,
        }])),
    }
}

async fn apply(
    user: &MessengerUser,
    call_to_actions: Option<Vec<Value>>,
    composer_input_disabled: Option<bool>,
    app_url: &str,
) -> Result<(), JobError> {
    let current = user
        .channel
        .get_user_custom_settings(&user.ctx, user.psid.clone())
        .await
        .map_err(|e| fail(e.to_string()))?;
    let plan = plan_menu_write(&current, call_to_actions, composer_input_disabled, || {
        call_to_actions_of_page(&user.persistent_menus, app_url)
    });
    let written = match plan {
        MenuWrite::Skip => {
            tracing::warn!(
                psid = user.psid,
                "Skipping disable Messenger composer: no persistent menu items available"
            );
            return Ok(());
        }
        MenuWrite::Delete => {
            user.channel
                .delete_user_persistent_menu(&user.ctx, user.psid.clone())
                .await
        }
        MenuWrite::Set(menu) => {
            user.channel
                .set_user_persistent_menu(&user.ctx, user.psid.clone(), menu)
                .await
        }
    };
    written.map_err(|e| fail(e.to_string()))
}

/// `buildWorkspaceCallToActions`.
fn call_to_actions_of_page(menus: &Value, app_url: &str) -> Vec<Value> {
    if menus.as_array().is_none_or(Vec::is_empty) {
        return Vec::new();
    }
    call_to_actions(menus, &branding_url(app_url))
}

/// `setMessengerPersona`: a registered persona of this page, else the page
/// default (`null`, with a warning when one was asked for).
async fn set_persona(ctx: &StepCtx<'_, '_>, user: &MessengerUser) -> Result<(), JobError> {
    let wanted = ctx.str("personaId").filter(|p| !p.is_empty());
    let persona = wanted.and_then(|id| find_registered_persona(&user.personas, id));
    if let (Some(id), None) = (wanted, &persona) {
        tracing::warn!(
            "Set Persona: persona {id} is not available on this page; resetting contact {} to the page default",
            ctx.contact_id()
        );
    }
    let contact_inbox_id = user.contact_inbox.id;
    ctx.run
        .deps
        .db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "ContactInbox" SET "personaId" = ?1 WHERE "id" = ?2"#,
                params![persona, contact_inbox_id],
            )?;
            Ok(())
        })
        .await
        .map_err(db_fail)
}

/// `findRegisteredPersona`: by local id, with a `facebookPersonaId`.
pub fn find_registered_persona(personas: &Value, id: &str) -> Option<String> {
    personas
        .as_array()?
        .iter()
        .find(|p| {
            p.get("id").and_then(Value::as_str) == Some(id)
                && p.get("facebookPersonaId")
                    .and_then(Value::as_str)
                    .is_some_and(|f| !f.is_empty())
        })
        .map(|_| id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menus_become_call_to_actions_with_the_branding_entry_last() {
        let branding = "https://app.test/?ref=selfhosted&channel=messenger";
        let menus = json!([
            { "label": "Built", "type": "url", "url": branding },
            { "label": "Start", "type": "flow", "flowId": "42" },
            { "label": "Site", "type": "url", "url": "https://x.test" },
        ]);
        let ctas = call_to_actions(&menus, branding);
        assert_eq!(
            ctas,
            vec![
                json!({ "type": "postback", "title": "Start",
                    "payload": encode_button_payload(&ButtonPayload { flow_id: "42".into(), ..ButtonPayload::default() }) }),
                json!({ "type": "web_url", "title": "Site", "url": "https://x.test" }),
                json!({ "type": "web_url", "title": "Built", "url": branding }),
            ]
        );
    }

    #[test]
    fn branding_url_sets_ref_and_channel() {
        let url = branding_url("https://app.test");
        assert!(url.starts_with("https://app.test/?ref="));
        assert!(url.ends_with("&channel=messenger"));
    }

    #[test]
    fn menu_write_follows_the_composer_and_item_fallbacks() {
        let cta = json!({ "type": "web_url", "title": "A", "url": "https://a" });
        let none = || Vec::new();
        // Disabling with no item anywhere is skipped.
        assert_eq!(
            plan_menu_write(&json!({}), None, Some(true), none),
            MenuWrite::Skip
        );
        // Enabling with no item clears the user's menu.
        assert_eq!(
            plan_menu_write(&json!({}), None, Some(false), none),
            MenuWrite::Delete
        );
        // A menu write keeps the current user-level composer state.
        let current =
            json!({ "userLevel": { "composer_input_disabled": true, "call_to_actions": [cta] } });
        assert_eq!(
            plan_menu_write(&current, Some(vec![cta.clone()]), None, none),
            MenuWrite::Set(
                json!([{ "locale": "default", "composer_input_disabled": true, "call_to_actions": [cta] }])
            )
        );
        // Items fall back to the page level, then to the workspace menu.
        let page = json!({ "pageLevel": { "call_to_actions": [cta] } });
        assert!(
            matches!(plan_menu_write(&page, None, Some(true), none), MenuWrite::Set(m) if m[0]["call_to_actions"][0] == cta)
        );
        let workspace = plan_menu_write(&json!({}), None, Some(true), || vec![cta.clone()]);
        assert!(matches!(workspace, MenuWrite::Set(m) if m[0]["composer_input_disabled"] == true));
    }

    #[test]
    fn only_registered_personas_are_found() {
        let personas = json!([
            { "id": "a", "facebookPersonaId": "fb-a", "isDefault": false, "name": "A" },
            { "id": "b", "isDefault": false, "name": "B" },
            { "id": "c", "facebookPersonaId": "", "isDefault": false, "name": "C" },
        ]);
        assert_eq!(
            find_registered_persona(&personas, "a").as_deref(),
            Some("a")
        );
        assert_eq!(find_registered_persona(&personas, "b"), None);
        assert_eq!(find_registered_persona(&personas, "c"), None);
        assert_eq!(find_registered_persona(&personas, "z"), None);
    }
}
