//! WhatsApp Business Calling over VoIP (`packages/business/src/whatsapp-call/`
//! `voip-call-control.ts`, `voip-call-service.ts`, `voip-signaling-service.ts`,
//! `voip-sdp.ts`, `ring-targets.ts`, `call-access-service.ts`,
//! `turn-credential.ts`, `voip-turn-credential-service.ts`,
//! `call-permission-service.ts`) and the Graph calling API
//! (`integrations/whatsapp/src/api/calling.ts`, `lib/calling-recording.ts`).
//!
//! The call-control record, the SDP offer and the outbound SDP answer live in
//! the durable kv (SQLite, the original's Redis `casStore`), so the API and the
//! worker share them. Every phase change is a fenced compare-and-swap.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use chatbotx_db::Db;
use chatbotx_db::repo::models::{ContactInbox, IntegrationWhatsapp, WhatsappCall};
use chatbotx_db::repo::partials::whatsapp_call::{
    WhatsappCallTerminalStatus, resolve_whatsapp_call_outcome,
};
use chatbotx_db::repo::whatsapp_call::{self as calls, FinalizeInput};
use chatbotx_db::repo::{now_ms, random_uuid};
use chatbotx_jobs::{Enqueuer, JobOptions, QueueName};
use chatbotx_kv::{CasStore as _, DurableKv, KvError, SystemClock};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;
use crate::whatsapp::{DEFAULT_API_VERSION, should_address_by_source_user_id};
use crate::workspace_member::has_permission;

// ---- control record ---------------------------------------------------------

/// `VoipCallPhase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Reserved,
    Answering,
    Accepted,
    Terminated,
    Dialing,
    Ringing,
}

/// `ALLOWED_TRANSITIONS` / `isTransitionAllowed`.
pub fn is_transition_allowed(from: Phase, to: Phase) -> bool {
    use Phase::*;
    matches!(
        (from, to),
        (Reserved, Answering | Terminated)
            | (Answering, Accepted | Terminated)
            | (Accepted, Terminated)
            | (Dialing, Ringing | Accepted | Terminated)
            | (Ringing, Accepted | Terminated)
    )
}

/// `VoipCallControl` at `voip:ctrl:<wacid>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Control {
    /// Empty while ring-all runs; the winner after `claim_for_answer`.
    pub reserved_user_id: String,
    pub phase: Phase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// Epoch ms: Meta's answer deadline.
    pub deadline_at: i64,
    pub fence_token: String,
}

pub fn control_key(wacid: &str) -> String {
    format!("voip:ctrl:{wacid}")
}
pub fn offer_key(wacid: &str) -> String {
    format!("voip:offer:{wacid}")
}
pub fn outbound_answer_key(attempt_id: &str) -> String {
    format!("voip:out:answer:{attempt_id}")
}

pub const MIN_RESERVATION_TTL_MS: i64 = 5000;
pub const VOIP_ANSWER_DEADLINE_MS: i64 = 55_000;
pub const ACTIVE_CALL_CONTROL_TTL_MS: i64 = 4 * 60 * 60 * 1000;
pub const TERMINATED_CONTROL_TTL_MS: i64 = 60_000;
pub const ACTIVE_CALL_LIVENESS_STALE_MS: i64 = 30 * 60 * 1000;
pub const STRANDED_CALL_RECOVERED_LAST_ERROR: &str = "stranded-accepted-recovered-on-dial";
pub const ACTIVE_CALL_ROW_TOUCH_INTERVAL_MS: i64 = 2 * 60 * 1000;
pub const VOIP_ANSWER_DEADLINE_SAFETY_MARGIN_MS: i64 = 3000;
pub const VOIP_CONTROL_EXPIRY_MARGIN_MS: i64 = 20_000;
/// `OUTBOUND_DIAL_DEADLINE_MS`.
pub const OUTBOUND_DIAL_DEADLINE_MS: i64 = 60_000;
/// `CALL_CANCELED_BY_BUSINESS_LAST_ERROR`.
pub const CALL_CANCELED_BY_BUSINESS_LAST_ERROR: &str = "canceled_by_business";
/// `META_RECORDING_ANNOUNCEMENT_REJECTED`.
pub const META_RECORDING_ANNOUNCEMENT_REJECTED: &str = "meta-rejected-recording-announcement";

/// `isAnswerDeadlineExpired`.
pub fn is_answer_deadline_expired(deadline_at: i64) -> bool {
    now_ms() + VOIP_ANSWER_DEADLINE_SAFETY_MARGIN_MS >= deadline_at
}

/// `remainingTtlMs`.
pub fn remaining_ttl_ms(deadline_at: i64) -> i64 {
    (deadline_at - now_ms()).max(MIN_RESERVATION_TTL_MS)
}

/// The Graph action that ends a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndAction {
    Reject,
    Terminate,
}

impl EndAction {
    pub fn as_str(self) -> &'static str {
        match self {
            EndAction::Reject => "reject",
            EndAction::Terminate => "terminate",
        }
    }
}

/// `EndVoipCallResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ended {
    pub from_phase: Phase,
    pub graph_action: EndAction,
    pub terminal_status: WhatsappCallTerminalStatus,
}

/// `VOIP_END_OUTCOME_BY_PHASE`; `None` for `terminated`.
pub fn end_outcome(phase: Phase) -> Option<Ended> {
    use WhatsappCallTerminalStatus as S;
    let (graph_action, terminal_status) = match phase {
        Phase::Reserved => (EndAction::Reject, S::Rejected),
        Phase::Answering | Phase::Dialing | Phase::Ringing => (EndAction::Terminate, S::Failed),
        Phase::Accepted => (EndAction::Terminate, S::Completed),
        Phase::Terminated => return None,
    };
    Some(Ended {
        from_phase: phase,
        graph_action,
        terminal_status,
    })
}

/// `resolveEndOutcomeWithoutControl` (`LIVE_CALL_PHASE_BY_STATUS`).
pub fn end_outcome_without_control(call: &WhatsappCall) -> Option<Ended> {
    let outbound = call.direction == "businessInitiated";
    let phase = match call.status.as_str() {
        "ringing" if outbound => Phase::Dialing,
        "ringing" => Phase::Reserved,
        "accepted" => Phase::Accepted,
        _ => return None,
    };
    end_outcome(phase)
}

/// `isCallEnded`.
pub fn is_call_ended(call: &WhatsappCall) -> bool {
    calls::WHATSAPP_CALL_TERMINAL_STATUSES.contains(&call.status.as_str())
}

pub fn terminal_status_str(status: WhatsappCallTerminalStatus) -> &'static str {
    match status {
        WhatsappCallTerminalStatus::Completed => "completed",
        WhatsappCallTerminalStatus::Rejected => "rejected",
        WhatsappCallTerminalStatus::Failed => "failed",
    }
}

// ---- the store --------------------------------------------------------------

fn kv_error(error: KvError) -> BusinessError {
    BusinessError::Repo(chatbotx_db::repo::Error::msg(error.to_string()))
}

fn ttl(ms: i64) -> u64 {
    u64::try_from(ms.max(1)).unwrap_or(1)
}

/// The durable call-control store (`casStore`).
#[derive(Clone)]
pub struct VoipStore {
    kv: Arc<DurableKv>,
}

