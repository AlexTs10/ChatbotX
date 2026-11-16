//! `packages/business/src/capabilities/service.ts` `getCapabilities` (Q.D11
//! slice 5): compact workspace reference data and the FlowSpec authoring
//! capabilities of `GET /v1/capabilities`. (`getFlowAuthoringContext` is
//! `flow::authoring_context`.)

use chatbotx_core::flow_config::deps::channel_types;
use chatbotx_core::flow_config::{
    CHANNEL_POLICY_VERSION, extract_template_params, flow_spec_step_types, wait_step_delay_units,
};
use chatbotx_db::Db;
use chatbotx_db::repo::{PaginationInput, SortItem, get_pagination_with_defaults};
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::{Map, Value, json};

use crate::errors::{BusinessError, Result};
use crate::{bot_field, custom_field, flow, tx};

/// `CAPABILITIES_LIST_LIMIT`.
pub const CAPABILITIES_LIST_LIMIT: i64 = 200;

/// `CAPABILITIES_INCLUDES`.
pub const CAPABILITIES_INCLUDES: &[&str] = &[
    "inboxes",
    "templates",
    "customFields",
    "botFields",
    "tags",
    "aiAgents",
    "sequences",
    "flows",
    "flowSpec",
];

/// `OPT_IN_INCLUDES`.
pub const OPT_IN_INCLUDES: &[&str] = &["aiAgents"];

/// `DEFAULT_INCLUDES`: every include except the opt-in ones.
pub fn default_includes() -> Vec<&'static str> {
    CAPABILITIES_INCLUDES
        .iter()
        .copied()
        .filter(|i| !OPT_IN_INCLUDES.contains(i))
        .collect()
}

/// The key order of every policy's `steps` object (`defineChannelPolicy`
/// declares them in this order; the Rust policy values are sorted maps).
// ponytail: one shared order, checked against all 11 policies at port time;
// a policy with its own order would need its keys kept in core.
const POLICY_STEP_ORDER: &[&str] = &[
    "landingPage",
    "chooseChannel",
    "sendText",
    "sendImage",
    "sendMultipleImages",
    "sendCard",
    "sendCarousel",
    "sendViewCatalog",
    "sendVideo",
    "sendGif",
    "sendMessengerOtn",
    "sendAudio",
    "sendFile",
    "sendQuickReply",
    "waitUserReply",
    "setDebounce",
    "wait",
    "followUp",
    "getUserData",
    "typing",
    "addContactTag",
    "removeContactTag",
    "deleteContact",
    "blockContact",
    "addContactNotes",
    "setCustomField",
    "clearCustomField",
    "cancelContactInput",
    "appointmentScheduling",
    "questionnaires",
    "setUpCoupon",
    "markCouponUsed",
    "changeGamificationScore",
    "pipeline",
    "condition",
    "evaluateDecision",
    "disableBot",
    "enableBot",
    "assignConversation",
    "autoAssignConversation",
    "unassignConversation",
    "markConversationAsUnread",
    "markConversationAsRead",
    "followConversation",
    "unfollowConversation",
    "archiveConversation",
    "unarchiveConversation",
    "notifyAgent",
    "threadControl",
    "aiGenerateText",
    "aiGenerateTextAgent",
    "aiAnalyzeImage",
    "aiGenerateImage",
    "aiEditImage",
    "aiSegmentImage",
    "aiSpeechToText",
    "aiTextToSpeech",
    "aiExtractData",
    "aiDeleteMessageHistory",
    "markEmailVerified",
    "optInEmail",
    "optOutEmail",
    "getDataFromJson",
    "formatDate",
    "generateCode",
    "countCharacters",
    "performAction",
    "callApi",
    "executeJavascript",
    "splitTraffic",
    "make",
    "triggerN8n",
    "startAnotherNode",
    "startExternalFlow",
    "startExternalNode",
    "openWebsite",
    "openMiniApp",
    "callPhoneNumber",
    "copyText",
    "addNotes",
    "subscribeBroadcast",
    "unsubscribeBroadcast",
    "spreadsheetSendData",
    "spreadsheetGetRow",
    "spreadsheetGetRandomRow",
    "spreadsheetUpdateRow",
    "spreadsheetClearRow",
    "activeCampaignSyncContact",
    "getResponseAddContact",
    "mailchimpAddMember",
    "mailerLiteAddSubscriber",
    "moosendCreateContact",
    "dripSubscribeSubscriber",
    "sendGridAddContact",
    "klaviyoSyncProfile",
    "subscribeSequence",
    "unsubscribeSequence",
    "email",
    "sendWaTemplateMessage",
    "whatsappOptionList",
    "whatsappCallButton",
    "whatsappLocation",
    "whatsappFlow",
    "sendProducts",
    "sendCollection",
    "sendMessengerTemplateMessage",
    "facebookCustomAudience",
    "sendMetaCapiEvent",
    "sendGoogleAdsConversion",
    "setMessengerUserPersistentMenu",
    "enableMessengerComposer",
    "disableMessengerComposer",
    "setMessengerPersona",
    "updateMessengerContactData",
    "findPlacesNearMe",
];

