//! `captureTemplateFlowResponse` (`apps/worker/src/integration/handlers/
//! template-flow-response.ts`, Q.P7-V4): the response of a Flow launched by
//! a template FLOW button is applied to the button it was sent with, then
//! scores the WhatsApp "survey" leaderboard rule.

use chatbotx_business::leaderboard::scoring::Award;
use chatbotx_business::tx;
use chatbotx_business::whatsapp_flow_response::{
    ApplyInput, apply_response, resolve_template_flow_button,
};
use chatbotx_core::flow_config::steps::decode_template_flow_token;
use chatbotx_db::repo::find_one;
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_jobs::JobError;
use serde_json::Value;

use crate::{Deps, fail, id_of};

pub const CAPTURE_TEMPLATE_FLOW_RESPONSE: &str = "captureTemplateFlowResponse";

pub async fn capture(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let (Some(ws), Some(conversation_id), Some(contact_inbox_id)) = (
        id_of(&data["workspaceId"]),
        id_of(&data["conversationId"]),
        id_of(&data["contactInboxId"]),
    ) else {
        return Err(JobError::unrecoverable(
            "captureTemplateFlowResponse: bad data",
        ));
    };
    let message_id = id_of(&data["messageId"]);
    let Some(token) = decode_template_flow_token(data["templateFlowToken"].as_str()) else {
        tracing::warn!(
            workspace_id = ws,
            message_id,
            "[template-flow-response] invalid token"
        );
        return Ok(());
    };
    // `detectConversationAndContactInbox`.
    let (conversation, contact_inbox) = tx::read(&deps.db, move |conn| {
        Ok((
            find_one::<Conversation>(conn, "\"id\" = ?1", &[&conversation_id])?,
            find_one::<ContactInbox>(conn, "\"id\" = ?1", &[&contact_inbox_id])?,
        ))
    })
    .await
    .map_err(|e| fail(e.to_string()))?;
    let (Some(conversation), Some(contact_inbox)) = (conversation, contact_inbox) else {
        return Err(fail("Conversation or contact inbox not found"));
    };
    if conversation.workspace_id != ws {
        tracing::warn!(
            workspace_id = ws,
            conversation_workspace_id = conversation.workspace_id,
            conversation_id,
            message_id,
            "[template-flow-response] workspace mismatch"
        );
        return Ok(());
    }
    let inbox_id = contact_inbox.inbox_id;
    let resolved = tx::read(&deps.db, move |conn| {
        resolve_template_flow_button(conn, &token, ws, inbox_id)
    })
    .await
    .map_err(|e| fail(e.to_string()))?;
    let Some(resolved) = resolved else {
        return Ok(());
    };
    apply_response(
        &deps.db,
        ApplyInput {
            workspace_id: ws,
            contact_id: conversation.contact_id,
            inbox_id: Some(inbox_id),
            integration_whatsapp_id: resolved.integration_whatsapp_id,
            flow_source_id: resolved.flow_source_id,
            field_mappings: resolved.param["fieldMappings"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
            flow_response: data["flowResponse"]
                .as_object()
                .cloned()
                .unwrap_or_default(),
            flow_files: data["flowFiles"].as_object().cloned(),
        },
    )
    .await
    .map_err(|e| fail(e.to_string()))?;
    award_survey(
        deps,
        ws,
        conversation.contact_id,
        &contact_inbox,
        message_id,
    )
    .await;
    Ok(())
}

/// `awardWhatsappFlowSurvey`: the stored response message is the source
/// event (its id the `sourceRef`, the time packed in it the event time).
pub(crate) async fn award_survey(
    deps: &Deps,
    workspace_id: i64,
    contact_id: i64,
    contact_inbox: &ContactInbox,
    message_id: Option<i64>,
) {
    let Some(message_id) = message_id.filter(|_| contact_inbox.channel == "whatsapp") else {
        return;
    };
    let Some(event_at) =
        chatbotx_core::utils::id::created_at_from_id(&message_id.to_string())
    else {
        return;
    };
    crate::leaderboard::enqueue(
        deps,
        false,
        Award {
            workspace_id,
            contact_id,
            inbox_id: Some(contact_inbox.inbox_id),
            minigame_id: None,
            rule: "survey".into(),
            source_ref: format!("waflow:{message_id}"),
            event_at,
            context: Value::Null,
        },
    )
    .await;
}