fn as_map(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

impl VoipStore {
    pub fn new(db: Arc<Db>) -> VoipStore {
        VoipStore {
            kv: Arc::new(DurableKv::new(db, Arc::new(SystemClock))),
        }
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let value = self.kv.get_json(key).await.map_err(kv_error)?;
        Ok(value.and_then(|v| serde_json::from_value(v).ok()))
    }

    async fn set_if_absent(&self, key: &str, value: &Value, ttl_ms: i64) -> Result<bool> {
        self.kv
            .set_if_absent(key, value, ttl(ttl_ms))
            .await
            .map_err(kv_error)
    }

    async fn swap(&self, key: &str, expected: &Value, next: &Control, ttl_ms: i64) -> Result<bool> {
        let expected = as_map(expected);
        let next = serde_json::to_value(next).unwrap_or_default();
        self.kv
            .compare_and_swap(key, Some(&expected), &next, ttl(ttl_ms))
            .await
            .map_err(kv_error)
    }

    async fn swap_from(
        &self,
        key: &str,
        current: &Control,
        next: &Control,
        ttl_ms: i64,
    ) -> Result<bool> {
        let expected = serde_json::to_value(current).unwrap_or_default();
        self.swap(key, &expected, next, ttl_ms).await
    }

    async fn del(&self, key: &str) -> Result<()> {
        self.kv.del(key).await.map_err(kv_error)
    }

    pub async fn read_control(&self, wacid: &str) -> Result<Option<Control>> {
        self.get(&control_key(wacid)).await
    }

    /// `claimUnreachable`: `SET NX` a `terminated` control.
    pub async fn claim_unreachable(&self, wacid: &str, deadline_at: i64) -> Result<bool> {
        let control = Control {
            reserved_user_id: String::new(),
            phase: Phase::Terminated,
            direction: None,
            deadline_at,
            fence_token: random_uuid(),
        };
        let value = serde_json::to_value(&control).unwrap_or_default();
        self.set_if_absent(&control_key(wacid), &value, TERMINATED_CONTROL_TTL_MS)
            .await
    }

    /// `reserveIncomingCall`: `true` = reserved (ring), `false` = already progressed.
    pub async fn reserve_incoming(&self, wacid: &str, deadline_at: i64) -> Result<bool> {
        let key = control_key(wacid);
        if let Some(existing) = self.get::<Control>(&key).await? {
            return Ok(existing.phase == Phase::Reserved);
        }
        let control = Control {
            reserved_user_id: String::new(),
            phase: Phase::Reserved,
            direction: None,
            deadline_at,
            fence_token: random_uuid(),
        };
        let value = serde_json::to_value(&control).unwrap_or_default();
        let ttl_ms = remaining_ttl_ms(deadline_at) + VOIP_CONTROL_EXPIRY_MARGIN_MS;
        if self.set_if_absent(&key, &value, ttl_ms).await? {
            return Ok(true);
        }
        Ok(self
            .get::<Control>(&key)
            .await?
            .is_some_and(|winner| winner.phase == Phase::Reserved))
    }

    /// `claimForAnswer` (`reserved -> answering`): the fence token, or `None`.
    pub async fn claim_for_answer(&self, wacid: &str, user_id: &str) -> Result<Option<String>> {
        let key = control_key(wacid);
        let Some(current) = self.get::<Control>(&key).await? else {
            return Ok(None);
        };
        if !(current.reserved_user_id.is_empty()
            && is_transition_allowed(current.phase, Phase::Answering))
        {
            return Ok(None);
        }
        let next = Control {
            phase: Phase::Answering,
            reserved_user_id: user_id.to_owned(),
            ..current.clone()
        };
        let ttl_ms = remaining_ttl_ms(current.deadline_at) + VOIP_CONTROL_EXPIRY_MARGIN_MS;
        Ok(self
            .swap_from(&key, &current, &next, ttl_ms)
            .await?
            .then_some(current.fence_token))
    }

    /// `commitAccepted` (`answering -> accepted`, fenced).
    pub async fn commit_accepted(&self, wacid: &str, fence_token: &str) -> Result<bool> {
        let key = control_key(wacid);
        let Some(current) = self.get::<Control>(&key).await? else {
            return Ok(false);
        };
        if current.fence_token != fence_token
            || !is_transition_allowed(current.phase, Phase::Accepted)
        {
            return Ok(false);
        }
        let next = Control {
            phase: Phase::Accepted,
            ..current.clone()
        };
        self.swap_from(&key, &current, &next, ACTIVE_CALL_CONTROL_TTL_MS)
            .await
    }

    /// `releaseClaim` (`answering -> reserved`, fenced rollback).
    pub async fn release_claim(&self, wacid: &str, fence_token: &str) -> Result<bool> {
        let key = control_key(wacid);
        let Some(current) = self.get::<Control>(&key).await? else {
            return Ok(false);
        };
        if current.phase != Phase::Answering || current.fence_token != fence_token {
            return Ok(false);
        }
        let next = Control {
            phase: Phase::Reserved,
            reserved_user_id: String::new(),
            ..current.clone()
        };
        let ttl_ms = remaining_ttl_ms(current.deadline_at) + VOIP_CONTROL_EXPIRY_MARGIN_MS;
        self.swap_from(&key, &current, &next, ttl_ms).await
    }

    /// `endCall`: the single termination path; `None` = nothing to end.
    pub async fn end_call(&self, wacid: &str, allow_from_accepted: bool) -> Result<Option<Ended>> {
        let key = control_key(wacid);
        let Some(current) = self.get::<Control>(&key).await? else {
            return Ok(None);
        };
        if !is_transition_allowed(current.phase, Phase::Terminated)
            || (current.phase == Phase::Accepted && !allow_from_accepted)
        {
            return Ok(None);
        }
        let next = Control {
            phase: Phase::Terminated,
            ..current.clone()
        };
        if !self
            .swap_from(&key, &current, &next, TERMINATED_CONTROL_TTL_MS)
            .await?
        {
            return Ok(None);
        }
        Ok(end_outcome(current.phase))
    }

    /// `startOutboundDial`: the initiator owns the control from the start.
    pub async fn start_outbound_dial(
        &self,
        db: &Db,
        wacid: &str,
        initiator_user_id: &str,
        deadline_at: i64,
    ) -> Result<Option<Control>> {
        let key_wacid = wacid.to_owned();
        let existing = tx::read(db, move |c| Ok(calls::find_by_wacid(c, &key_wacid)?)).await?;
        let phase = if existing.is_some_and(|c| c.status == "accepted") {
            Phase::Accepted
        } else {
            Phase::Dialing
        };
        let control = Control {
            reserved_user_id: initiator_user_id.to_owned(),
            phase,
            direction: Some("businessInitiated".into()),
            deadline_at,
            fence_token: random_uuid(),
        };
        let ttl_ms = if phase == Phase::Accepted {
            ACTIVE_CALL_CONTROL_TTL_MS
        } else {
            remaining_ttl_ms(deadline_at) + VOIP_CONTROL_EXPIRY_MARGIN_MS
        };
        let value = serde_json::to_value(&control).unwrap_or_default();
        Ok(self
            .set_if_absent(&control_key(wacid), &value, ttl_ms)
            .await?
            .then_some(control))
    }

    /// `markOutboundRinging` (best-effort `dialing -> ringing`).
    pub async fn mark_outbound_ringing(&self, wacid: &str) -> Result<bool> {
        let key = control_key(wacid);
        let Some(current) = self.get::<Control>(&key).await? else {
            return Ok(false);
        };
        if !is_transition_allowed(current.phase, Phase::Ringing) {
            return Ok(false);
        }
        let next = Control {
            phase: Phase::Ringing,
            ..current.clone()
        };
        let ttl_ms = remaining_ttl_ms(current.deadline_at) + VOIP_CONTROL_EXPIRY_MARGIN_MS;
        self.swap_from(&key, &current, &next, ttl_ms).await
    }

    /// `markOutboundAccepted` (best-effort `dialing|ringing -> accepted`).
    pub async fn mark_outbound_accepted(&self, wacid: &str) -> Result<bool> {
        let key = control_key(wacid);
        let Some(current) = self.get::<Control>(&key).await? else {
            return Ok(false);
        };
        if !matches!(current.phase, Phase::Dialing | Phase::Ringing) {
            return Ok(false);
        }
        let next = Control {
            phase: Phase::Accepted,
            ..current.clone()
        };
        self.swap_from(&key, &current, &next, ACTIVE_CALL_CONTROL_TTL_MS)
            .await
    }

    /// `heartbeatActiveCall`: renews the control and the row's liveness.
    pub async fn heartbeat_active_call(
        &self,
        db: &Db,
        wacid: &str,
        workspace_id: i64,
        user_id: i64,
    ) -> Result<bool> {
        let key_wacid = wacid.to_owned();
        let call = tx::read(db, move |c| Ok(calls::find_by_wacid(c, &key_wacid)?)).await?;
        let Some(call) = call.filter(|c| c.workspace_id == workspace_id) else {
            return Ok(false);
        };
        let Some(control) = self.read_control(wacid).await? else {
            let owns_live_call = call.status == "accepted"
                && (call.answered_by_user_id == Some(user_id)
                    || call.initiated_by_user_id == Some(user_id));
            if !owns_live_call {
                return Ok(false);
            }
            touch_call_liveness(db, call.id).await;
            return Ok(true);
        };
        if control.phase != Phase::Accepted || control.reserved_user_id != user_id.to_string() {
            return Ok(false);
        }
        let expected = json!({ "phase": "accepted", "fenceToken": control.fence_token });
        if let Err(error) = self
            .swap(
                &control_key(wacid),
                &expected,
                &control,
                ACTIVE_CALL_CONTROL_TTL_MS,
            )
            .await
        {
            tracing::warn!(err = %error.message(), wacid, "WhatsApp VoIP active-call heartbeat: control renewal failed");
        }
        touch_call_liveness(db, call.id).await;
        Ok(true)
    }

    // ---- SDP records --------------------------------------------------------

    /// `storeOffer`: immutable first-seen offer; `true` = this call created it.
    pub async fn store_offer(&self, wacid: &str, sdp: &str, deadline_at: i64) -> Result<bool> {
        let record = json!({ "sdp": sdp, "deadlineAt": deadline_at });
        self.set_if_absent(&offer_key(wacid), &record, remaining_ttl_ms(deadline_at))
            .await
    }

    /// `readOffer`: the offer SDP.
    pub async fn read_offer(&self, wacid: &str) -> Result<Option<String>> {
        Ok(self
            .get::<Value>(&offer_key(wacid))
            .await?
            .and_then(|v| v["sdp"].as_str().map(str::to_owned)))
    }

    pub async fn delete_offer(&self, wacid: &str) -> Result<()> {
        self.del(&offer_key(wacid)).await
    }

    /// `storeOutboundAnswer`.
    pub async fn store_outbound_answer(&self, attempt_id: &str, sdp: &str) -> Result<bool> {
        self.set_if_absent(
            &outbound_answer_key(attempt_id),
            &json!({ "sdp": sdp }),
            VOIP_ANSWER_DEADLINE_MS,
        )
        .await
    }

    /// `readOutboundAnswer`: the answer SDP.
    pub async fn read_outbound_answer(&self, attempt_id: &str) -> Result<Option<String>> {
        Ok(self
            .get::<Value>(&outbound_answer_key(attempt_id))
            .await?
            .and_then(|v| v["sdp"].as_str().map(str::to_owned)))
    }

    pub async fn delete_outbound_answer(&self, attempt_id: &str) -> Result<()> {
        self.del(&outbound_answer_key(attempt_id)).await
    }
}

/// `touchCallLiveness` (never fails).
async fn touch_call_liveness(db: &Db, id: i64) {
    let older_than = now_ms() - ACTIVE_CALL_ROW_TOUCH_INTERVAL_MS;
    if let Err(error) = tx::write(db, move |c| {
        Ok(calls::touch_liveness_if_stale(c, id, older_than)?)
    })
    .await
    {
        tracing::warn!(err = %error.message(), whatsapp_call_id = id, "WhatsApp VoIP active-call heartbeat: durable liveness touch failed");
    }
}

/// `pinAnswerDtlsSetup`: `a=setup:actpass` lines become `a=setup:active`.
pub fn pin_answer_dtls_setup(sdp: &str) -> String {
    let mut out = String::with_capacity(sdp.len());
    for (index, line) in sdp.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let (body, cr) = match line.strip_suffix('\r') {
            Some(body) => (body, "\r"),
            None => (line, ""),
        };
        if body == "a=setup:actpass" {
            out.push_str("a=setup:active");
            out.push_str(cr);
        } else {
            out.push_str(line);
        }
    }
    out
}

