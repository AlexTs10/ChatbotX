//! The heavy-step outcome record (`heavy-step-runner.ts`): one durable KV
//! row per step run, moved `pending` -> `writing` -> `succeeded` / `failed`,
//! plus the resume lease. Each Redis `eval` is one `update_json`
//! transaction, so the same races are safe.

use chatbotx_jobs::queues::heavy::HeavyOutputValue;
use chatbotx_kv::{DurableKv, JsonUpdate, KvError};
use ring::digest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const OUTCOME_KEY_PREFIX: &str = "heavy-step-outcome";
/// Below the smallest heavy-step backoff (15 s), so a retry sees a stale
/// write and replays it.
pub const STALE_WRITE_MS: i64 = 10_000;
/// The resume lease spans a whole `runFlowNode`.
pub const RESUME_LEASE_MS: i64 = 30_000;
const TIMED_OUT: &str = "Heavy step timed out";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub deadline_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_values: Option<Vec<HeavyOutputValue>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_lease_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_lease_until: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_status: Option<String>,
    /// `success` / `skip`; none = success (records before skip existed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_status: Option<String>,
    /// `pending`, `writing`, `succeeded` or `failed`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writing_started_at: Option<i64>,
}

impl Outcome {
    fn parse(value: Option<Value>) -> Option<Outcome> {
        serde_json::from_value(value?).ok()
    }

    fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    fn is_terminal(&self) -> bool {
        matches!(self.status.as_str(), "succeeded" | "failed")
    }

    fn time_out(&mut self) {
        self.status = "failed".into();
        self.error_message = Some(TIMED_OUT.into());
        self.writing_started_at = None;
        self.output_values = None;
    }
}

/// The per-run identity of the job id and the outcome key: stable across
/// retries of one step execution, new for every later execution.
pub struct Identity<'a> {
    pub action: &'a str,
    pub contact_inbox_id: &'a str,
    pub conversation_id: &'a str,
    pub parent_job_id: &'a str,
    pub step_id: &'a str,
}

impl Identity<'_> {
    /// `buildHeavyStepIdentityHash`: sha256 of the `JSON.stringify` (key
    /// order as written), 32 hex chars.
    pub fn hash(&self) -> String {
        let text = format!(
            "{{\"action\":{},\"contactInboxId\":{},\"conversationId\":{},\"parentJobId\":{},\"stepId\":{}}}",
            json!(self.action),
            json!(self.contact_inbox_id),
            json!(self.conversation_id),
            json!(self.parent_job_id),
            json!(self.step_id),
        );
        let hash = digest::digest(&digest::SHA256, text.as_bytes());
        let hex: String = hash.as_ref().iter().map(|b| format!("{b:02x}")).collect();
        hex[..32].to_owned()
    }

    pub fn job_id(&self) -> String {
        format!("heavy-{}-{}", self.action, self.hash())
    }

    pub fn outcome_key(&self) -> String {
        format!("{OUTCOME_KEY_PREFIX}:{}:{}", self.action, self.hash())
    }
}

/// `outcomeTtlMs`: the key outlives its own deadline.
pub fn outcome_ttl_ms(wait_timeout_ms: u64, completion_wait_ms: u64) -> u64 {
    (wait_timeout_ms * 4)
        .max(10 * 60_000)
        .max(completion_wait_ms * 2)
}

/// `readOutcome`: an invalid record reads as none.
pub async fn read(kv: &DurableKv, key: &str) -> Result<Option<Outcome>, KvError> {
    Ok(Outcome::parse(kv.get::<Value>(key).await?))
}

/// `ensurePending`: `SET NX` of a pending record, then the stored record.
pub async fn ensure_pending(
    kv: &DurableKv,
    key: &str,
    completion_wait_ms: u64,
    ttl_ms: u64,
) -> Result<Outcome, KvError> {
    kv.update_json(key, move |current, now| {
        if let Some(current) = current {
            // An unparsable record is the TS "not persisted" throw.
            let parsed = Outcome::parse(Some(current));
            return (JsonUpdate::Keep, parsed);
        }
        let fresh = Outcome {
            deadline_at: now + completion_wait_ms as i64,
            status: "pending".into(),
            ..Outcome::default()
        };
        (JsonUpdate::WriteTtl(fresh.to_value(), ttl_ms), Some(fresh))
    })
    .await?
    .ok_or_else(|| {
        KvError::Db(chatbotx_db::DbError::other(
            "Heavy step outcome was not persisted",
        ))
    })
}

/// What a `transition` to `writing` answers.
#[derive(Debug, Clone, PartialEq)]
pub enum Transition {
    Missing,
    /// The record already was (or now is) this status.
    Status(String),
    Busy,
    /// The write is claimed; replay these values (the first claim's).
    Claimed(Option<Vec<HeavyOutputValue>>),
}