/// The raw policy object of a channel (`CHANNEL_FLOW_POLICIES[channel]`).
fn policy_value(channel: &str) -> Option<&'static Value> {
    use chatbotx_core::flow_config::*;
    Some(match channel {
        "api" => api_flow_policy(),
        "instagram" => instagram_flow_policy(),
        "messenger" => messenger_flow_policy(),
        "omnichannel" => omnichannel_flow_policy(),
        "smtp" => smtp_flow_policy(),
        "telegram" => telegram_flow_policy(),
        "threads" => threads_flow_policy(),
        "tiktok" => tiktok_flow_policy(),
        "webchat" => webchat_flow_policy(),
        "whatsapp" => whatsapp_flow_policy(),
        "zalo" => zalo_flow_policy(),
        _ => return None,
    })
}

/// The steps of a policy with one support level, in declaration order. The
/// Execute JavaScript step is dropped (DECISIONS 2026-11-11).
fn steps_with(policy: &Value, support: &str) -> Vec<&'static str> {
    POLICY_STEP_ORDER
        .iter()
        .copied()
        .filter(|step| *step != "executeJavascript")
        .filter(|step| policy["steps"][*step] == support)
        .collect()
}

/// `getFlowSpecCapabilities(channel)`.
pub fn flow_spec_capabilities(channel: Option<&str>) -> Value {
    let mut out = json!({
        "stepTypes": flow_spec_step_types(),
        "waitUnits": wait_step_delay_units().enum_values(),
        "channels": channel_types().enum_values(),
        "policyVersion": CHANNEL_POLICY_VERSION,
    });
    let Some(policy) = channel.and_then(policy_value) else {
        return out;
    };
    let mut selected = json!({
        "policyVersion": CHANNEL_POLICY_VERSION,
        "unsupportedSteps": steps_with(policy, "unsupported"),
        "noButtonSteps": steps_with(policy, "noButtons"),
        "nativeButtonTypes": policy["nativeButtonTypes"],
        "limits": policy["limits"],
    });
    if let Some(product_messages) = policy.get("productMessages").filter(|v| !v.is_null()) {
        selected["productMessages"] = product_messages.clone();
    }
    out["selectedChannelPolicy"] = selected;
    out
}

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `getPaginationWithDefaults({ perPage: 200 })`: clamped to `maxLimit` (50).
fn paged_limit() -> i64 {
    get_pagination_with_defaults(PaginationInput {
        page: None,
        per_page: Some(CAPABILITIES_LIST_LIMIT),
    })
    .limit
}

fn stable_sort() -> Option<Vec<SortItem>> {
    Some(vec![SortItem {
        id: "id".into(),
        desc: false,
    }])
}

/// Rows of `sql` (one `?1` workspace bind, one `?2` limit) as JSON objects
/// of `keys` (an `"id"` key is stringified).
fn rows(
    conn: &Connection,
    sql: &str,
    workspace_id: i64,
    limit: i64,
    keys: &[&str],
) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql).map_err(repo_err)?;
    let out = stmt
        .query_map(params![workspace_id, limit], |row| {
            let mut object = Map::new();
            for (i, key) in keys.iter().enumerate() {
                let value = if *key == "id" {
                    json!(row.get::<_, i64>(i)?.to_string())
                } else {
                    json!(row.get::<_, Option<String>>(i)?)
                };
                object.insert((*key).to_string(), value);
            }
            Ok(Value::Object(object))
        })
        .map_err(repo_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(repo_err)?;
    Ok(out)
}

/// `toCapabilitiesTemplate`: a non-array `components` has no params.
fn template_json(
    id: i64,
    name: String,
    language: String,
    status: String,
    components: Value,
) -> Value {
    let components = components.as_array().cloned().unwrap_or_default();
    json!({
        "id": id.to_string(),
        "name": name,
        "language": language,
        "status": status,
        "params": extract_template_params(&components),
    })
}