// ---- signalling jobs ----------------------------------------------------------

pub const HANDLE_CONNECT: &str = "handleConnect";
pub const EXPIRE_IF_UNANSWERED: &str = "expireIfUnanswered";
pub const HANDLE_OUTBOUND_ANSWER: &str = "handleOutboundAnswer";
pub const EXPIRE_OUTBOUND_DIAL: &str = "expireOutboundDial";

fn jobs_error(error: chatbotx_jobs::JobsError) -> BusinessError {
    BusinessError::Repo(chatbotx_db::repo::Error::msg(error.to_string()))
}

async fn add_signal(
    jobs: &dyn Enqueuer,
    name: &str,
    data: Value,
    job_id: String,
    delay_ms: Option<i64>,
) -> Result<()> {
    let opts = JobOptions {
        job_id: Some(job_id),
        delay_ms: delay_ms.map(|d| u64::try_from(d.max(0)).unwrap_or(0)),
        ..JobOptions::default()
    };
    jobs.add(
        QueueName::WhatsappVoipSignaling,
        name,
        json!({ "type": name, "data": data }),
        opts,
    )
    .await
    .map(|_| ())
    .map_err(jobs_error)
}

async fn enqueue_handle_connect(
    jobs: &dyn Enqueuer,
    wacid: &str,
    deadline_at: i64,
    phone_number_id: &str,
    received_at: Option<i64>,
) -> Result<()> {
    let data = json!({
        "wacid": wacid,
        "deadlineAt": deadline_at,
        "phoneNumberId": phone_number_id,
        "receivedAt": received_at.unwrap_or_else(now_ms),
    });
    add_signal(
        jobs,
        HANDLE_CONNECT,
        data,
        format!("voip-signal-{wacid}"),
        None,
    )
    .await
}

/// `captureConnectOffer`: stores the offer, then enqueues `handleConnect` and
/// `expireIfUnanswered` (the SDP never enters a job).
pub async fn capture_connect_offer(
    store: &VoipStore,
    jobs: &dyn Enqueuer,
    wacid: &str,
    sdp: &str,
    phone_number_id: &str,
    received_at: Option<i64>,
) -> Result<()> {
    let deadline_at = now_ms() + VOIP_ANSWER_DEADLINE_MS;
    if !store.store_offer(wacid, sdp, deadline_at).await? {
        return Ok(());
    }
    let enqueued = async {
        enqueue_handle_connect(jobs, wacid, deadline_at, phone_number_id, received_at).await?;
        let data =
            json!({ "wacid": wacid, "deadlineAt": deadline_at, "phoneNumberId": phone_number_id });
        add_signal(
            jobs,
            EXPIRE_IF_UNANSWERED,
            data,
            format!("voip-expire-{wacid}"),
            Some(deadline_at - now_ms()),
        )
        .await
    }
    .await;
    if let Err(error) = enqueued {
        let _ = store.delete_offer(wacid).await;
        return Err(error);
    }
    Ok(())
}

