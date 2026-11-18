//! The read routes of `features/whatsapp-calls/api/public.ts` (scope
//! `integrations`): `whatsappCalls.list` (`whatsappCallHistoryService.list`
//! with `member: "workspace"`), `getRecording`, `getTranscript`, `getSummary`,
//! `listSummaryProviders`.
//!
//! ponytail: `generateSummary` (the AI summary writer) is not ported yet: the
//! AI client lives in `chatbotx-worker`.
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chatbotx_business::{BusinessError, tx};
use chatbotx_db::repo::models::WhatsappCall;
use chatbotx_db::repo::whatsapp_call::{
    self as calls, ListForWorkspaceInput, WhatsappCallHistoryScope, WhatsappCallListCursor,
    WhatsappCallListFilters, WhatsappCallListRow,
};
use chatbotx_db::rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use super::contact_inboxes::lenient_base64;
use super::input::{Input, Str, key};
use super::tags::digits_at;
use super::{
    ApiState, Call, ErrorMap, FINDING, LISTING, Reply, business_error, internal_error, json,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "whatsappCalls.list",
    "whatsappCalls.getRecording",
    "whatsappCalls.getTranscript",
    "whatsappCalls.getSummary",
    "whatsappCalls.listSummaryProviders",
];

/// `aiProviders` with `PROVIDER_LABELS` and the integration table
/// (`listConnectedCallSummaryProviders`; `openaiCompatible` is left out).
const SUMMARY_PROVIDERS: [(&str, &str, &str); 6] = [
    ("openai", "OpenAI", "IntegrationOpenai"),
    ("gemini", "Gemini", "IntegrationGemini"),
    ("claude", "Claude", "IntegrationClaude"),
    ("deepseek", "DeepSeek", "IntegrationDeepseek"),
    ("openrouter", "OpenRouter", "IntegrationOpenrouter"),
    ("muse", "Muse", "IntegrationMuse"),
];

/// `possibleErrorsOnListingWithCursor`.
const LISTING_WITH_CURSOR: ErrorMap = &[("businessError", 400), ("invalidCursor", 400)];
const PAGE_SIZE: i64 = 25;
/// `RECORDING_SIGNED_URL_TTL_SECONDS`.
const RECORDING_URL_TTL_SECONDS: u64 = 15 * 60;

/// `resolveDisplayCallOutcome`.
fn display_outcome<'a>(status: &'a str, outcome: Option<&'a str>) -> Option<&'a str> {
    outcome.or(matches!(status, "completed" | "rejected" | "failed").then_some(status))
}

/// `resolveCallKind` (`CALL_KIND_RULES`, first match).
pub fn call_kind(status: &str, outcome: Option<&str>, direction: &str) -> Option<&'static str> {
    if matches!(status, "ringing" | "accepted") {
        return Some("ongoing");
    }
    let inbound = direction == "userInitiated";
    Some(match (display_outcome(status, outcome)?, inbound) {
        ("canceled", _) => "canceled",
        ("rejected", _) => "declined",
        ("failed", true) => "missed",
        ("failed", false) if direction == "businessInitiated" => "unanswered",
        ("completed", true) => "answeredInbound",
        ("completed", false) if direction == "businessInitiated" => "answeredOutbound",
        _ => return None,
    })
}

fn row_json(row: &WhatsappCallListRow) -> Value {
    let call = &row.call;
    let user = |u: &Option<calls::CallUser>| {
        u.as_ref().map_or(
            Value::Null,
            |u| json!({ "id": u.id.to_string(), "name": u.name }),
        )
    };
    json!({
        "id": call.id.to_string(),
        "createdAt": chatbotx_db::repo::iso(call.created_at),
        "direction": call.direction,
        "status": call.status,
        "outcome": call.outcome,
        "kind": call_kind(&call.status, call.outcome.as_deref(), &call.direction),
        "durationSeconds": call.duration_seconds,
        "hasRecording": call.recording_path.as_deref().is_some_and(|p| !p.is_empty()),
        "conversationId": call.conversation_id.to_string(),
        "contact": {
            "id": row.contact.id.to_string(),
            "fullName": row.contact.full_name,
            "avatar": row.contact.avatar,
        },
        "inbox": { "id": row.inbox_id.to_string(), "name": row.inbox_name },
        "answeredByUser": user(&row.answered_by_user),
        "initiatedByUser": user(&row.initiated_by_user),
    })
}

/// `decodeCursor(cursor, whatsappCallListCursorSchema)`.
fn decode_cursor(text: &str) -> Option<WhatsappCallListCursor> {
    let value: Value = serde_json::from_slice(&lenient_base64(text)?).ok()?;
    let created_at = value.get("createdAt")?.as_str()?;
    chatbotx_db::repo::parse_iso(created_at)?;
    let id = value.get("id")?.as_str()?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(WhatsappCallListCursor {
        created_at: created_at.to_owned(),
        id: id.parse().ok()?,
    })
}