async fn load(db: &Db, include: &str, workspace_id: i64, channel: Option<&str>) -> Result<Value> {
    let ws = workspace_id;
    Ok(match include {
        // `inboxService.list({ workspaceId, perPage })`: connected inboxes, no sort.
        "inboxes" => {
            tx::read(db, move |conn| {
                rows(
                    conn,
                    r#"SELECT "id", "name", "channel" FROM "Inbox" WHERE "workspaceId" = ?1 AND "status" = 'connected' ORDER BY "id" LIMIT ?2"#,
                    ws,
                    paged_limit(),
                    &["id", "name", "channel"],
                )
            })
            .await?
            .into()
        }
        // `whatsappMessageTemplateService.list` (createdAt asc), first 200.
        "templates" => {
            tx::read(db, move |conn| {
                let mut stmt = conn
                    .prepare(
                        r#"SELECT t."id", t."name", t."language", t."status", t."components" FROM "WhatsappMessageTemplate" t JOIN "IntegrationWhatsapp" i ON i."id" = t."integrationWhatsappId" WHERE i."workspaceId" = ?1 ORDER BY t."createdAt", t."id" LIMIT ?2"#,
                    )
                    .map_err(repo_err)?;
                let out = stmt
                    .query_map(params![ws, CAPABILITIES_LIST_LIMIT], |row| {
                        let components: Option<String> = row.get(4)?;
                        let components = components
                            .and_then(|c| serde_json::from_str(&c).ok())
                            .unwrap_or(Value::Null);
                        Ok(template_json(row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, components))
                    })
                    .map_err(repo_err)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(repo_err)?;
                Ok(out)
            })
            .await?
            .into()
        }
        "customFields" => {
            let page = custom_field::list(
                db,
                ws,
                custom_field::ListCustomFields {
                    per_page: Some(CAPABILITIES_LIST_LIMIT),
                    sort: stable_sort(),
                    ..Default::default()
                },
            )
            .await?;
            page.data
                .iter()
                .map(|f| json!({ "id": f.id.to_string(), "name": f.name, "type": f.r#type }))
                .collect::<Vec<_>>()
                .into()
        }
        "botFields" => {
            let page = bot_field::list(
                db,
                ws,
                bot_field::ListBotFields {
                    per_page: Some(CAPABILITIES_LIST_LIMIT),
                    sort: stable_sort(),
                    ..Default::default()
                },
            )
            .await?;
            page.data
                .iter()
                .map(|f| json!({ "id": f.id.to_string(), "name": f.name, "type": f.r#type }))
                .collect::<Vec<_>>()
                .into()
        }
        // `tagService.listActive` (name asc), first 200.
        "tags" => {
            tx::read(db, move |conn| {
                rows(
                    conn,
                    r#"SELECT "id", "name" FROM "Tag" WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL ORDER BY "name", "id" LIMIT ?2"#,
                    ws,
                    CAPABILITIES_LIST_LIMIT,
                    &["id", "name"],
                )
            })
            .await?
            .into()
        }
        "aiAgents" => tx::read(db, move |conn| {
            rows(
                conn,
                r#"SELECT "id", "name" FROM "AIAgent" WHERE "workspaceId" = ?1 ORDER BY "id" LIMIT ?2"#,
                ws,
                paged_limit(),
                &["id", "name"],
            )
        })
        .await?
        .into(),
        "sequences" => tx::read(db, move |conn| {
            rows(
                conn,
                r#"SELECT "id", "name" FROM "Sequence" WHERE "workspaceId" = ?1 ORDER BY "id" LIMIT ?2"#,
                ws,
                paged_limit(),
                &["id", "name"],
            )
        })
        .await?
        .into(),
        "flows" => {
            let page = flow::list(
                db,
                flow::ListFlows {
                    workspace_id: ws,
                    per_page: Some(CAPABILITIES_LIST_LIMIT),
                    sort: stable_sort(),
                    ..Default::default()
                },
            )
            .await?;
            page.data
                .iter()
                .map(|f| json!({ "id": f.flow.id.to_string(), "name": f.flow.name }))
                .collect::<Vec<_>>()
                .into()
        }
        "flowSpec" => flow_spec_capabilities(channel),
        other => {
            return Err(BusinessError::exception(
                format!("unknown capabilities include {other}"),
                "BAD_REQUEST",
                400,
            ));
        }
    })
}

/// `getCapabilities`: one key per include (default: [`default_includes`]),
/// in include order. A failing loader is logged and left out.
pub async fn get_capabilities(
    db: &Db,
    workspace_id: i64,
    include: Option<&[String]>,
    channel: Option<&str>,
) -> Value {
    let includes: Vec<String> = match include {
        Some(list) => list.to_vec(),
        None => default_includes().into_iter().map(String::from).collect(),
    };
    let mut out = Map::new();
    for include in includes {
        match load(db, &include, workspace_id, channel).await {
            Ok(value) => {
                out.insert(include, value);
            }
            Err(err) => tracing::error!(
                err = %err.message(),
                workspace_id,
                "capabilities.get: one loader failed; omitting it from the response"
            ),
        }
    }
    Value::Object(out)
}