/// `rejectUnprocessableConnect`: a connect without a usable offer.
pub async fn reject_unprocessable_connect(
    jobs: &dyn Enqueuer,
    wacid: &str,
    phone_number_id: &str,
    received_at: Option<i64>,
) -> Result<()> {
    enqueue_handle_connect(
        jobs,
        wacid,
        now_ms() + VOIP_ANSWER_DEADLINE_MS,
        phone_number_id,
        received_at,
    )
    .await
}

/// `captureOutboundAnswer`: stores the (DTLS-pinned) answer by attemptId and
/// enqueues `handleOutboundAnswer`. An unresolvable row is dropped.
pub async fn capture_outbound_answer(
    db: &Db,
    store: &VoipStore,
    jobs: &dyn Enqueuer,
    attempt_id: &str,
    wacid: &str,
    sdp: &str,
) -> Result<()> {
    let (attempt, key_wacid) = (attempt_id.to_owned(), wacid.to_owned());
    let row = tx::read(db, move |c| {
        let by_attempt = if attempt.is_empty() {
            None
        } else {
            calls::find_by_attempt_id(c, &attempt)?
        };
        Ok(match by_attempt {
            Some(row) => Some(row),
            None => calls::find_by_wacid(c, &key_wacid)?,
        })
    })
    .await?;
    let Some(row) = row else {
        tracing::warn!(
            attempt_id,
            wacid,
            "Whatsapp outbound answer: no matching call row found; dropping"
        );
        return Ok(());
    };
    let attempt_id = row
        .attempt_id
        .clone()
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| attempt_id.to_owned());
    if attempt_id.is_empty() {
        tracing::warn!(
            wacid,
            whatsapp_call_id = row.id,
            "Whatsapp outbound answer: resolved call row has no attemptId; cannot store/enqueue the answer"
        );
        return Ok(());
    }
    if !store
        .store_outbound_answer(&attempt_id, &pin_answer_dtls_setup(sdp))
        .await?
    {
        return Ok(());
    }
    let wacid = Some(wacid.to_owned())
        .filter(|w| !w.is_empty())
        .or(row.wacid.clone());
    let data = json!({
        "attemptId": attempt_id,
        "whatsappCallId": row.id.to_string(),
        "wacid": wacid,
        "workspaceId": row.workspace_id.to_string(),
    });
    if let Err(error) = add_signal(
        jobs,
        HANDLE_OUTBOUND_ANSWER,
        data,
        format!("voip-out-answer-{attempt_id}"),
        None,
    )
    .await
    {
        let _ = store.delete_outbound_answer(&attempt_id).await;
        return Err(error);
    }
    Ok(())
}

/// `enqueueOutboundDialExpiry`.
pub async fn enqueue_outbound_dial_expiry(
    jobs: &dyn Enqueuer,
    attempt_id: &str,
    whatsapp_call_id: i64,
    wacid: &str,
    workspace_id: i64,
    deadline_at: i64,
) -> Result<()> {
    let data = json!({
        "attemptId": attempt_id,
        "whatsappCallId": whatsapp_call_id.to_string(),
        "wacid": wacid,
        "workspaceId": workspace_id.to_string(),
        "deadlineAt": deadline_at,
    });
    add_signal(
        jobs,
        EXPIRE_OUTBOUND_DIAL,
        data,
        format!("voip-out-expire-{attempt_id}"),
        Some(deadline_at - now_ms()),
    )
    .await
}

// ---- ring targets and call access --------------------------------------------

/// `MAX_VOIP_RING_TARGETS`.
pub const MAX_VOIP_RING_TARGETS: usize = 10;

/// `isEligibleForConversationCall` (`CALL_ELIGIBILITY_RULES`).
pub fn is_eligible(user_id: i64, permissions: &Value, assigned_user_id: Option<i64>) -> bool {
    has_permission(permissions, "superAdmin")
        || has_permission(permissions, "contacts")
        || (has_permission(permissions, "onlyAssignedContacts")
            && assigned_user_id == Some(user_id))
}

/// A conversation's assignment (`RingConversation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Assignment {
    pub user_id: Option<i64>,
    pub team_id: Option<i64>,
}

/// `selectRingTargets` (pure): assignee, then the assigned team, then every
/// eligible online member; presence order; at most 10.
pub fn select_ring_targets(
    conversation: Option<Assignment>,
    online: &[i64],
    permissions: &std::collections::HashMap<i64, Value>,
    team_members: &[i64],
) -> Vec<i64> {
    let assigned = conversation.and_then(|c| c.user_id);
    let eligible_online = |candidates: &[i64]| -> Vec<i64> {
        online
            .iter()
            .copied()
            .filter(|user| candidates.contains(user))
            .filter(|user| {
                permissions
                    .get(user)
                    .is_some_and(|p| is_eligible(*user, p, assigned))
            })
            .collect()
    };
    let tiers: [Vec<i64>; 3] = [
        assigned.map_or_else(Vec::new, |a| eligible_online(&[a])),
        if conversation.and_then(|c| c.team_id).is_some() {
            eligible_online(team_members)
        } else {
            Vec::new()
        },
        eligible_online(online),
    ];
    tiers
        .into_iter()
        .find(|tier| !tier.is_empty())
        .map(|mut tier| {
            tier.truncate(MAX_VOIP_RING_TARGETS);
            tier
        })
        .unwrap_or_default()
}

fn load_assignment(
    conn: &chatbotx_db::rusqlite::Connection,
    workspace_id: i64,
    conversation_id: i64,
) -> Result<Option<Assignment>> {
    use chatbotx_db::rusqlite::OptionalExtension as _;
    Ok(conn
        .query_row(
            r#"SELECT "assignedUserId", "assignedInboxTeamId" FROM "Conversation" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            chatbotx_db::rusqlite::params![conversation_id, workspace_id],
            |row| {
                Ok(Assignment {
                    user_id: row.get(0)?,
                    team_id: row.get(1)?,
                })
            },
        )
        .optional()?)
}

fn load_permissions(
    conn: &chatbotx_db::rusqlite::Connection,
    workspace_id: i64,
    user_id: i64,
) -> Result<Option<Value>> {
    Ok(
        chatbotx_db::repo::workspace_member::list_permissions_by_user_ids(
            conn,
            workspace_id,
            &[user_id],
        )?
        .into_iter()
        .next()
        .map(|row| row.permissions.0),
    )
}

/// `selectRingTargetsForCall`: `online` is the workspace presence set.
pub async fn select_ring_targets_for_call(
    db: &Db,
    workspace_id: i64,
    conversation_id: i64,
    online: Vec<i64>,
) -> Result<Vec<i64>> {
    if online.is_empty() {
        return Ok(Vec::new());
    }
    tx::read(db, move |conn| {
        let assignment = load_assignment(conn, workspace_id, conversation_id)?;
        let permissions = chatbotx_db::repo::workspace_member::list_permissions_by_user_ids(
            conn,
            workspace_id,
            &online,
        )?
        .into_iter()
        .map(|row| (row.user_id, row.permissions.0))
        .collect();
        let team = match assignment.and_then(|a| a.team_id) {
            Some(team) => chatbotx_db::repo::inbox_team_member::list_user_ids_by_team_id(
                conn,
                workspace_id,
                team,
            )?,
            None => Vec::new(),
        };
        Ok(select_ring_targets(
            assignment,
            &online,
            &permissions,
            &team,
        ))
    })
    .await
}