fn encode_cursor(cursor: &WhatsappCallListCursor) -> String {
    STANDARD
        .encode(json!({ "createdAt": cursor.created_at, "id": cursor.id.to_string() }).to_string())
}

fn call_not_found() -> BusinessError {
    BusinessError::not_found("Call not found")
}

/// `loadCallForWorkspace`.
async fn load_call(state: &ApiState, ws: i64, id: i64) -> Result<WhatsappCall, BusinessError> {
    tx::read(&state.db, move |c| {
        Ok(calls::find_by_id_for_workspace(c, id, ws)?)
    })
    .await?
    .filter(|call| call.workspace_id == ws)
    .ok_or_else(call_not_found)
}

/// `resolveSpeakerNames`: the agent's name (else the workspace's) and the
/// customer's name (else phone).
async fn speaker_names(state: &ApiState, call: &WhatsappCall) -> Result<Value, BusinessError> {
    let agent = if call.direction == "businessInitiated" {
        call.initiated_by_user_id
    } else {
        call.answered_by_user_id
    };
    let (ws, contact_inbox) = (call.workspace_id, call.contact_inbox_id);
    tx::read(&state.db, move |c| {
        let agent_name: Option<String> = match agent {
            Some(id) => c
                .query_row(
                    r#"SELECT "name" FROM "User" WHERE "id" = ?1"#,
                    params![id],
                    |r| r.get(0),
                )
                .optional()?
                .flatten(),
            None => None,
        };
        let business = match agent_name.filter(|n: &String| !n.is_empty()) {
            Some(name) => name,
            None => c
                .query_row(
                    r#"SELECT "name" FROM "Workspace" WHERE "id" = ?1"#,
                    params![ws],
                    |r| r.get(0),
                )
                .optional()?
                .unwrap_or_default(),
        };
        let customer: Option<(Option<String>, Option<String>)> = c
            .query_row(
                r#"SELECT ct."fullName", ct."phoneNumber" FROM "ContactInbox" ci
                   JOIN "Contact" ct ON ct."id" = ci."contactId" AND ct."workspaceId" = ?2
                   WHERE ci."id" = ?1"#,
                params![contact_inbox, ws],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let customer = customer
            .and_then(|(name, phone)| name.or(phone))
            .unwrap_or_default();
        Ok(json!({ "business": business, "customer": customer }))
    })
    .await
}

fn id_input(call: &Call<'_>) -> Result<i64, crate::orpc::OrpcError> {
    let mut input = Input::from_query(call.params, call.query);
    let value = input.get("id").cloned();
    let id = digits_at(&mut input, &key("id"), value.as_ref(), true).unwrap_or_default();
    input.finish()?;
    Ok(id)
}

pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    match route_id {
        "whatsappCalls.list" => list(state, call).await,
        "whatsappCalls.getRecording" => {
            let id = id_input(&call)?;
            let path = tx::read(&state.db, move |c| {
                Ok(calls::find_by_id_for_workspace(c, id, ws)?)
            })
            .await
            .map_err(|e| business_error(e, FINDING))?
            .and_then(|c| c.recording_path)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| {
                business_error(
                    BusinessError::not_found("Call recording not found"),
                    FINDING,
                )
            })?;
            let files = state
                .files
                .as_ref()
                .ok_or_else(|| internal_error("file store is off"))?;
            let url = files
                .get_presigned_download(&path, Some(RECORDING_URL_TTL_SECONDS))
                .await
                .map_err(|e| internal_error(&e.to_string()))?;
            Ok(json(200, json!({ "url": url })))
        }
        "whatsappCalls.getTranscript" => {
            let id = id_input(&call)?;
            let run = async {
                let found = load_call(state, ws, id).await?;
                let segments: Vec<Value> = found
                    .transcript_segments
                    .as_ref()
                    .and_then(|s| s.0.as_array().cloned())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|s| json!({ "speaker": s["speaker"], "start": s["start"], "end": s["end"], "text": s["text"] }))
                    .collect();
                let has_speakers = segments
                    .iter()
                    .any(|s| s["speaker"].as_str().is_some_and(|v| !v.is_empty()));
                let names = speaker_names(state, &found).await?;
                Ok(
                    json!({ "segments": segments, "speakerNames": names, "hasSpeakers": has_speakers }),
                )
            };
            run.await
                .map(|out| json(200, out))
                .map_err(|e| business_error(e, FINDING))
        }
        "whatsappCalls.listSummaryProviders" => tx::read(&state.db, move |c| {
            let mut providers = Vec::new();
            for (provider, label, table) in SUMMARY_PROVIDERS {
                let id: Option<i64> = c
                    .query_row(
                        &format!(
                            r#"SELECT "id" FROM "{table}" WHERE "workspaceId" = ?1 ORDER BY "id" LIMIT 1"#
                        ),
                        params![ws],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(id) = id {
                    providers.push(
                        json!({ "id": id.to_string(), "provider": provider, "label": label }),
                    );
                }
            }
            Ok(providers)
        })
        .await
        .map(|providers| json(200, json!({ "providers": providers })))
        .map_err(|e| business_error(e, LISTING)),
        _ => {
            let id = id_input(&call)?;
            load_call(state, ws, id)
                .await
                .map(|c| {
                    let summary = c.ai_summary.map(|s| s.0).filter(|s| !s.is_null());
                    let provider = summary.as_ref().and(c.ai_summary_provider);
                    json(200, json!({ "summary": summary, "provider": provider }))
                })
                .map_err(|e| business_error(e, FINDING))
        }
    }
}