/// `transitionOutcome`.
pub async fn transition(
    kv: &DurableKv,
    key: &str,
    next: &'static str,
    error_message: Option<String>,
    output_values: Option<Vec<HeavyOutputValue>>,
    result_status: Option<&'static str>,
) -> Result<Transition, KvError> {
    kv.update_json(key, move |current, now| {
        if current.is_none() {
            return (JsonUpdate::Keep, Transition::Missing);
        }
        let Some(mut obj) = Outcome::parse(current) else {
            return (JsonUpdate::Keep, Transition::Missing);
        };
        if obj.is_terminal() {
            return (JsonUpdate::Keep, Transition::Status(obj.status));
        }
        if now > obj.deadline_at {
            obj.time_out();
            return (
                JsonUpdate::Write(obj.to_value()),
                Transition::Status("failed".into()),
            );
        }
        if next == "writing" {
            let stale = obj
                .writing_started_at
                .is_none_or(|at| now - at > STALE_WRITE_MS);
            if obj.status == "pending" || (obj.status == "writing" && stale) {
                obj.status = "writing".into();
                obj.writing_started_at = Some(now);
                // A stale reclaim keeps the values the first claim stored.
                if obj.output_values.is_none() {
                    obj.output_values = output_values;
                }
                let values = obj.output_values.clone();
                return (
                    JsonUpdate::Write(obj.to_value()),
                    Transition::Claimed(values),
                );
            }
            if obj.status == "writing" {
                return (JsonUpdate::Keep, Transition::Busy);
            }
            return (JsonUpdate::Keep, Transition::Status(obj.status));
        }
        obj.status = next.into();
        obj.writing_started_at = None;
        obj.output_values = None;
        if next == "failed" {
            obj.error_message = Some(error_message.unwrap_or_else(|| "Heavy step failed".into()));
        }
        if next == "succeeded" {
            obj.result_status = Some(result_status.unwrap_or("success").into());
        }
        (
            JsonUpdate::Write(obj.to_value()),
            Transition::Status(next.into()),
        )
    })
    .await
}

/// `claimHeavyStepResume`: `claimed`, `pending` or `resumed`.
pub async fn claim_resume(
    kv: &DurableKv,
    key: &str,
    token: String,
) -> Result<&'static str, KvError> {
    kv.update_json(key, move |current, now| {
        let Some(mut obj) = current.and_then(|c| Outcome::parse(Some(c))) else {
            return (JsonUpdate::Keep, "resumed");
        };
        if matches!(obj.status.as_str(), "pending" | "writing") {
            if now < obj.deadline_at {
                return (JsonUpdate::Keep, "pending");
            }
            obj.time_out();
        }
        if obj.resume_status.as_deref() == Some("resumed") {
            return (JsonUpdate::Keep, "resumed");
        }
        if obj.resume_status.as_deref() == Some("resuming")
            && obj.resume_lease_until.unwrap_or(0) > now
        {
            return (JsonUpdate::Keep, "pending");
        }
        obj.resume_status = Some("resuming".into());
        obj.resume_lease_token = Some(token);
        obj.resume_lease_until = Some(now + RESUME_LEASE_MS);
        (JsonUpdate::Write(obj.to_value()), "claimed")
    })
    .await
}

/// `finishHeavyStepResume`.
pub async fn finish_resume(
    kv: &DurableKv,
    key: &str,
    token: String,
    succeeded: bool,
) -> Result<(), KvError> {
    kv.update_json(key, move |current, _| {
        let Some(mut obj) = current.and_then(|c| Outcome::parse(Some(c))) else {
            return (JsonUpdate::Keep, ());
        };
        if obj.resume_lease_token.as_deref() != Some(token.as_str()) {
            return (JsonUpdate::Keep, ());
        }
        obj.resume_status = succeeded.then(|| "resumed".into());
        obj.resume_lease_token = None;
        obj.resume_lease_until = None;
        (JsonUpdate::Write(obj.to_value()), ())
    })
    .await
}

/// `HeavyStepAttempt`.
#[derive(Debug, Clone, PartialEq)]
pub enum Attempt {
    Run,
    Replay(Vec<HeavyOutputValue>),
    Skip,
}

