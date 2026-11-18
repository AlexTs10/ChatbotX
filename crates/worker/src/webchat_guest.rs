//! `handleCreateWebchatMessage` (`features/messages/actions/
//! create-webchat-message.action.ts`) after the route checks: the guest's
//! contact and conversation, the welcome / menu flow, ref and text inputs,
//! the stored message and its routing. The HTTP layer (token, origin, rate
//! limit) is `chatbotx-server`'s `webchat` module.

use chatbotx_business::integration_webchat::{self as webchat, WebchatRow};
use chatbotx_business::message::{contact_inbox_json, conversation_json};
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::{JobOptions, QueueName};
use serde_json::{Value, json};

use crate::Deps;
use crate::channel::{InboundInbox, IncomingMessage, Received};
use crate::inbound::{self, Stored};

/// The input variant of `createWebchatMessageRequest` (zod union order).
#[derive(Debug, Clone, PartialEq)]
pub enum GuestInput {
    Text {
        text: String,
        postback: Option<String>,
    },
    Flow(i64),
    InitRef(String),
    Init,
}

/// The parsed request (after the route's checks).
#[derive(Debug, Clone, PartialEq)]
pub struct GuestMessage {
    pub workspace_id: i64,
    pub guest_conversation_id: String,
    pub input: GuestInput,
    pub client_id: Option<String>,
    pub parent_url: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
}

/// A `ChatbotXException` (message, HTTP status).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestError {
    pub message: String,
    pub status: u16,
}

impl GuestError {
    fn new(message: impl Into<String>, status: u16) -> GuestError {
        GuestError {
            message: message.into(),
            status,
        }
    }
}

fn internal(error: impl std::fmt::Display) -> GuestError {
    GuestError::new(error.to_string(), 400)
}

/// `contactInboxService.findLatestBySource` (workspace scoped).
pub async fn find_guest_contact_inbox(
    deps: &Deps,
    workspace_id: i64,
    inbox_id: i64,
    guest_conversation_id: &str,
) -> Result<Option<ContactInbox>, GuestError> {
    let source_id = guest_conversation_id.to_owned();
    deps.db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    &format!(
                        r#"{} WHERE "inboxId" = ?1 AND "sourceId" = ?2
                             AND "contactId" IN (SELECT "id" FROM "Contact" WHERE "workspaceId" = ?3)
                           ORDER BY "lastMessageAt" IS NULL, "lastMessageAt" DESC LIMIT 1"#,
                        ContactInbox::select_sql()
                    ),
                    params![inbox_id, source_id, workspace_id],
                    ContactInbox::from_row,
                )
                .optional()?)
        })
        .await
        .map_err(internal)
}

/// `conversationService.findBy({ workspaceId, contactId })`.
pub async fn find_guest_conversation(
    deps: &Deps,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<Conversation>, GuestError> {
    deps.db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    &format!(
                        r#"{} WHERE "workspaceId" = ?1 AND "contactId" = ?2 LIMIT 1"#,
                        Conversation::select_sql()
                    ),
                    params![workspace_id, contact_id],
                    Conversation::from_row,
                )
                .optional()?)
        })
        .await
        .map_err(internal)
}

/// `getConversationFromInput`: `(contactInbox, conversation, isNewContact)`.
async fn conversation_from_input(
    deps: &Deps,
    webchat_row: &WebchatRow,
    owner_id: i64,
    input: &GuestMessage,
) -> Result<(ContactInbox, Conversation, bool), GuestError> {
    let ws = input.workspace_id;
    if let Some(contact_inbox) =
        find_guest_contact_inbox(deps, ws, webchat_row.inbox_id, &input.guest_conversation_id)
            .await?
    {
        let conversation = find_guest_conversation(deps, ws, contact_inbox.contact_id)
            .await?
            .ok_or_else(|| GuestError::new("Conversation not found", 400))?;
        return Ok((contact_inbox, conversation, false));
    }
    let inbox_id = webchat_row.inbox_id;
    let guest = input.clone();
    let created = deps
        .db
        .write(move |tx| {
            webchat::create_guest_conversation(
                tx,
                ws,
                owner_id,
                inbox_id,
                &guest.guest_conversation_id,
                guest.locale.as_deref(),
                guest.timezone.as_deref(),
                guest.parent_url.as_deref(),
                now_ms(),
            )
            .map_err(|e| chatbotx_db::DbError::other(e.message()))
        })
        .await
        .map_err(internal)?
        .ok_or_else(|| GuestError::new("Contact limit reached", 422))?;
    // ponytail: `emitContactCreated` (flow triggers / webhooks of a new
    // contact) waits for the contact event producers (Q.W2).
    let contact_inbox = find_guest_contact_inbox(deps, ws, inbox_id, &input.guest_conversation_id)
        .await?
        .ok_or_else(|| GuestError::new("Contact inbox not found", 404))?;
    let conversation = find_guest_conversation(deps, ws, created.contact_id)
        .await?
        .ok_or_else(|| GuestError::new("Conversation not found", 404))?;
    Ok((contact_inbox, conversation, true))
}

