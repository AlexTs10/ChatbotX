//! Q.P6-U5 (COVERAGE E12, D1): the email recipient lifecycle.
//! `email-topic/tracking-service.ts` (`emailTrackingService`),
//! `email-topic/click-url.ts`, the `AnalyticsEmailTopic` writes of
//! `email-topic-stats.repository.ts`, `findAnalyticsWorkspaceIdByToken` and
//! `leaderboard/email-sources.ts` (`awardLeaderboardEmailEngagement`).
//!
//! The raw counters, the first-time transition, the contact activity row and
//! the topic counter of one phase commit in one transaction. The topic
//! counter is the last statement (it is one hot row per topic).

use chatbotx_db::Db;
use chatbotx_db::repo::contact_activity::{ContactActivityRecord, upsert_activities};
use chatbotx_db::repo::partials::contact_activity::NO_REFERENCE_ID;
use chatbotx_db::repo::workspace::lock_workspace_for_child_write;
use chatbotx_db::repo::{new_id, now_ms, random_uuid};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use chatbotx_jobs::queues::integration::{
    IntegrationJob, IntegrationJobAction, enqueue_integration_job,
};
use chatbotx_jobs::{Enqueuer, JobOptions};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest as _, Sha256};

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `EmailTrackingPhase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Delivered,
    Opened,
    Clicked,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Delivered => "delivered",
            Phase::Opened => "opened",
            Phase::Clicked => "clicked",
        }
    }

    pub fn parse(value: &str) -> Option<Phase> {
        match value {
            "delivered" => Some(Phase::Delivered),
            "opened" => Some(Phase::Opened),
            "clicked" => Some(Phase::Clicked),
            _ => None,
        }
    }

    /// `PHASES[phase]`: the activity type, the first-time column, the topic
    /// counter and the raw (count, last) columns.
    fn columns(
        self,
    ) -> (
        &'static str,
        &'static str,
        &'static str,
        Option<(&'static str, &'static str)>,
    ) {
        match self {
            Phase::Delivered => ("messageDelivered", "deliveredAt", "deliveredsTotal", None),
            Phase::Opened => (
                "messageOpened",
                "firstSeenAt",
                "seensTotal",
                Some(("seenCount", "lastSeenAt")),
            ),
            Phase::Clicked => (
                "messageClicked",
                "firstClickedAt",
                "clicksTotal",
                Some(("clickCount", "lastClickedAt")),
            ),
        }
    }
}

/// `CreateEmailRecipientInput`.
#[derive(Debug, Clone, Default)]
pub struct NewRecipient {
    /// `None` for an email without a topic ("No Topic").
    pub topic_id: Option<i64>,
    pub workspace_id: i64,
    pub email: String,
    pub contact_id: Option<i64>,
    pub conversation_id: Option<i64>,
    pub contact_inbox_id: Option<i64>,
}

/// `PhaseActivity`.
struct Activity {
    activity_type: &'static str,
    contact_id: Option<i64>,
    /// `(counter, topicId)`.
    counter: Option<(&'static str, i64)>,
    occurred_at: i64,
    topic_reference_id: Option<i64>,
    workspace_id: i64,
}

/// `recordActivity`: the contact's activity row (when the contact still
/// exists), then the topic counter as the last statement.
fn record_activity(conn: &Connection, activity: &Activity) -> Result<()> {
    if let Some(contact_id) = activity.contact_id
        && lock_workspace_for_child_write(conn, activity.workspace_id)?
    {
        upsert_activities(
            conn,
            activity.workspace_id,
            &[ContactActivityRecord {
                activity_type: activity.activity_type.to_owned(),
                contact_id,
                occurred_at: activity.occurred_at,
                parent_reference_id: 0,
                reference_id: activity.topic_reference_id.unwrap_or(NO_REFERENCE_ID),
                reference_type: "emailTopic".to_owned(),
            }],
        )?;
    }
    if let Some((counter, topic_id)) = activity.counter {
        conn.execute(
            &format!(
                r#"UPDATE "EmailTopic" SET "{counter}" = "{counter}" + 1 WHERE "id" = ?1 AND "workspaceId" = ?2"#
            ),
            params![topic_id, activity.workspace_id],
        )?;
    }
    Ok(())
}

/// `createRecipient`: the recipient row (and its token) and "sent".
pub async fn create_recipient(db: &Db, input: NewRecipient) -> Result<String> {
    let token = random_uuid();
    let out = token.clone();
    tx::write(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "AnalyticsEmailTopic" ("id", "token", "topicId", "topicReferenceId", "workspaceId", "email", "contactId", "conversationId", "contactInboxId")
               VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, ?7, ?8)"#,
            params![
                new_id(),
                token,
                input.topic_id,
                input.workspace_id,
                input.email,
                input.contact_id,
                input.conversation_id,
                input.contact_inbox_id
            ],
        )?;
        record_activity(
            tx,
            &Activity {
                activity_type: "messageSent",
                contact_id: input.contact_id,
                counter: input.topic_id.map(|id| ("sendsTotal", id)),
                occurred_at: now_ms(),
                topic_reference_id: input.topic_id,
                workspace_id: input.workspace_id,
            },
        )
    })
    .await?;
    Ok(out)
}

/// `markFailed`.
pub async fn mark_failed(db: &Db, token: &str) -> Result<()> {
    let token = token.to_owned();
    tx::write(db, move |tx| {
        tx.execute(
            r#"UPDATE "AnalyticsEmailTopic" SET "failedAt" = ?2 WHERE "token" = ?1"#,
            params![token, now_ms()],
        )?;
        Ok(())
    })
    .await
}

/// `recordTransition`: one phase. Idempotent: a repeat moves only the raw
/// counters.
pub async fn record_transition(db: &Db, phase: Phase, token: &str) -> Result<()> {
    let token = token.to_owned();
    let (activity_type, first, counter, raw) = phase.columns();
    tx::write(db, move |tx| {
        let now = now_ms();
        if let Some((count, last)) = raw {
            tx.execute(
                &format!(
                    r#"UPDATE "AnalyticsEmailTopic" SET "{count}" = "{count}" + 1, "{last}" = ?2 WHERE "token" = ?1"#
                ),
                params![token, now],
            )?;
        }
        let row = tx
            .query_row(
                &format!(
                    r#"UPDATE "AnalyticsEmailTopic" SET "{first}" = ?2 WHERE "token" = ?1 AND "{first}" IS NULL
                       RETURNING "contactId", "{first}", "topicId", "topicReferenceId", "workspaceId""#
                ),
                params![token, now],
                |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((contact_id, at, topic_id, reference_id, workspace_id)) = row else {
            return Ok(());
        };
        record_activity(
            tx,
            &Activity {
                activity_type,
                contact_id,
                // NULL once the topic was deleted: no counter left to bump.
                counter: topic_id.map(|id| (counter, id)),
                occurred_at: at.unwrap_or(now),
                topic_reference_id: reference_id.or(topic_id),
                workspace_id,
            },
        )
    })
    .await
}

/// `emailTrackingTransitionJobId`: only a digest of the token (job ids
/// appear in logs); no `:`.
pub fn transition_job_id(token: &str, phase: Phase) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("email-tracking-{}-{}", &hex[..32], phase.as_str())
}

/// `recordTransitionOrEnqueueRepair`: never fails; a failed transition is
/// handed to the `recordEmailTrackingTransition` repair job.
pub async fn record_transition_or_enqueue_repair(
    db: &Db,
    jobs: Option<&dyn Enqueuer>,
    phase: Phase,
    token: &str,
    workspace_id: Option<i64>,
    link: Option<&str>,
) {
    let Err(err) = record_transition(db, phase, token).await else {
        return;
    };
    tracing::error!(err = %err, phase = phase.as_str(), "email-tracking: transition failed, enqueueing repair");
    let Some(jobs) = jobs else {
        tracing::error!(
            phase = phase.as_str(),
            "email-tracking: failed to enqueue repair"
        );
        return;
    };
    let job = IntegrationJob {
        action: IntegrationJobAction::RecordEmailTrackingTransition,
        data: json!({
            "phase": phase.as_str(),
            "token": token,
            "workspaceId": workspace_id.map(|id| id.to_string()),
            "link": link,
        }),
    };
    let opts = JobOptions {
        job_id: Some(transition_job_id(token, phase)),
        ..JobOptions::default()
    };
    if let Err(err) = enqueue_integration_job(jobs, job, Some(opts)).await {
        tracing::error!(err = %err, phase = phase.as_str(), "email-tracking: failed to enqueue repair");
    }
}

/// `findAnalyticsWorkspaceIdByToken`.
pub async fn workspace_id_by_token(db: &Db, token: &str) -> Result<Option<i64>> {
    let token = token.to_owned();
    tx::read(db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT "workspaceId" FROM "AnalyticsEmailTopic" WHERE "token" = ?1"#,
                [token],
                |r| r.get(0),
            )
            .optional()?)
    })
    .await
}

/// `awardLeaderboardEmailEngagement`: scores a topic email's first open /
/// first click (per link) on every SMTP inbox of the workspace. Call it after
/// the transition. Never fails.
pub async fn award_leaderboard_engagement(
    db: &Db,
    jobs: Option<&dyn Enqueuer>,
    token: &str,
    open: bool,
    link: Option<&str>,
) {
    let token = token.to_owned();
    let found = tx::read(db, move |conn| {
        let row = conn
            .query_row(
                r#"SELECT "id", "workspaceId", "contactId", "firstSeenAt", "firstClickedAt", coalesce("topicReferenceId", "topicId")
                   FROM "AnalyticsEmailTopic" WHERE "token" = ?1"#,
                [token],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                        r.get::<_, Option<i64>>(4)?,
                        r.get::<_, Option<i64>>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some(row) = row else { return Ok(None) };
        let mut stmt =
            conn.prepare(r#"SELECT "inboxId" FROM "IntegrationSmtp" WHERE "workspaceId" = ?1"#)?;
        let inboxes = stmt
            .query_map([row.1], |r| r.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Some((row, inboxes)))
    })
    .await;
    let ((id, workspace_id, contact_id, seen, clicked, topic), inboxes) = match found {
        Ok(Some(found)) => found,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(err = %err, "failed to award leaderboard email engagement");
            return;
        }
    };
    let (Some(contact_id), Some(_), Some(event_at)) =
        (contact_id, topic, if open { seen } else { clicked })
    else {
        return;
    };
    let Some(jobs) = jobs else { return };
    let source_ref = match link.filter(|_| !open) {
        Some(link) => {
            let digest = Sha256::digest(link.as_bytes());
            let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
            format!("{id}:{}", &hex[..16])
        }
        None => id.to_string(),
    };
    for inbox_id in inboxes {
        let award = crate::leaderboard::scoring::Award {
            workspace_id,
            contact_id,
            inbox_id: Some(inbox_id),
            minigame_id: None,
            rule: if open { "emailOpen" } else { "emailClick" }.to_owned(),
            source_ref: source_ref.clone(),
            event_at,
            context: serde_json::Value::Null,
        };
        crate::leaderboard::service::enqueue(jobs, db, false, &award).await;
    }
}

// ---- click-url.ts ------------------------------------------------------------

/// `TOKEN_TTL_MS`: one year (marketing mails are opened late).
const CLICK_TOKEN_TTL_MS: i64 = 365 * 24 * 60 * 60 * 1000;

/// `EmailClickPayload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClickPayload {
    pub url: String,
    pub exp: i64,
    #[serde(
        rename = "workspaceId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub workspace_id: Option<String>,
}