/// `canCallConversation`: fresh member and conversation reads.
pub async fn can_call_conversation(
    db: &Db,
    workspace_id: i64,
    conversation_id: i64,
    user_id: i64,
) -> Result<bool> {
    tx::read(db, move |conn| {
        let Some(permissions) = load_permissions(conn, workspace_id, user_id)? else {
            return Ok(false);
        };
        let Some(assignment) = load_assignment(conn, workspace_id, conversation_id)? else {
            return Ok(false);
        };
        Ok(is_eligible(user_id, &permissions, assignment.user_id))
    })
    .await
}

/// `isCallHistoryAdmin`.
pub fn is_call_history_admin(permissions: &Value) -> bool {
    has_permission(permissions, "superAdmin") || has_permission(permissions, "analytics")
}

/// `CALL_READ_SCOPES`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadScope {
    History,
    Artifact,
}

/// `canReadCall` with an already-resolved member.
pub async fn can_read_call(
    db: &Db,
    workspace_id: i64,
    whatsapp_call_id: i64,
    user_id: i64,
    permissions: Value,
    scope: ReadScope,
) -> Result<bool> {
    if is_call_history_admin(&permissions) {
        return Ok(true);
    }
    tx::read(db, move |conn| {
        let Some(call) = calls::find_by_id_for_workspace(conn, whatsapp_call_id, workspace_id)?
        else {
            return Ok(false);
        };
        let Some(assignment) = load_assignment(conn, workspace_id, call.conversation_id)? else {
            return Ok(false);
        };
        let eligible = is_eligible(user_id, &permissions, assignment.user_id);
        Ok(match scope {
            ReadScope::Artifact => eligible,
            ReadScope::History => {
                eligible
                    && (call.answered_by_user_id == Some(user_id)
                        || call.initiated_by_user_id == Some(user_id))
            }
        })
    })
    .await
}

// ---- TURN ---------------------------------------------------------------------

/// `VOIP_TURN_CREDENTIAL_TTL_SECONDS`.
pub const VOIP_TURN_CREDENTIAL_TTL_SECONDS: i64 = 10 * 60;
const PUBLIC_STUN_URL: &str = "stun:stun.l.google.com:19302";

/// `mintTurnCredential`: `(username, base64(HMAC-SHA1(secret, username)))`.
pub fn mint_turn_credential(
    secret: &str,
    user_id: &str,
    ttl_seconds: i64,
    now: i64,
) -> (String, String) {
    let username = format!("{}:{user_id}", now / 1000 + ttl_seconds);
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret.as_bytes());
    let tag = ring::hmac::sign(&key, username.as_bytes());
    let credential = base64::engine::general_purpose::STANDARD.encode(tag.as_ref());
    (username, credential)
}

/// `voipTurnCredentialService.issueCredentials`.
pub fn issue_turn_credentials(
    user_id: i64,
    wacid: &str,
    turn_url: Option<&str>,
    turn_static_secret: Option<&str>,
) -> Value {
    let mut ice = vec![json!({ "urls": PUBLIC_STUN_URL })];
    let (Some(url), Some(secret)) = (
        turn_url.filter(|u| !u.is_empty()),
        turn_static_secret.filter(|s| !s.is_empty()),
    ) else {
        return json!({ "iceServers": ice, "turnConfigured": false });
    };
    let (username, credential) = mint_turn_credential(
        secret,
        &format!("{user_id}:{wacid}"),
        VOIP_TURN_CREDENTIAL_TTL_SECONDS,
        now_ms(),
    );
    ice.push(json!({ "urls": url, "username": username, "credential": credential }));
    json!({ "iceServers": ice, "turnConfigured": true })
}

/// `TURN_URL` / `TURN_STATIC_SECRET` from the environment.
pub fn turn_env() -> (Option<String>, Option<String>) {
    (
        std::env::var("TURN_URL").ok(),
        std::env::var("TURN_STATIC_SECRET").ok(),
    )
}

// ---- call permission --------------------------------------------------------------

/// `callPermissionStatuses`.
pub const PERMISSION_PERMANENT: &str = "permanent";
pub const PERMISSION_TEMPORARY: &str = "temporary";
pub const PERMISSION_NONE: &str = "no_permission";

/// `whatsappCallPermissionService.resolveStatus` (local record only).
pub async fn resolve_permission_status(
    db: &Db,
    contact_inbox_id: i64,
    now: i64,
) -> Result<Option<&'static str>> {
    let record = tx::read(db, move |c| {
        Ok(
            chatbotx_db::repo::whatsapp_call_permission::find_by_contact_inbox_id(
                c,
                contact_inbox_id,
            )?,
        )
    })
    .await?;
    Ok(record.map(|p| {
        if p.response == "reject" {
            PERMISSION_NONE
        } else if p.is_permanent {
            PERMISSION_PERMANENT
        } else if p.expires_at.is_some_and(|e| e > now) {
            PERMISSION_TEMPORARY
        } else {
            PERMISSION_NONE
        }
    }))
}

/// `mirrorProviderGrant`: stores a grant Meta reported; `true` when written.
pub async fn mirror_provider_grant(
    db: &Db,
    workspace_id: i64,
    contact_inbox_id: i64,
    status: &str,
    expiration_seconds: Option<f64>,
) -> Result<bool> {
    let permanent = status == PERMISSION_PERMANENT;
    if !(permanent || status == PERMISSION_TEMPORARY) {
        return Ok(false);
    }
    let expires_at = expiration_seconds
        .filter(|s| *s != 0.0)
        .map(|s| (s * 1000.0) as i64);
    if !permanent && expires_at.is_none() {
        return Ok(false);
    }
    let input = chatbotx_db::repo::whatsapp_call_permission::UpsertPermissionInput {
        workspace_id,
        contact_inbox_id,
        response: "accept".into(),
        is_permanent: permanent,
        expires_at: if permanent { None } else { expires_at },
        responded_at: now_ms(),
    };
    tx::write(db, move |c| {
        chatbotx_db::repo::whatsapp_call_permission::upsert_for_contact_inbox(c, &input)?;
        Ok(())
    })
    .await?;
    Ok(true)
}

/// `toCallPermissionStatus` of a `call_permissions` response.
pub fn meta_permission_status(response: &Value) -> Option<&'static str> {
    match response
        .pointer("/permission/status")
        .and_then(Value::as_str)
    {
        Some("no_permission") => Some(PERMISSION_NONE),
        Some("temporary") => Some(PERMISSION_TEMPORARY),
        Some("permanent") => Some(PERMISSION_PERMANENT),
        _ => None,
    }
}

/// `toPermissionExpirationTimestamp`.
pub fn meta_permission_expiration(response: &Value) -> Option<f64> {
    match response.pointer("/permission/expiration_time")? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|v| v.is_finite()),
        _ => None,
    }
}