/// `resolveHeavyStepAttempt`: a stale `writing` record with stored values
/// replays them (no second provider bill); anything else not pending skips.
pub async fn resolve_attempt(kv: &DurableKv, key: &str, now: i64) -> Result<Attempt, KvError> {
    let Some(outcome) = read(kv, key).await? else {
        return Ok(Attempt::Skip);
    };
    if outcome.status == "pending" {
        return Ok(Attempt::Run);
    }
    let stale = outcome.status == "writing"
        && outcome
            .writing_started_at
            .is_none_or(|at| now - at > STALE_WRITE_MS);
    match outcome.output_values {
        Some(values) if stale => Ok(Attempt::Replay(values)),
        _ => Ok(Attempt::Skip),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chatbotx_kv::{Clock as _, TestClock};

    use super::*;

    async fn kv() -> (DurableKv, Arc<TestClock>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "heavy-outcome-{}-{}",
            std::process::id(),
            chatbotx_db::repo::new_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = chatbotx_db::Db::open(dir.join("t.db")).unwrap();
        db.migrate().await.unwrap();
        let clock = Arc::new(TestClock::new(1_000_000));
        (DurableKv::new(Arc::new(db), clock.clone()), clock, dir)
    }

    fn value(v: &str) -> Vec<HeavyOutputValue> {
        vec![HeavyOutputValue {
            custom_field_id: "f".into(),
            value: v.into(),
        }]
    }

    #[tokio::test]
    async fn pending_write_succeed_then_resume_once() {
        let (kv, _clock, _dir) = kv().await;
        let o = ensure_pending(&kv, "k", 60_000, 600_000).await.unwrap();
        assert_eq!(o.status, "pending");
        assert_eq!(
            resolve_attempt(&kv, "k", 1_000_000).await.unwrap(),
            Attempt::Run
        );
        let claim = transition(&kv, "k", "writing", None, Some(value("a")), None)
            .await
            .unwrap();
        assert_eq!(claim, Transition::Claimed(Some(value("a"))));
        // A second claim inside the stale window is busy.
        let busy = transition(&kv, "k", "writing", None, Some(value("b")), None)
            .await
            .unwrap();
        assert_eq!(busy, Transition::Busy);
        transition(&kv, "k", "succeeded", None, None, Some("skip"))
            .await
            .unwrap();
        let done = read(&kv, "k").await.unwrap().unwrap();
        assert_eq!(
            (done.status.as_str(), done.result_status.as_deref()),
            ("succeeded", Some("skip"))
        );
        assert_eq!(done.output_values, None);
        // Terminal: a late failure does not overwrite it.
        let late = transition(&kv, "k", "failed", Some("x".into()), None, None)
            .await
            .unwrap();
        assert_eq!(late, Transition::Status("succeeded".into()));

        assert_eq!(
            claim_resume(&kv, "k", "t1".into()).await.unwrap(),
            "claimed"
        );
        assert_eq!(
            claim_resume(&kv, "k", "t2".into()).await.unwrap(),
            "pending"
        );
        // A foreign token does not release the lease.
        finish_resume(&kv, "k", "t2".into(), true).await.unwrap();
        assert_eq!(
            claim_resume(&kv, "k", "t3".into()).await.unwrap(),
            "pending"
        );
        finish_resume(&kv, "k", "t1".into(), true).await.unwrap();
        assert_eq!(
            claim_resume(&kv, "k", "t4".into()).await.unwrap(),
            "resumed"
        );
    }

    #[tokio::test]
    async fn stale_write_replays_the_first_values_and_deadline_fails() {
        let (kv, clock, _dir) = kv().await;
        ensure_pending(&kv, "k", 60_000, 600_000).await.unwrap();
        transition(&kv, "k", "writing", None, Some(value("first")), None)
            .await
            .unwrap();
        clock.advance(STALE_WRITE_MS + 1);
        assert_eq!(
            resolve_attempt(&kv, "k", clock.now_ms()).await.unwrap(),
            Attempt::Replay(value("first"))
        );
        let reclaim = transition(&kv, "k", "writing", None, Some(value("second")), None)
            .await
            .unwrap();
        assert_eq!(reclaim, Transition::Claimed(Some(value("first"))));
        // A failed resume releases the lease for a later claim.
        clock.advance(60_000);
        assert_eq!(claim_resume(&kv, "k", "t".into()).await.unwrap(), "claimed");
        let timed_out = read(&kv, "k").await.unwrap().unwrap();
        assert_eq!(timed_out.status, "failed");
        assert_eq!(timed_out.error_message.as_deref(), Some(TIMED_OUT));
        finish_resume(&kv, "k", "t".into(), false).await.unwrap();
        assert_eq!(claim_resume(&kv, "k", "u".into()).await.unwrap(), "claimed");
    }

    #[test]
    fn identity_hash_is_stable_and_keyed() {
        let id = Identity {
            action: "aiGenerateImage",
            contact_inbox_id: "5",
            conversation_id: "6",
            parent_job_id: "flow-1",
            step_id: "7",
        };
        assert_eq!(id.hash().len(), 32);
        assert_eq!(id.job_id(), format!("heavy-aiGenerateImage-{}", id.hash()));
        assert!(
            id.outcome_key()
                .starts_with("heavy-step-outcome:aiGenerateImage:")
        );
        let other = Identity { step_id: "8", ..id };
        assert_ne!(
            other.hash(),
            Identity {
                step_id: "7",
                ..other
            }
            .hash()
        );
    }
}
