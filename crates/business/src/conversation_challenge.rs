//! `conversationService.{updateChallenge, consumeChallenge,
//! restoreChallengeIfAbsent, advanceStepChallengeAttempt}`
//! (`packages/business/src/conversation/service.ts`): the pending
//! `additionalAttributes.challenge` of a conversation. Each write is one
//! `UPDATE`, so a compare-and-set has exactly one winner.

use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use serde_json::Value;

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `setChallenge`.
const SET: &str = r#"json_set(COALESCE("additionalAttributes", '{}'), '$.challenge', json(?1))"#;
/// `dropChallenge`.
const DROP: &str = r#"json_remove("additionalAttributes", '$.challenge')"#;

/// `updateChallenge`: sets (`Some`) or drops (`None`) the challenge; 404
/// "Conversation not found" when no row matches.
pub async fn update_challenge(
    db: &Db,
    workspace_id: i64,
    conversation_id: i64,
    challenge: Option<Value>,
) -> Result<()> {
    let changed = tx::write(db, move |conn| {
        Ok(match &challenge {
            Some(challenge) => conn.execute(
                &format!(
                    r#"UPDATE "Conversation" SET "additionalAttributes" = {SET}
                       WHERE "id" = ?2 AND "workspaceId" = ?3"#
                ),
                params![challenge.to_string(), conversation_id, workspace_id],
            )?,
            None => conn.execute(
                &format!(
                    r#"UPDATE "Conversation" SET "additionalAttributes" = {DROP}
                       WHERE "id" = ?1 AND "workspaceId" = ?2"#
                ),
                params![conversation_id, workspace_id],
            )?,
        })
    })
    .await?;
    if changed == 0 {
        return Err(BusinessError::not_found("Conversation not found"));
    }
    Ok(())
}

/// `consumeChallenge`: clears the challenge only while it is this step's
/// cycle `challenge_id`; `true` for the one caller that cleared it.
pub async fn consume_challenge(
    db: &Db,
    workspace_id: i64,
    conversation_id: i64,
    step_id: &str,
    challenge_id: &str,
) -> Result<bool> {
    let (step_id, challenge_id) = (step_id.to_owned(), challenge_id.to_owned());
    tx::write(db, move |conn| {
        Ok(conn.execute(
            &format!(
                r#"UPDATE "Conversation" SET "additionalAttributes" = {DROP}
                   WHERE "id" = ?1 AND "workspaceId" = ?2
                     AND CAST(json_extract("additionalAttributes", '$.challenge.data.stepId') AS TEXT) = ?3
                     AND CAST(json_extract("additionalAttributes", '$.challenge.data.challengeId') AS TEXT) = ?4"#
            ),
            params![conversation_id, workspace_id, step_id, challenge_id],
        )? > 0)
    })
    .await
}

/// `restoreChallengeIfAbsent`: puts `challenge` back only while the
/// conversation has none; `false` when one exists (restore skipped).
pub async fn restore_challenge_if_absent(
    db: &Db,
    workspace_id: i64,
    conversation_id: i64,
    challenge: Value,
) -> Result<bool> {
    tx::write(db, move |conn| {
        Ok(conn.execute(
            &format!(
                r#"UPDATE "Conversation" SET "additionalAttributes" = {SET}
                   WHERE "id" = ?2 AND "workspaceId" = ?3
                     AND ("additionalAttributes" IS NULL
                          OR json_type("additionalAttributes", '$.challenge') IS NULL)"#
            ),
            params![challenge.to_string(), conversation_id, workspace_id],
        )? > 0)
    })
    .await
}

/// `advanceStepChallengeAttempt` input.
#[derive(Debug, Clone)]
pub struct AdvanceAttempt {
    pub workspace_id: i64,
    pub conversation_id: i64,
    pub step_id: String,
    pub challenge_id: String,
    pub from_attempts: i64,
    pub to_attempts: i64,
    pub last_attempt_at: i64,
}

/// `advanceStepChallengeAttempt`: compare-and-set of a step challenge's
/// attempt counter (step, cycle and current attempt must match);
/// `lastAttemptAt` moves with it, `startedAt` stays.
pub async fn advance_step_challenge_attempt(db: &Db, input: AdvanceAttempt) -> Result<bool> {
    tx::write(db, move |conn| {
        Ok(conn.execute(
            r#"UPDATE "Conversation"
               SET "additionalAttributes" = json_set("additionalAttributes",
                     '$.challenge.data.attempts', ?1, '$.challenge.data.lastAttemptAt', ?2)
               WHERE "id" = ?3 AND "workspaceId" = ?4
                 AND json_extract("additionalAttributes", '$.challenge.type') = 'step'
                 AND CAST(json_extract("additionalAttributes", '$.challenge.data.stepId') AS TEXT) = ?5
                 AND CAST(json_extract("additionalAttributes", '$.challenge.data.challengeId') AS TEXT) = ?6
                 AND CAST(json_extract("additionalAttributes", '$.challenge.data.attempts') AS INTEGER) = ?7"#,
            params![
                input.to_attempts,
                chatbotx_db::repo::iso(input.last_attempt_at),
                input.conversation_id,
                input.workspace_id,
                input.step_id,
                input.challenge_id,
                input.from_attempts,
            ],
        )? > 0)
    })
    .await
}