fn send_flow_job(conversation: &Conversation, contact_inbox: &ContactInbox, flow_id: i64) -> Value {
    json!({ "type": "sendFlow", "data": {
        "conversationId": conversation_json(conversation),
        "contactInboxId": contact_inbox_json(contact_inbox),
        "flowId": flow_id.to_string(),
        "origin": "channel",
    }})
}

async fn add(deps: &Deps, name: &str, data: Value, opts: JobOptions) -> Result<(), GuestError> {
    inbound::add_job(deps, QueueName::Integration, name, data, opts)
        .await
        .map_err(|e| internal(e.to_string()))
}

/// `shouldRunWebchatChallenge`.
pub fn should_run_webchat_challenge(
    challenge: Option<&Value>,
    has_postback: bool,
    is_step_input_skip_tap: bool,
) -> bool {
    let Some(challenge) = challenge.filter(|c| !c.is_null()) else {
        return false;
    };
    if is_step_input_skip_tap {
        return false;
    }
    !(challenge["type"].as_str() == Some("quickReply") && has_postback)
}

/// `handleCreateWebchatMessage` after the access checks. `Ok(None)` = the
/// original's `null` (an init, flow or ref input).
pub async fn create_message(
    deps: &Deps,
    webchat_row: &WebchatRow,
    owner_id: i64,
    input: GuestMessage,
) -> Result<Option<Value>, GuestError> {
    let (contact_inbox, conversation, is_new_contact) =
        conversation_from_input(deps, webchat_row, owner_id, &input).await?;
    let is_init = input.input == GuestInput::Init;
    if is_init
        && is_new_contact
        && let Some(flow_id) = webchat_row.welcome_flow_id
    {
        let data = send_flow_job(&conversation, &contact_inbox, flow_id);
        add(deps, "sendFlow", data, JobOptions::default()).await?;
    }
    let (text, postback) = match &input.input {
        GuestInput::Flow(flow_id) => {
            // Only a flow of this webchat's persistent menu may run.
            let configured = webchat_row.persistent_menus.iter().any(|menu| {
                menu["type"].as_str() == Some("flow")
                    && crate::id_of(&menu["flowId"]) == Some(*flow_id)
            });
            if !configured {
                return Err(GuestError::new("Flow not found", 404));
            }
            let data = send_flow_job(&conversation, &contact_inbox, *flow_id);
            add(deps, "sendFlow", data, JobOptions::default()).await?;
            return Ok(None);
        }
        GuestInput::Init => return Ok(None),
        GuestInput::InitRef(r#ref) => {
            if !r#ref.is_empty() {
                let data = json!({ "type": "runRef", "data": {
                    "conversationId": conversation_json(&conversation),
                    "contactInboxId": contact_inbox_json(&contact_inbox),
                    "ref": r#ref,
                    "isNewContact": is_new_contact,
                }});
                add(deps, "runRef", data, JobOptions::default()).await?;
            }
            return Ok(None);
        }
        GuestInput::Text { text, postback } => (text.clone(), postback.clone()),
    };
    let postback = postback.filter(|p| !p.is_empty());
    let skip_challenge_id =
        chatbotx_core::flow_config::step_input_resolution::decode_step_input_skip_postback(
            postback.as_deref(),
        );
    // ponytail: file uploads (`files`) arrive only through the builder's
    // server action (FormData); the JSON route cannot carry them.
    let inbox = InboundInbox {
        inbox_id: webchat_row.inbox_id,
        workspace_id: conversation.workspace_id,
        channel: "webchat".into(),
    };
    let mut incoming = IncomingMessage::incoming(None);
    incoming.text = Some(text.clone());
    let received = Received::default();
    let (message, _) = inbound::save_message(
        deps,
        &inbox,
        &contact_inbox,
        &conversation,
        &incoming,
        &received,
        input.client_id.as_deref(),
    )
    .await
    .map_err(|e| internal(e.to_string()))?;
    let message_id = crate::id_of(&message["id"]).unwrap_or_default();
    if let Some(parent_url) = input.parent_url.clone() {
        let id = contact_inbox.id;
        let result = deps
            .db
            .write(move |tx| {
                tx.execute(
                    r#"UPDATE "ContactInbox" SET "webchatParentUrl" = ?1 WHERE "id" = ?2"#,
                    params![parent_url, id],
                )?;
                Ok(())
            })
            .await;
        if let Err(err) = result {
            tracing::warn!(err = %err, "webchatParentUrl tracking failed");
        }
    }
    inbound::unblock_if_blocked(deps, contact_inbox.contact_id, conversation.workspace_id).await;
    let stored = Stored {
        inbox,
        conversation: conversation.clone(),
        contact_inbox: contact_inbox.clone(),
        message: Some(message.clone()),
        message_id: Some(message_id),
        received,
        is_new_contact,
        step_input_skip: skip_challenge_id.is_some(),
    };
    if let Some(postback) = postback.as_deref().filter(|_| skip_challenge_id.is_none())
        && let Err(err) =
            inbound::enqueue_flow_action(deps, &stored, "postback", postback, message_id).await
    {
        return Err(internal(err.to_string()));
    }
    let challenge = conversation
        .additional_attributes
        .as_ref()
        .and_then(|a| a.0.get("challenge").cloned());
    let has_postback = postback.is_some();
    // Routing is best effort (`Promise.allSettled`).
    if should_run_webchat_challenge(
        challenge.as_ref(),
        has_postback,
        skip_challenge_id.is_some(),
    ) {
        let data = json!({ "type": "runChallenge", "data": {
            "conversationId": conversation_json(&conversation),
            "contactInboxId": contact_inbox_json(&contact_inbox),
            "messageId": message_id.to_string(),
            "messageCreatedAt": message["createdAt"],
            "challenge": challenge,
        }});
        let opts = JobOptions {
            job_id: Some(format!(
                "webchat-challenge-{}-{message_id}",
                conversation.id
            )),
            ..JobOptions::default()
        };
        if let Err(err) = add(deps, "runChallenge", data, opts).await {
            tracing::warn!(err = %err.message, "webchat challenge enqueue failed");
        }
    } else if !has_postback
        && inbound::ensure_active(deps, &conversation).await
        && let Err(err) =
            inbound::enqueue_automated_response(deps, &stored, message_id, Some(text), Vec::new())
                .await
    {
        // ponytail: `enqueueHandoffReentry` needs the AI handover port (Q.D6).
        tracing::warn!(err = %err, "webchat automated response enqueue failed");
    }
    if let Some(challenge_id) = skip_challenge_id {
        use chatbotx_jobs::queues::resolve_step_input::{
            ResolveStepInput, ResolveStepInputReason, build_resolve_step_input_job,
        };
        let job = build_resolve_step_input_job(&ResolveStepInput {
            conversation_id: conversation.id.to_string(),
            contact_inbox_id: contact_inbox.id.to_string(),
            challenge_id,
            reason: ResolveStepInputReason::UserSkipped,
            message_id: Some(message_id.to_string()),
            message_created_at: message["createdAt"].as_str().map(str::to_owned),
        });
        add(deps, &job.name, job.data, job.opts).await?;
    }
    Ok(Some(message))
}