fn click_error() -> BusinessError {
    BusinessError::exception("Invalid email click token", "badRequest", 400)
}

/// `signEmailClickUrl`: the destination sealed into an authenticated,
/// base64url token (the click route trusts no raw URL: no open redirect).
pub fn sign_click_url(url: &str, workspace_id: i64) -> Result<String> {
    use base64::Engine as _;
    let sealed = crate::meta_catalog::process_encryption()
        .encrypt_object(
            &ClickPayload {
                url: url.to_owned(),
                exp: now_ms() + CLICK_TOKEN_TTL_MS,
                workspace_id: Some(workspace_id.to_string()),
            },
            None,
        )
        .map_err(|e| BusinessError::exception(e.to_string(), "internalServerError", 500))?;
    let json = serde_json::to_string(&sealed).map_err(|_| click_error())?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
}

/// `verifyEmailClickToken`: fails for a malformed, tampered or expired token.
pub fn verify_click_token(token: &str) -> Result<ClickPayload> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(token.trim_end_matches('='))
        .map_err(|_| click_error())?;
    let sealed: chatbotx_core::encryption::EncryptedData =
        serde_json::from_slice(&bytes).map_err(|_| click_error())?;
    let payload: ClickPayload = crate::meta_catalog::process_encryption()
        .decrypt_object(&sealed, None)
        .map_err(|_| click_error())?;
    if payload.exp < now_ms() {
        return Err(click_error());
    }
    Ok(payload)
}