async fn list(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let activity = input.get("activity").cloned();
    let activity = input.enum_at(
        &key("activity"),
        activity.as_ref(),
        &["missed", "noReply"],
        false,
    );
    let inbox = input.get("inboxId").cloned();
    let inbox_id = digits_at(&mut input, &key("inboxId"), inbox.as_ref(), false);
    let agent = input.get("agentUserId").cloned();
    let agent_user_id = digits_at(&mut input, &key("agentUserId"), agent.as_ref(), false);
    let cursor = input.opt_string("cursor", Str::default());
    input.finish()?;
    let cursor = match cursor.as_deref().filter(|c| !c.is_empty()) {
        Some(text) => Some(decode_cursor(text).ok_or_else(|| {
            business_error(
                BusinessError::exception(
                    "Whatsapp calls: cursor failed to decode",
                    "invalidCursor",
                    400,
                ),
                LISTING_WITH_CURSOR,
            )
        })?),
        None => None,
    };
    // `CALL_ACTIVITY_FILTERS`.
    let direction = match activity.as_deref() {
        Some("missed") => Some("userInitiated".to_owned()),
        Some(_) => Some("businessInitiated".to_owned()),
        None => None,
    };
    let filters = WhatsappCallListFilters {
        outcome: activity.as_ref().map(|_| "failed".to_owned()),
        direction,
        inbox_id,
        agent_user_id,
        ongoing: false,
    };
    let query = ListForWorkspaceInput {
        workspace_id: call.workspace_id,
        scope: WhatsappCallHistoryScope::AllCalls,
        filters,
        cursor,
        limit: PAGE_SIZE + 1,
    };
    let mut rows = tx::read(&state.db, move |c| {
        Ok(calls::list_for_workspace(c, &query)?)
    })
    .await
    .map_err(|e| business_error(e, LISTING_WITH_CURSOR))?;
    let has_more = rows.len() as i64 > PAGE_SIZE;
    rows.truncate(PAGE_SIZE as usize);
    let next = rows.last().filter(|_| has_more).map(|row| {
        encode_cursor(&WhatsappCallListCursor {
            created_at: row.created_at_cursor.clone(),
            id: row.call.id,
        })
    });
    Ok(json(
        200,
        json!({ "data": rows.iter().map(row_json).collect::<Vec<_>>(), "nextCursor": next }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_kinds_follow_the_rule_order() {
        assert_eq!(call_kind("ringing", None, "userInitiated"), Some("ongoing"));
        assert_eq!(call_kind("failed", None, "userInitiated"), Some("missed"));
        assert_eq!(
            call_kind("failed", None, "businessInitiated"),
            Some("unanswered")
        );
        assert_eq!(
            call_kind("completed", Some("canceled"), "userInitiated"),
            Some("canceled")
        );
        assert_eq!(
            call_kind("completed", None, "businessInitiated"),
            Some("answeredOutbound")
        );
        assert_eq!(
            call_kind("rejected", None, "userInitiated"),
            Some("declined")
        );
    }

    #[test]
    fn the_cursor_round_trips_and_rejects_garbage() {
        let cursor = WhatsappCallListCursor {
            created_at: "2026-11-18T01:02:03.456Z".into(),
            id: 42,
        };
        assert_eq!(decode_cursor(&encode_cursor(&cursor)), Some(cursor));
        assert_eq!(decode_cursor("nope"), None);
        assert_eq!(
            decode_cursor(&STANDARD.encode(r#"{"createdAt":"x","id":"1"}"#)),
            None
        );
    }
}