#[cfg(test)]
mod tests {
    //! `apps/builder/__tests__/should-run-webchat-challenge.test.ts`.
    use super::*;

    fn quick_reply() -> Value {
        json!({ "type": "quickReply", "data": { "flowId": "f", "nodeId": "n", "attempts": 0, "maxRetries": 3 } })
    }

    fn step() -> Value {
        json!({ "type": "step", "data": { "flowId": "f", "nodeId": "n", "stepId": "s", "attempts": 0 } })
    }

    #[test]
    fn no_challenge_is_false() {
        assert!(!should_run_webchat_challenge(None, false, false));
    }

    #[test]
    fn free_text_with_a_quick_reply_retry_pending_is_true() {
        assert!(should_run_webchat_challenge(
            Some(&quick_reply()),
            false,
            false
        ));
    }

    #[test]
    fn a_quick_reply_tap_never_counts_as_a_non_quick_reply_answer() {
        assert!(!should_run_webchat_challenge(
            Some(&quick_reply()),
            true,
            false
        ));
    }

    #[test]
    fn get_user_data_keeps_its_existing_behaviour_for_postbacks() {
        assert!(should_run_webchat_challenge(Some(&step()), true, false));
    }

    #[test]
    fn a_skip_tap_is_resolved_by_its_own_job_never_run_as_a_typed_reply() {
        assert!(!should_run_webchat_challenge(Some(&step()), true, true));
    }

    #[test]
    fn without_the_skip_flag_the_third_argument_changes_nothing() {
        assert!(should_run_webchat_challenge(Some(&step()), true, false));
        assert!(should_run_webchat_challenge(Some(&step()), false, false));
    }
}