/// `canPerformCallAction(response, action)`.
pub fn can_perform_call_action(response: &Value, action: &str) -> bool {
    response["actions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["action_name"] == action)
        .is_some_and(|a| {
            let flag = if a["can_perform_action"].is_null() {
                &a["can_perform"]
            } else {
                &a["can_perform_action"]
            };
            flag == &Value::Bool(true)
        })
}

// ---- dial identity / blocked countries -----------------------------------------------

/// `BLOCKED_OUTBOUND_COUNTRIES`.
pub const BLOCKED_OUTBOUND_COUNTRIES: &[&str] = &["VN", "US", "CA", "EG", "NG"];

/// `isBlockedBusinessCallingCountry`.
pub fn is_blocked_business_calling_country(display_phone_number: &str) -> bool {
    let text = display_phone_number.trim();
    let text = if text.starts_with('+') {
        text.to_owned()
    } else {
        format!("+{}", text.trim_start_matches('+'))
    };
    phonenumber::parse(None, &text)
        .ok()
        .and_then(|n| n.country().id())
        .is_some_and(|id| BLOCKED_OUTBOUND_COUNTRIES.contains(&id.as_ref()))
}

/// `resolveDialIdentity`: `(field, value)` of the dial (`to` / `recipient`) and
/// of the permission read (`user_wa_id` / `recipient`).
pub fn dial_identity(contact_inbox: &ContactInbox) -> (&'static str, String, &'static str) {
    if should_address_by_source_user_id(
        &contact_inbox.source_id,
        contact_inbox.source_user_id.as_deref(),
    ) {
        let recipient = contact_inbox.source_user_id.clone().unwrap_or_default();
        ("recipient", recipient, "recipient")
    } else {
        let to: String = contact_inbox
            .source_id
            .chars()
            .filter(char::is_ascii_digit)
            .collect();
        ("to", to, "user_wa_id")
    }
}

// ---- Graph calling API ----------------------------------------------------------------

/// `WhatsappException` of a calling Graph call.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphError {
    /// The HTTP status (`0` = no response).
    pub status: u16,
    /// Meta's `error.code` (a number), or a local code string.
    pub code: Value,
    pub message: String,
    /// `error_user_title` / `error_user_msg`.
    pub user_title: Option<String>,
    pub user_message: Option<String>,
}

impl GraphError {
    pub fn code_number(&self) -> Option<i64> {
        self.code.as_i64()
    }

    fn local(message: impl Into<String>, status: u16, code: &str) -> GraphError {
        GraphError {
            status,
            code: json!(code),
            message: message.into(),
            user_title: None,
            user_message: None,
        }
    }

    /// `throwWhatsappApiActionError`'s sentence.
    pub fn public_message(&self, fallback: &str) -> String {
        let pick = [&self.user_message, &self.user_title]
            .into_iter()
            .flatten()
            .find(|t| !t.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| self.message.clone());
        let pick = crate::errors::sanitize_public_text(pick.trim());
        if pick.is_empty() || crate::errors::is_query_dump(&pick) {
            fallback.to_owned()
        } else {
            pick
        }
    }
}

/// The Graph base + auth of a number.
pub struct GraphAuth<'a> {
    pub base_url: &'a str,
    pub auth: &'a Value,
}

impl GraphAuth<'_> {
    fn parts(&self) -> std::result::Result<(String, &str), GraphError> {
        let token = self
            .auth
            .pointer("/tokens/accessToken")
            .and_then(Value::as_str);
        let phone = self
            .auth
            .pointer("/metadata/phoneNumber/id")
            .and_then(Value::as_str);
        let (Some(token), Some(phone)) = (token, phone) else {
            return Err(GraphError::local(
                "Invalid WhatsApp credentials",
                401,
                "invalidAuth",
            ));
        };
        let version = self
            .auth
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_API_VERSION);
        Ok((format!("{}/{version}/{phone}", self.base_url), token))
    }
}

pub(crate) fn graph_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default()
    })
}

pub(crate) async fn graph_send(
    request: reqwest::RequestBuilder,
) -> std::result::Result<Value, GraphError> {
    let response = request.send().await.map_err(|e| GraphError {
        status: 0,
        code: Value::Null,
        message: e.to_string(),
        user_title: None,
        user_message: None,
    })?;
    let status = response.status().as_u16();
    let body: Value = response
        .text()
        .await
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    if (200..300).contains(&status) {
        return Ok(body);
    }
    let error = &body["error"];
    let text = |k: &str| error[k].as_str().map(str::to_owned);
    Err(GraphError {
        status,
        code: error.get("code").cloned().unwrap_or(Value::Null),
        message: text("message").unwrap_or_else(|| "WhatsApp call action failed".into()),
        user_title: text("error_user_title"),
        user_message: text("error_user_msg"),
    })
}

/// `MAX_CALL_ANNOUNCEMENT_PURPOSE_CHARS`.
pub const MAX_CALL_ANNOUNCEMENT_PURPOSE_CHARS: usize = 250;
pub const DEFAULT_CALL_ANNOUNCEMENT_PURPOSE: &str = "Quality assurance and support";
const SUPPORTED_CALL_ANNOUNCEMENT_LANGUAGES: &[&str] = &[
    "en", "en_US", "en_AU", "en_CA", "en_GB", "en_IN", "en_NZ", "nl", "fr", "de", "hi", "it", "kn",
    "pt", "es", "es_ES", "te", "vi",
];

/// `resolveAnnouncementLanguage`.
pub fn resolve_announcement_language(input: Option<&str>) -> &'static str {
    let Some(input) = input.filter(|i| !i.is_empty()) else {
        return "en_US";
    };
    let normalized = input.replacen('-', "_", 1);
    let find = |code: &str| {
        SUPPORTED_CALL_ANNOUNCEMENT_LANGUAGES
            .iter()
            .copied()
            .find(|c| *c == code)
    };
    find(&normalized)
        .or_else(|| normalized.split('_').next().and_then(find))
        .unwrap_or("en_US")
}

/// `WhatsappCallAnnouncementOptions`: the `{ status, purpose, announcementLanguage }`
/// of recording / transcription.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Announcements {
    pub recording: Option<(String, String)>,
    pub transcription: Option<(String, String)>,
}

impl Announcements {
    pub fn is_empty(&self) -> bool {
        self.recording.is_none() && self.transcription.is_none()
    }
}

/// `buildCallAnnouncementOptions`.
pub fn build_announcements(
    integration: &IntegrationWhatsapp,
    contact_locale: Option<&str>,
) -> Announcements {
    let recording =
        integration.call_recording_enabled && integration.call_recording_mode == "metaNative";
    let transcription = chatbotx_core::utils::whatsapp_call::transcribes_calls(
        chatbotx_core::utils::whatsapp_call::WhatsappCallMediaToggles {
            call_recording_enabled: integration.call_recording_enabled,
            call_transcription_enabled: integration.call_transcription_enabled,
        },
    ) && integration.call_transcription_mode == "metaNative";
    if !(recording || transcription) {
        return Announcements::default();
    }
    let purpose = integration
        .call_recording_purpose
        .clone()
        .unwrap_or_else(|| DEFAULT_CALL_ANNOUNCEMENT_PURPOSE.into());
    let language = resolve_announcement_language(
        integration
            .call_announcement_language
            .as_deref()
            .or(contact_locale),
    )
    .to_owned();
    let entry = (purpose, language);
    Announcements {
        recording: recording.then(|| entry.clone()),
        transcription: transcription.then_some(entry),
    }
}

fn announcement_body(entry: &(String, String)) -> std::result::Result<Value, GraphError> {
    let chars = entry.0.chars().count();
    if chars > MAX_CALL_ANNOUNCEMENT_PURPOSE_CHARS {
        return Err(GraphError::local(
            format!(
                "WhatsApp call announcement \"purpose\" exceeds {MAX_CALL_ANNOUNCEMENT_PURPOSE_CHARS} characters (got {chars})"
            ),
            400,
            "whatsappCallAnnouncementPurposeTooLong",
        ));
    }
    Ok(json!({
        "status": "ENABLED",
        "purpose": entry.0,
        "announcement_language": resolve_announcement_language(Some(&entry.1)),
    }))
}

