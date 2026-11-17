//! `apps/worker/src/integration/handlers/thread-control.ts`: the
//! `threadControlAction` job (archive auto-release) and the `threadControlEvent`
//! routing webhook job.

use chatbotx_business::thread_control as tc;
use chatbotx_db::repo::parse_iso;
use chatbotx_jobs::JobError;
use serde_json::Value;

use crate::{Deps, str_of};

/// ponytail: the worker has no Graph base override; channel tests use the
/// business crate's own local-server tests.
const GRAPH: &str = chatbotx_business::whatsapp::GRAPH_API_URL;

fn id_of(data: &Value, key: &str) -> Option<i64> {
    str_of(data, key).and_then(|v| v.parse().ok())
}

/// `releaseOwnedThread`: re-checks owned + version + still archived, then releases;
/// a permanent rejection is logged and the job completes.
pub async fn release_owned_thread(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let (Some(ws), Some(contact_inbox), Some(conversation_id)) = (
        id_of(data, "workspaceId"),
        id_of(data, "contactInboxId"),
        id_of(data, "conversationId"),
    ) else {
        return Err(JobError::unrecoverable(
            "invalid threadControlAction payload",
        ));
    };
    let current = tc::resolve_current_state(&deps.db, ws, contact_inbox)
        .await
        .map_err(|e| JobError::retryable(e.to_string()))?;
    let Some((_, updated_at)) = current.filter(|(state, _)| state == "owned") else {
        tracing::info!(
            contact_inbox,
            "Thread release after archive skipped: the thread is no longer owned"
        );
        return Ok(());
    };
    // A job queued before the version was carried keeps the owned-only check.
    if let Some(expected) = data.get("threadControlUpdatedAt")
        && expected.as_str().and_then(parse_iso) != updated_at
    {
        tracing::info!(
            contact_inbox,
            "Thread release after archive skipped: ownership changed since the archive"
        );
        return Ok(());
    }
    let conversation =
        match chatbotx_business::conversation_list::find_or_fail(&deps.db, ws, conversation_id)
            .await
        {
            Ok(conversation) => Some(conversation),
            Err(error) if error.code() == Some("notFound") => None,
            Err(error) => return Err(JobError::retryable(error.message())),
        };
    let Some(conversation) = conversation.filter(|c| c.archived_at.is_some()) else {
        tracing::info!(
            contact_inbox,
            "Thread release after archive skipped: the conversation is no longer archived"
        );
        return Ok(());
    };
    let action = str_of(data, "action").unwrap_or("release");
    let result = tc::request_action(
        &deps.db,
        deps.realtime.as_ref(),
        GRAPH,
        tc::ActionRequest {
            workspace_id: ws,
            contact_inbox_id: contact_inbox,
            conversation_id: conversation.id,
            action,
            target_role: None,
            metadata: None,
            expected_updated_at: Some(updated_at),
        },
    )
    .await;
    match result {
        Ok(_) => Ok(()),
        Err(error) if error.is_permanent() => {
            tracing::warn!(%error, contact_inbox, "Thread release after archive was rejected; not retrying");
            Ok(())
        }
        Err(error) => Err(JobError::retryable(error.to_string())),
    }
}