const DOCUMENTED_CALLING_ERROR_CODES: &[i64] = &[
    613, 131_009, 131_030, 131_044, 131_055, 138_000, 138_001, 138_002, 138_003, 138_004, 138_005,
    138_006, 138_007, 138_009, 138_012, 138_013, 138_014, 138_015, 138_017, 138_018, 138_019,
    138_020, 138_021, 138_022, 138_023,
];

/// `isCallAnnouncementValidationError`.
pub fn is_announcement_validation_error(error: &GraphError) -> bool {
    if error.code == json!("whatsappCallAnnouncementPurposeTooLong") {
        return false;
    }
    let code = error
        .code
        .as_i64()
        .or_else(|| error.code.as_str().and_then(|c| c.parse().ok()));
    if matches!(error.status, 401 | 403 | 429)
        || code.is_some_and(|c| DOCUMENTED_CALLING_ERROR_CODES.contains(&c))
    {
        return false;
    }
    (400..500).contains(&error.status)
}

/// `postCallAction`: `POST /{phone}/calls`.
async fn post_call_action(
    graph: &GraphAuth<'_>,
    body: Value,
) -> std::result::Result<Value, GraphError> {
    let (base, token) = graph.parts()?;
    let action = body["action"].as_str().unwrap_or_default().to_owned();
    let result = graph_send(
        graph_client()
            .post(format!("{base}/calls"))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string()),
    )
    .await;
    if let Err(error) = &result {
        tracing::error!(action, http_status = error.status, call_id = ?body["call_id"].as_str(), "WhatsApp call action failed");
    }
    result
}

fn with_announcements(
    mut body: Value,
    options: &Announcements,
) -> std::result::Result<Value, GraphError> {
    if let Some(entry) = &options.recording {
        body["recording"] = announcement_body(entry)?;
    }
    if let Some(entry) = &options.transcription {
        body["transcription"] = announcement_body(entry)?;
    }
    Ok(body)
}

/// `preAcceptCall`.
pub async fn pre_accept_call(
    graph: &GraphAuth<'_>,
    wacid: &str,
    sdp: &str,
) -> std::result::Result<(), GraphError> {
    let body = json!({ "messaging_product": "whatsapp", "call_id": wacid, "action": "pre_accept", "session": { "sdp_type": "answer", "sdp": sdp } });
    post_call_action(graph, body).await.map(|_| ())
}

/// `acceptCall`.
pub async fn accept_call(
    graph: &GraphAuth<'_>,
    wacid: &str,
    sdp: &str,
    options: &Announcements,
) -> std::result::Result<(), GraphError> {
    let body = json!({ "messaging_product": "whatsapp", "call_id": wacid, "action": "accept", "session": { "sdp_type": "answer", "sdp": sdp } });
    post_call_action(graph, with_announcements(body, options)?)
        .await
        .map(|_| ())
}

/// `rejectCall` / `terminateCall`.
pub async fn end_call_at_meta(
    graph: &GraphAuth<'_>,
    wacid: &str,
    action: EndAction,
) -> std::result::Result<(), GraphError> {
    let body =
        json!({ "messaging_product": "whatsapp", "call_id": wacid, "action": action.as_str() });
    post_call_action(graph, body).await.map(|_| ())
}

/// `connectCall`: Meta's call id (wacid).
pub async fn connect_call(
    graph: &GraphAuth<'_>,
    sdp_offer: &str,
    attempt_id: &str,
    identity: (&str, &str),
    options: &Announcements,
) -> std::result::Result<String, GraphError> {
    let mut body = json!({
        "messaging_product": "whatsapp",
        "action": "connect",
        "biz_opaque_callback_data": attempt_id,
        "session": { "sdp_type": "offer", "sdp": sdp_offer },
    });
    body[identity.0] = json!(identity.1);
    let response = post_call_action(graph, with_announcements(body, options)?).await?;
    response
        .pointer("/calls/0/id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            tracing::error!(
                action = "connect",
                "WhatsApp connect call action returned no call id"
            );
            GraphError::local(
                "WhatsApp connect call action returned no call id",
                502,
                "callFailed",
            )
        })
}

/// `getCallPermissions`: `GET /{phone}/call_permissions?<field>=<value>`.
pub async fn get_call_permissions(
    graph: &GraphAuth<'_>,
    field: &str,
    value: &str,
) -> std::result::Result<Value, GraphError> {
    let (base, token) = graph.parts()?;
    graph_send(
        graph_client()
            .get(format!(
                "{base}/call_permissions?{}",
                url::form_urlencoded::Serializer::new(String::new())
                    .append_pair(field, value)
                    .finish()
            ))
            .bearer_auth(token),
    )
    .await
}

/// `getCallingSettings`: `calling` of `GET /{phone}/settings`.
pub async fn get_calling_settings(graph: &GraphAuth<'_>) -> std::result::Result<Value, GraphError> {
    let (base, token) = graph.parts()?;
    let body = graph_send(
        graph_client()
            .get(format!("{base}/settings"))
            .bearer_auth(token),
    )
    .await?;
    Ok(body
        .get("calling")
        .filter(|c| c.is_object())
        .or_else(|| body.pointer("/data/0/calling").filter(|c| c.is_object()))
        .cloned()
        .unwrap_or_else(|| json!({ "status": "DISABLED" })))
}

/// `findPhoneNumberDetail` (the fields the calling preflight reads).
pub async fn phone_number_detail(graph: &GraphAuth<'_>) -> std::result::Result<Value, GraphError> {
    let (base, token) = graph.parts()?;
    graph_send(
        graph_client()
            .get(format!(
                "{base}?fields=platform_type%2Cwhatsapp_business_manager_messaging_limit"
            ))
            .bearer_auth(token),
    )
    .await
}

/// `updateCallingSettings`: `POST /{phone}/settings { calling }`.
pub async fn update_calling_settings(
    graph: &GraphAuth<'_>,
    calling: Value,
) -> std::result::Result<(), GraphError> {
    let (base, token) = graph.parts()?;
    graph_send(
        graph_client()
            .post(format!("{base}/settings"))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(json!({ "calling": calling }).to_string()),
    )
    .await
    .map(|_| ())
}

/// `mapMetaErrorCodeToOutcome`.
pub fn outbound_outcome_for_code(code: Option<i64>) -> &'static str {
    match code {
        Some(138_006) => "needsPermission",
        Some(138_000) => "callingNotEnabled",
        Some(138_003) => "callAlreadyInProgress",
        Some(138_012) => "dailyLimitReached",
        Some(138_013) => "ineligibleNumber",
        Some(138_001) => "recipientUncallable",
        Some(138_014) => "temporarilyDisabled",
        Some(138_005 | 138_002) => "rateLimited",
        Some(131_044) => "paymentIssue",
        _ => "callFailed",
    }
}

// ---- call rows ------------------------------------------------------------------------

/// `finalizeEndedCall` without a card (the action paths).
pub async fn finalize_row(
    db: &Db,
    id: i64,
    terminal: WhatsappCallTerminalStatus,
    canceled_by_business: bool,
    last_error: Option<String>,
) -> Result<Option<WhatsappCall>> {
    let outcome = resolve_whatsapp_call_outcome(terminal, canceled_by_business);
    let input = FinalizeInput {
        id,
        status: terminal_status_str(terminal).into(),
        outcome: outcome.as_str().into(),
        ended_at: Some(Some(now_ms())),
        last_error: last_error.map(Some),
        ..FinalizeInput::default()
    };
    tx::write(db, move |c| Ok(calls::finalize_by_id(c, &input)?)).await
}

/// `resolveWhatsappCallerName` (a miss is `None`).
pub async fn caller_name(db: &Db, call: &WhatsappCall) -> Option<String> {
    let (ws, ci) = (call.workspace_id, call.contact_inbox_id);
    tx::read(db, move |c| {
        use chatbotx_db::rusqlite::OptionalExtension as _;
        Ok(c.query_row(
            r#"SELECT ct."fullName" FROM "ContactInbox" ci JOIN "Contact" ct ON ct."id" = ci."contactId"
               WHERE ci."id" = ?1 AND ct."workspaceId" = ?2"#,
            chatbotx_db::rusqlite::params![ci, ws],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
    })
    .await
    .unwrap_or_else(|error| {
        tracing::warn!(err = %error.message(), "resolveWhatsappCallerName failed; falling back to null caller name");
        None
    })
}

/// `listResumableIncoming`: still-ringing unclaimed calls the agent may answer.
pub async fn list_resumable_incoming(
    db: &Db,
    store: &VoipStore,
    workspace_id: i64,
    user_id: i64,
) -> Result<Value> {
    let (candidates, permissions) = tx::read(db, move |c| {
        Ok((
            calls::find_ringing_by_workspace(c, workspace_id)?,
            load_permissions(c, workspace_id, user_id)?,
        ))
    })
    .await?;
    let complete = (candidates.len() as i64) < calls::FIND_RINGING_BY_WORKSPACE_LIMIT;
    let mut out = Vec::new();
    for call in candidates {
        let Some(wacid) = call.wacid.clone() else {
            continue;
        };
        let Some(control) = store.read_control(&wacid).await? else {
            continue;
        };
        if control.phase != Phase::Reserved || !control.reserved_user_id.is_empty() {
            continue;
        }
        let Some(permissions) = permissions.clone() else {
            continue;
        };
        let conversation_id = call.conversation_id;
        let assignment = tx::read(db, move |c| {
            load_assignment(c, workspace_id, conversation_id)
        })
        .await?;
        let Some(assignment) = assignment else {
            continue;
        };
        if !is_eligible(user_id, &permissions, assignment.user_id) {
            continue;
        }
        let Some(sdp) = store.read_offer(&wacid).await? else {
            continue;
        };
        out.push(json!({
            "whatsappCallId": call.id.to_string(),
            "wacid": wacid,
            "conversationId": call.conversation_id.to_string(),
            "contactInboxId": call.contact_inbox_id.to_string(),
            "contactName": caller_name(db, &call).await,
            "offer": { "sdpType": "offer", "sdp": sdp },
            "deadlineAt": chatbotx_db::repo::iso(control.deadline_at),
        }));
    }
    Ok(json!({ "calls": out, "isComplete": complete }))
}

/// `readOutboundAnswerForInitiator`.
pub async fn read_outbound_answer_for_initiator(
    db: &Db,
    store: &VoipStore,
    workspace_id: i64,
    whatsapp_call_id: i64,
    user_id: i64,
) -> Result<Option<String>> {
    let call = tx::read(db, move |c| {
        Ok(calls::find_by_id_for_workspace(
            c,
            whatsapp_call_id,
            workspace_id,
        )?)
    })
    .await?;
    let Some(call) = call.filter(|c| {
        c.direction == "businessInitiated"
            && c.answered_by_user_id == Some(user_id)
            && !is_call_ended(c)
    }) else {
        return Ok(None);
    };
    match call.attempt_id.as_deref() {
        Some(attempt) if !attempt.is_empty() => store.read_outbound_answer(attempt).await,
        _ => Ok(None),
    }
}

/// The one-live-call guard (`assertNoActiveCallForContact`): `false` = a live
/// call is in progress.
pub async fn no_active_call_for_contact(
    db: &Db,
    store: &VoipStore,
    inbox_id: i64,
    contact_inbox_id: i64,
) -> Result<bool> {
    let active = tx::read(db, move |c| {
        Ok(calls::find_active_by_contact_inbox(
            c,
            inbox_id,
            contact_inbox_id,
        )?)
    })
    .await?;
    let Some(active) = active else {
        return Ok(true);
    };
    recover_stranded_accepted_call(db, store, &active).await
}

/// `recoverStrandedAcceptedCall`.
async fn recover_stranded_accepted_call(
    db: &Db,
    store: &VoipStore,
    call: &WhatsappCall,
) -> Result<bool> {
    if call.status != "accepted" {
        return Ok(false);
    }
    if let Some(wacid) = &call.wacid
        && store
            .read_control(wacid)
            .await?
            .is_some_and(|c| c.phase != Phase::Terminated)
    {
        return Ok(false);
    }
    let (id, older_than) = (call.id, now_ms() - ACTIVE_CALL_LIVENESS_STALE_MS);
    let recovered = tx::write(db, move |c| {
        Ok(calls::recover_stranded_accepted(
            c,
            id,
            older_than,
            STRANDED_CALL_RECOVERED_LAST_ERROR,
        )?)
    })
    .await?;
    if recovered.is_some() {
        tracing::info!(whatsapp_call_id = id, wacid = ?call.wacid, "WhatsApp call: closed a stranded accepted call so this contact can be dialed again");
        return Ok(true);
    }
    let latest = tx::read(db, move |c| Ok(calls::find_by_id(c, id)?)).await?;
    Ok(!latest.is_some_and(|l| matches!(l.status.as_str(), "accepted" | "ringing")))
}

/// `recordCallRecordingArrangement` (never fails).
pub async fn record_recording_arrangement(
    db: &Db,
    whatsapp_call_id: i64,
    recording_requested: bool,
    recording_was_requested: bool,
) {
    let rejected = recording_was_requested && !recording_requested;
    tracing::info!(
        whatsapp_call_id,
        recording_was_requested,
        recording_requested,
        accepted_by_meta = !rejected,
        "[wa-call-media] call media arrangement"
    );
    let reason = rejected.then_some(META_RECORDING_ANNOUNCEMENT_REJECTED);
    if let Err(error) = tx::write(db, move |c| {
        Ok(calls::mark_recording_arrangement(
            c,
            whatsapp_call_id,
            recording_requested,
            reason,
        )?)
    })
    .await
    {
        tracing::error!(err = %error.message(), whatsapp_call_id, "WhatsApp call: failed to store the call's recording arrangement");
    }
    if rejected {
        tracing::error!(
            whatsapp_call_id,
            "WhatsApp call: Meta refused the recording announcement — this call is NOT recorded"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transitions_follow_the_table() {
        assert!(is_transition_allowed(Phase::Reserved, Phase::Answering));
        assert!(!is_transition_allowed(Phase::Accepted, Phase::Answering));
        assert!(is_transition_allowed(Phase::Dialing, Phase::Accepted));
        assert!(!is_transition_allowed(Phase::Terminated, Phase::Terminated));
    }

    #[test]
    fn dtls_setup_pinned_on_both_line_endings() {
        assert_eq!(
            pin_answer_dtls_setup("v=0\r\na=setup:actpass\r\nm=audio"),
            "v=0\r\na=setup:active\r\nm=audio"
        );
        assert_eq!(
            pin_answer_dtls_setup("a=setup:actpass\na=setup:passive"),
            "a=setup:active\na=setup:passive"
        );
        assert_eq!(
            pin_answer_dtls_setup("a=setup:actpassive"),
            "a=setup:actpassive"
        );
    }
}
