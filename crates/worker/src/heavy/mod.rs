//! Q.W2 run 4: the heavy AI flow steps (`aiGenerateImage`, `aiEditImage`,
//! `aiSegmentImage`, `aiSpeechToText`, `aiTextToSpeech`).
//!
//! - The flow step (`runViaHeavyWorker`) records a pending outcome, queues
//!   the heavy job and a fallback `resumeHeavyStep` at the completion
//!   deadline, and waits.
//! - The heavy job (`runHeavyStep` + the worker's completion code) calls the
//!   provider, stores the file under the workspace prefix, writes the output
//!   fields once (`writing` claim, replayed on a retry) and queues the
//!   resume.
//! - `resumeHeavyStep` claims the resume lease and re-enters the flow at the
//!   step, which then routes on the recorded outcome.

pub mod outcome;
pub mod provider;

use std::sync::{Arc, LazyLock};

use chatbotx_core::flow_config::{
    AI_EDIT_IMAGE_FALLBACK_OPENAI_MODEL, ai_edit_image_schema, ai_generate_image_schema,
    ai_segment_image_schema, ai_speech_to_text_schema, ai_text_to_speech_schema,
    get_ai_generated_image_path,
};
use chatbotx_core::utils::channel_safe_image::{
    self as images, NormalizeError, sniff_format, to_channel_safe_image,
};
use chatbotx_db::repo::models::{ContactInbox, Conversation};
use chatbotx_jobs::queues::heavy::{
    HeavyJobAction, HeavyJobData, HeavyOutputValue, HeavyStepResult,
    heavy_completion_wait_timeout_ms, heavy_job_options,
};
use chatbotx_jobs::queues::integration::flow_continuation_job_options;
use chatbotx_jobs::{JobCtx, JobError, JobOptions, QueueName};
use chatbotx_kv::heavy_provider::{HeavyProviderSlot, wait_for_heavy_provider_slot};
use chatbotx_kv::{Clock as _, DurableKv, MemKv, SystemClock};
use serde_json::{Map, Value, json};

use self::outcome::{Attempt, Identity, Transition};
use self::provider::{Generated, HeavyError, ImageAnswer, ImageRequest};
use crate::ai::llm::Model;
use crate::ai::model;
use crate::channels::whatsapp_media::{DownloadError, download_checked, media_client};
use crate::flow::StepStatus;
use crate::steps::{StepCtx, step_error};
use crate::{Deps, db_fail, id_of, load_contact_inbox, load_conversation, str_of};

/// `IntegrationJobAction.resumeHeavyStep`.
pub const RESUME_HEAVY_STEP: &str = "resumeHeavyStep";
/// `SEGMENT_MAX_INPUT_PIXELS`: the decoded bitmap is bounded (pixel bombs).
pub const SEGMENT_MAX_INPUT_PIXELS: u64 = 25_000_000;
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];

fn env_num(name: &str, default: u64, min: u64, max: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| (min..=max).contains(v))
        .unwrap_or(default)
}

/// `HEAVY_JOB_WAIT_TIMEOUT_MS`: one paid attempt.
pub fn wait_timeout_ms() -> u64 {
    env_num("HEAVY_JOB_WAIT_TIMEOUT_MS", 120_000, 5000, 9 * 60_000)
}

/// `MUSE_TRANSCRIBE_TIMEOUT_MS`: one Muse Speech to Text attempt.
pub fn muse_transcribe_timeout_ms() -> u64 {
    env_num("MUSE_TRANSCRIBE_TIMEOUT_MS", 9 * 60_000, 5000, 9 * 60_000)
}

/// `HEAVY_MAX_AUDIO_BYTES`.
fn max_audio_bytes() -> u64 {
    env_num("HEAVY_MAX_AUDIO_BYTES", 25 << 20, 1, 100 << 20)
}

/// `MUSE_TRANSCRIBE_MAX_AUDIO_BYTES` (a 10-minute 24 kHz WAV is over
/// `HEAVY_MAX_AUDIO_BYTES`).
fn muse_max_audio_bytes() -> u64 {
    let max = provider::MUSE_TRANSCRIBE_MAX_REQUEST_BYTES
        - provider::MUSE_TRANSCRIBE_MULTIPART_OVERHEAD_BYTES;
    env_num("MUSE_TRANSCRIBE_MAX_AUDIO_BYTES", max, 1, max)
}

/// `getHeavyStepAttemptBudgetMs`: a Muse transcription may run longer than
/// one heavy wait.
fn attempt_budget_ms(action: HeavyJobAction, provider: &str) -> u64 {
    if action == HeavyJobAction::AiSpeechToText && provider == "muse" {
        return wait_timeout_ms().max(muse_transcribe_timeout_ms());
    }
    wait_timeout_ms()
}

/// `HEAVY_MAX_IMAGE_BYTES`.
pub fn max_image_bytes() -> u64 {
    env_num("HEAVY_MAX_IMAGE_BYTES", 10 << 20, 1, 25 << 20)
}

/// The heavy worker lease: the longest attempt plus a handoff margin, so a
/// slow provider is not redelivered while it runs.
pub fn worker_lease_ms() -> u64 {
    // `getMaxHeavyStepAttemptBudgetMs`.
    (wait_timeout_ms().max(muse_transcribe_timeout_ms()) + 60_000).max(5 * 60_000)
}

fn kv(deps: &Deps) -> DurableKv {
    DurableKv::new(deps.db.clone(), Arc::new(SystemClock))
}

/// The flow step types that run on the heavy queue.
pub fn action_of(step_type: &str) -> Option<HeavyJobAction> {
    Some(match step_type {
        "aiEditImage" => HeavyJobAction::AiEditImage,
        "aiGenerateImage" => HeavyJobAction::AiGenerateImage,
        "aiSegmentImage" => HeavyJobAction::AiSegmentImage,
        "aiSpeechToText" => HeavyJobAction::AiSpeechToText,
        "aiTextToSpeech" => HeavyJobAction::AiTextToSpeech,
        _ => return None,
    })
}

fn step_schema(action: HeavyJobAction) -> chatbotx_core::flow_config::z::Schema {
    match action {
        HeavyJobAction::AiEditImage => ai_edit_image_schema(),
        HeavyJobAction::AiSegmentImage => ai_segment_image_schema(),
        HeavyJobAction::AiSpeechToText => ai_speech_to_text_schema(),
        HeavyJobAction::AiTextToSpeech => ai_text_to_speech_schema(),
        _ => ai_generate_image_schema(),
    }
}

/// `runViaHeavyWorker`.
pub(crate) async fn run_step(
    ctx: &StepCtx<'_, '_>,
    action: HeavyJobAction,
) -> Result<StepStatus, JobError> {
    let run = ctx.run;
    let step_id = ctx.str("id").unwrap_or("").to_owned();
    let contact_inbox_id = run.contact_inbox.id.to_string();
    let conversation_id = run.conversation.id.to_string();
    let identity = Identity {
        action: action.as_str(),
        contact_inbox_id: &contact_inbox_id,
        conversation_id: &conversation_id,
        parent_job_id: &run.flow_execution_key,
        step_id: &step_id,
    };
    let key = identity.outcome_key();
    let wait = attempt_budget_ms(action, ctx.str("provider").unwrap_or(""));
    let completion = heavy_completion_wait_timeout_ms(action, wait);
    let kv = kv(run.deps);
    let recorded = outcome::ensure_pending(
        &kv,
        &key,
        completion,
        outcome::outcome_ttl_ms(wait, completion),
    )
    .await
    .map_err(db_fail)?;
    match recorded.status.as_str() {
        "succeeded" if recorded.result_status.as_deref() == Some("skip") => {
            return Ok(StepStatus::Skip);
        }
        "succeeded" => return Ok(StepStatus::Success),
        "failed" => {
            let message = recorded
                .error_message
                .as_deref()
                .unwrap_or("Heavy step failed");
            return Ok(step_error(ctx, message));
        }
        "writing" => return Ok(StepStatus::Wait),
        _ => {}
    }

    // The zod `parse` of the step (trimmed values, unknown keys dropped).
    let step = match step_schema(action).parse(ctx.step) {
        Ok(step) => step,
        Err(err) => {
            // ponytail: the original throws (the sendFlow job retries the
            // same invalid step); a permanent schema error routes to error.
            let message = format!("Invalid {} step: {err}", action.as_str());
            fail_outcome(&kv, &key, &message).await?;
            return Ok(step_error(ctx, &message));
        }
    };
    let mut continuation = run.base_job(&run.target_node_id, ctx.node_visits, ctx.anchor.as_ref());
    continuation.insert("flowExecutionKey".into(), json!(run.flow_execution_key));
    if let Some(version) = run.version_id() {
        continuation.insert("flowVersionId".into(), json!(version));
    }
    if matches!(run.target_type, "button" | "quickReply") {
        continuation.insert("targetType".into(), json!(run.target_type));
        continuation.insert("targetId".into(), json!(run.target_id));
    }
    let mut data = Map::new();
    data.insert("conversationId".into(), json!(conversation_id));
    data.insert("contactInboxId".into(), json!(contact_inbox_id));
    data.insert("outcomeKey".into(), json!(key));
    data.insert("continuation".into(), Value::Object(continuation.clone()));
    if let Some(metadata) = run.metadata() {
        data.insert("metadata".into(), metadata.clone());
    }
    data.insert("step".into(), step);
    let job_data = json!({ "type": action.as_str(), "data": data });
    if let Err(err) = HeavyJobData::parse(&job_data) {
        let message = format!("Invalid heavy job data: {err}");
        fail_outcome(&kv, &key, &message).await?;
        return Ok(step_error(ctx, &message));
    }
    run.deps
        .jobs
        .add(
            QueueName::Heavy,
            action.as_str(),
            job_data,
            JobOptions {
                job_id: Some(identity.job_id()),
                ..heavy_job_options(action)
            },
        )
        .await
        .map_err(db_fail)?;
    let mut resume = continuation;
    resume.insert("outcomeKey".into(), json!(key));
    resume.insert("startFromStepId".into(), json!(step_id));
    run.deps
        .jobs
        .add(
            QueueName::Integration,
            RESUME_HEAVY_STEP,
            json!({ "type": RESUME_HEAVY_STEP, "data": resume }),
            JobOptions {
                job_id: Some(format!("heavy-resume-fallback-{}", identity.hash())),
                delay_ms: Some(completion),
                ..flow_continuation_job_options(run.metadata())
            },
        )
        .await
        .map_err(db_fail)?;
    Ok(StepStatus::Wait)
}

async fn fail_outcome(kv: &DurableKv, key: &str, message: &str) -> Result<(), JobError> {
    outcome::transition(kv, key, "failed", Some(message.to_owned()), None, None)
        .await
        .map(|_| ())
        .map_err(db_fail)
}

/// `resumeHeavyStep`: one of the fallback and the worker continuation runs
/// the flow, the other is a no-op.
pub async fn resume_heavy_step(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let Some(key) = str_of(data, "outcomeKey") else {
        return Err(JobError::unrecoverable("resumeHeavyStep has no outcomeKey"));
    };
    let kv = kv(deps);
    let token = chatbotx_db::repo::new_id().to_string();
    let claim = outcome::claim_resume(&kv, key, token.clone())
        .await
        .map_err(db_fail)?;
    if claim != "claimed" {
        return Ok(());
    }
    let flow_execution_key = str_of(data, "flowExecutionKey").unwrap_or("").to_owned();
    let result = crate::flow::run_flow_node(deps, data, flow_execution_key).await;
    outcome::finish_resume(&kv, key, token, result.is_ok())
        .await
        .map_err(db_fail)?;
    result
}

/// `resumeHeavyFlow`: the immediate continuation after the heavy job.
async fn queue_resume(
    deps: &Deps,
    data: &Value,
    job_id: &str,
    key: &str,
    step_id: &str,
) -> Result<(), JobError> {
    let mut resume = data
        .get("continuation")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for name in ["contactInboxId", "conversationId"] {
        if let Some(value) = data.get(name) {
            resume.insert(name.into(), value.clone());
        }
    }
    resume.insert("outcomeKey".into(), json!(key));
    resume.insert("startFromStepId".into(), json!(step_id));
    let metadata = resume.get("metadata").cloned();
    deps.jobs
        .add(
            QueueName::Integration,
            RESUME_HEAVY_STEP,
            json!({ "type": RESUME_HEAVY_STEP, "data": resume }),
            JobOptions {
                job_id: Some(format!("heavy-resume-{job_id}")),
                ..flow_continuation_job_options(metadata.as_ref().filter(|m| !m.is_null()))
            },
        )
        .await
        .map(|_| ())
        .map_err(db_fail)
}

/// `completeHeavyStep`: the output fields are written once.
async fn complete(
    deps: &Deps,
    kv: &DurableKv,
    key: &str,
    result: &HeavyStepResult,
    conversation: &Conversation,
) -> Result<(), JobError> {
    match result {
        HeavyStepResult::Error { error_message } => fail_outcome(kv, key, error_message).await,
        HeavyStepResult::Skip => {
            outcome::transition(kv, key, "succeeded", None, None, Some("skip"))
                .await
                .map_err(db_fail)?;
            Ok(())
        }
        HeavyStepResult::Success { output_values } => {
            let claim =
                outcome::transition(kv, key, "writing", None, Some(output_values.clone()), None)
                    .await
                    .map_err(db_fail)?;
            let Transition::Claimed(stored) = claim else {
                return Ok(());
            };
            for output in stored.as_ref().unwrap_or(output_values) {
                crate::steps::ai_step::save_result_to_custom_field(
                    &deps.db,
                    conversation.workspace_id,
                    conversation.contact_id,
                    &output.custom_field_id,
                    output.value.clone(),
                )
                .await
                .map_err(JobError::retryable)?;
            }
            outcome::transition(kv, key, "succeeded", None, None, Some("success"))
                .await
                .map_err(db_fail)?;
            Ok(())
        }
    }
}

/// The process-wide provider slots (`waitForHeavyProviderSlot`).
static PROVIDER_SLOTS: LazyLock<MemKv> = LazyLock::new(|| MemKv::new(Arc::new(SystemClock)));

/// The heavy queue's flow-step jobs.
pub async fn handle_step_job(deps: &Deps, job: &JobCtx) -> Result<Value, JobError> {
    let parsed = HeavyJobData::parse(&job.data).map_err(|e| JobError::unrecoverable(e.message))?;
    let (action, step_data) = match parsed {
        HeavyJobData::AiEditImage(d) => (HeavyJobAction::AiEditImage, d),
        HeavyJobData::AiGenerateImage(d) => (HeavyJobAction::AiGenerateImage, d),
        HeavyJobData::AiSegmentImage(d) => (HeavyJobAction::AiSegmentImage, d),
        HeavyJobData::AiSpeechToText(d) => (HeavyJobAction::AiSpeechToText, d),
        HeavyJobData::AiTextToSpeech(d) => (HeavyJobAction::AiTextToSpeech, d),
        _ => {
            return Err(JobError::unrecoverable(format!(
                "Heavy job action is not available: {}",
                job.name
            )));
        }
    };
    let data = &job.data["data"];
    let conversation_id = data.get("conversationId").and_then(id_of).unwrap_or(0);
    let conversation = load_conversation(&deps.db, conversation_id)
        .await?
        .ok_or_else(|| JobError::unrecoverable("Conversation not found"))?;
    let contact_inbox_id = data.get("contactInboxId").and_then(id_of).unwrap_or(0);
    let contact_inbox = load_contact_inbox(&deps.db, contact_inbox_id, conversation.contact_id)
        .await?
        .ok_or_else(|| JobError::unrecoverable("Contact inbox not found"))?;
    if crate::is_blocked_workspace(&deps.db, &conversation.workspace_id.to_string()).await? {
        return Ok(Value::Null);
    }
    let step = &step_data.step;
    let step_id = str_of(step, "id").unwrap_or("").to_owned();
    let kv = kv(deps);
    let key = step_data.outcome_key.as_deref();
    let attempt = match key {
        Some(key) => Some(
            outcome::resolve_attempt(&kv, key, SystemClock.now_ms())
                .await
                .map_err(db_fail)?,
        ),
        None => None,
    };
    let result = match attempt {
        Some(Attempt::Skip) => {
            tracing::info!(job_id = %job.job_id, outcome_key = key, "Heavy flow step already reached a terminal outcome");
            return Ok(Value::Null);
        }
        Some(Attempt::Replay(output_values)) => {
            tracing::info!(action = action.as_str(), job_id = %job.job_id, outcome_key = key, "Replaying stored heavy step output");
            HeavyStepResult::Success { output_values }
        }
        _ => {
            let provider = if action == HeavyJobAction::AiSegmentImage {
                "muse"
            } else {
                str_of(step, "provider").unwrap_or("")
            };
            let interval = env_num("HEAVY_PROVIDER_MIN_INTERVAL_MS", 250, 0, 60_000);
            wait_for_heavy_provider_slot(
                &*PROVIDER_SLOTS,
                HeavyProviderSlot {
                    min_interval_ms: interval,
                    provider,
                    workspace_id: &conversation.workspace_id.to_string(),
                },
            )
            .await;
            let input = Compute {
                deps,
                conversation: &conversation,
                metadata: data.get("metadata"),
                step,
                job_id: &job.job_id,
            };
            match compute(&input, action).await {
                Ok(result) => result,
                Err(err) => {
                    record_provider_error(deps, provider, &conversation, &contact_inbox, &err)
                        .await;
                    tracing::error!(
                        err = %err.message,
                        conversation_id = conversation.id,
                        contact_inbox_id = contact_inbox.id,
                        job_id = %job.job_id,
                        job_type = action.as_str(),
                        workspace_id = conversation.workspace_id,
                        retryable = err.retryable,
                        "Heavy step failed"
                    );
                    if err.retryable {
                        // The `failed` hook of the last attempt.
                        if job.is_final_attempt()
                            && let Some(key) = key
                            && data.get("continuation").is_some_and(Value::is_object)
                        {
                            fail_outcome(&kv, key, &err.message).await?;
                            queue_resume(deps, data, &job.job_id, key, &step_id).await?;
                        }
                        return Err(JobError::retryable(err.message));
                    }
                    HeavyStepResult::Error {
                        error_message: if err.message.is_empty() {
                            "Heavy step failed".into()
                        } else {
                            err.message
                        },
                    }
                }
            }
        }
    };
    if let Some(key) = key
        && data.get("continuation").is_some_and(Value::is_object)
    {
        complete(deps, &kv, key, &result, &conversation).await?;
        queue_resume(deps, data, &job.job_id, key, &step_id).await?;
    }
    Ok(result.to_value())
}

/// `recordHeavyAIStepProviderError`.
async fn record_provider_error(
    deps: &Deps,
    provider: &str,
    conversation: &Conversation,
    contact_inbox: &ContactInbox,
    err: &HeavyError,
) {
    let input = crate::events::ProviderError {
        provider: crate::events::ai_error_log_provider(provider).to_owned(),
        workspace_id: conversation.workspace_id.to_string(),
        contact_id: Some(conversation.contact_id.to_string()),
        source_id: Some(contact_inbox.source_id.clone()),
        error: json!({ "message": err.message, "statusCode": err.status }),
        ..Default::default()
    };
    crate::events::log_provider_errors(deps.events.as_ref(), &[input]).await;
}

/// What a step computation reads (`HeavyStepComputeProps`).
struct Compute<'a> {
    deps: &'a Deps,
    conversation: &'a Conversation,
    metadata: Option<&'a Value>,
    step: &'a Value,
    /// The heavy job id: the same for every retry of one run.
    job_id: &'a str,
}

impl Compute<'_> {
    fn str(&self, key: &str) -> &str {
        str_of(self.step, key).unwrap_or("")
    }

    /// `metadata?.stepId ?? step.id`.
    fn execution_id(&self) -> String {
        self.metadata
            .and_then(|m| str_of(m, "stepId"))
            .unwrap_or(self.str("id"))
            .to_owned()
    }
}

async fn compute(c: &Compute<'_>, action: HeavyJobAction) -> Result<HeavyStepResult, HeavyError> {
    let output = |value: String| HeavyStepResult::Success {
        output_values: vec![HeavyOutputValue {
            custom_field_id: c.str("outputFieldId").to_owned(),
            value,
        }],
    };
    match action {
        HeavyJobAction::AiGenerateImage => generate_image_output(c).await.map(output),
        HeavyJobAction::AiEditImage => edit_image_output(c).await.map(output),
        HeavyJobAction::AiTextToSpeech => text_to_speech_output(c).await.map(output),
        HeavyJobAction::AiSpeechToText => speech_to_text_output(c).await.map(output),
        _ => segment_image_output(c).await,
    }
}

/// `aiIntegrationService.findBy({ workspaceId, provider })`.
async fn integration_auth(
    deps: &Deps,
    workspace_id: i64,
    provider: &str,
) -> Result<Option<Value>, HeavyError> {
    let provider = provider.to_owned();
    deps.db
        .read(move |conn| Ok(model::native_auth(conn, workspace_id, &provider, None)?))
        .await
        .map_err(|e| HeavyError::transient(e.to_string()))
}

/// `createAIImageModelInstance` (and the speech provider): the workspace's
/// integration row as a model.
async fn native_model(
    c: &Compute<'_>,
    provider: &str,
    model_id: &str,
) -> Result<Model, HeavyError> {
    let auth = integration_auth(c.deps, c.conversation.workspace_id, provider)
        .await?
        .ok_or_else(|| HeavyError::expected("AI integration not found"))?;
    model::native_model(provider, &auth, model_id).map_err(HeavyError::expected)
}

/// The tenant `storageUrl` (`resolveTenantSettings`).
pub(crate) fn storage_url(deps: &Deps) -> String {
    chatbotx_business::contact::avatar::TenantUrls::from_app_url(
        &deps.app_url,
        std::env::var("NEXT_PUBLIC_STORAGE_URL").ok().as_deref(),
    )
    .storage_url
}

/// `uploader.putObject` then `getPublicFileUrl`.
async fn store(
    deps: &Deps,
    path: &str,
    bytes: Vec<u8>,
    content_type: &str,
) -> Result<String, HeavyError> {
    let files = deps
        .files
        .as_ref()
        .ok_or_else(|| HeavyError::expected("File storage is not configured"))?;
    let options = chatbotx_files::PutOptions {
        content_type: Some(content_type.to_owned()),
        content_length: Some(bytes.len() as u64),
        ..chatbotx_files::PutOptions::default()
    };
    files
        .put_object(path, bytes, options, None)
        .await
        .map_err(|e| HeavyError::transient(e.to_string()))?;
    chatbotx_core::utils::storage::get_public_file_url(path, &storage_url(deps))
        .map_err(|e| HeavyError::transient(e.to_string()))
}

/// `downloadWithByteLimit({ label: "image" })`: SSRF-checked on every hop,
/// at most `HEAVY_MAX_IMAGE_BYTES`.
async fn download_image(url: &str) -> Result<Vec<u8>, HeavyError> {
    download_limited(url, max_image_bytes(), "image")
        .await
        .map(|(bytes, _)| bytes)
}

/// `downloadWithByteLimit`: SSRF-checked on every hop (`download_checked`),
/// at most `max` bytes; `(bytes, content type)`.
async fn download_limited(
    url: &str,
    max: u64,
    label: &str,
) -> Result<(Vec<u8>, String), HeavyError> {
    static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(media_client);
    match download_checked(&CLIENT, url, None, max, &format!("{label} URL")).await {
        Ok((bytes, content_type)) => Ok((bytes.to_vec(), content_type.unwrap_or_default())),
        Err(DownloadError::Unsafe(_)) => Err(HeavyError::expected(format!("Unsafe {label} URL"))),
        Err(DownloadError::Status(status)) => {
            let message = format!("{label} download failed with status {}", status.as_u16());
            if status.is_server_error() {
                Err(HeavyError::transient(message))
            } else {
                Err(HeavyError::expected(message))
            }
        }
        Err(DownloadError::TooLarge) => Err(HeavyError::expected(format!(
            "{label} exceeds size limit (max {max})"
        ))),
        Err(DownloadError::Other(message)) => Err(HeavyError::transient(message)),
    }
}

/// An `http(s)` URL (`z.url({ protocol: /^https?$/ })`).
fn http_url(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    let parsed = url::Url::parse(value).ok()?;
    matches!(parsed.scheme(), "http" | "https").then(|| value.to_owned())
}

async fn input_field_url(c: &Compute<'_>) -> Result<Option<String>, HeavyError> {
    let value = crate::condition::find_custom_field_value(
        &c.deps.db,
        &c.conversation.contact_id.to_string(),
        c.str("inputFieldId"),
    )
    .await
    .map_err(HeavyError::transient)?;
    Ok(http_url(value.as_deref()))
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, HeavyError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| HeavyError::transient(e.to_string()))
}

/// The generated image made channel-safe, bounded and stored as
/// `<storagePrefix>/conversations/<conversationId>/<name>.<ext>`.
async fn store_generated_image(
    c: &Compute<'_>,
    image: Generated,
    name: &str,
) -> Result<String, HeavyError> {
    if image.bytes.is_empty() {
        return Err(HeavyError::transient("Empty image payload from provider"));
    }
    let media_type = image.media_type.clone();
    let safe = blocking(move || to_channel_safe_image(image.bytes, &media_type))
        .await?
        .map_err(HeavyError::transient)?;
    if safe.bytes.len() as u64 > max_image_bytes() {
        return Err(HeavyError::expected(format!(
            "Image too large: {} bytes",
            safe.bytes.len()
        )));
    }
    let raw_ext = safe
        .content_type
        .split('/')
        .nth(1)
        .and_then(|e| e.split(';').next())
        .unwrap_or("")
        .trim()
        .to_owned();
    let extension = if IMAGE_EXTENSIONS.contains(&raw_ext.as_str()) {
        raw_ext
    } else {
        "png".into()
    };
    let path = get_ai_generated_image_path(
        &chatbotx_files::get_storage_prefix(&c.conversation.workspace_id.to_string()),
        &format!("{name}.{extension}"),
        &c.conversation.id.to_string(),
    );
    store(c.deps, &path, safe.bytes, &safe.content_type).await
}

async fn image_bytes(answer: ImageAnswer) -> Result<Generated, HeavyError> {
    match answer {
        ImageAnswer::Bytes(image) => Ok(image),
        // dall-e answers a URL; `createDallEImageFetch` downloads it.
        ImageAnswer::Url(url) => {
            let bytes = download_image(&url).await?;
            Ok(Generated {
                media_type: provider::image_media_type(&bytes),
                bytes,
            })
        }
    }
}

/// `getOpenAIImageQuality`.
fn openai_image_quality(model_id: &str, quality: &str) -> &'static str {
    let gpt = model_id.starts_with("gpt-image") || model_id.starts_with("chatgpt-image");
    match (gpt, quality) {
        (_, "auto") => "auto",
        (true, "hd") => "high",
        (true, "md") => "medium",
        (true, _) => "low",
        (false, "hd") => "hd",
        (false, _) => "standard",
    }
}

/// `getOpenAIEditImageQuality`.
fn openai_edit_image_quality(quality: &str) -> Result<&'static str, HeavyError> {
    Ok(match quality {
        "auto" => "auto",
        "low" | "ld" => "low",
        "medium" | "md" => "medium",
        "high" | "hd" => "high",
        other => {
            return Err(HeavyError::transient(format!(
                "Unsupported OpenAI image quality: {other}"
            )));
        }
    })
}

/// `generateImageOutput`.
async fn generate_image_output(c: &Compute<'_>) -> Result<String, HeavyError> {
    let provider = c.str("provider");
    let model = native_model(c, provider, c.str("model")).await?;
    let size = c.str("size");
    let quality = c.str("quality");
    let quality = (provider == "openai" && quality != "auto")
        .then(|| openai_image_quality(&model.model_id, quality));
    let request = ImageRequest {
        prompt: c.str("prompt"),
        size: (matches!(provider, "openai" | "muse") && size != "auto").then_some(size),
        aspect_ratio: (provider == "gemini" && size != "auto").then_some(size),
        quality,
        input: None,
    };
    let image = image_bytes(provider::generate_image(&model, &request).await?).await?;
    store_generated_image(c, image, &c.execution_id()).await
}

/// `editImageOutput`.
async fn edit_image_output(c: &Compute<'_>) -> Result<String, HeavyError> {
    let image_url = input_field_url(c).await?;
    let provider = c.str("provider");
    let (Some(image_url), false, false, false) = (
        image_url,
        c.str("prompt").trim().is_empty(),
        c.str("model").trim().is_empty(),
        c.str("size").trim().is_empty(),
    ) else {
        tracing::warn!(
            conversation_id = c.conversation.id,
            "[ai-edit-image] Invalid input, skipping"
        );
        return Err(HeavyError::expected("Invalid input for image edit"));
    };
    let auth = integration_auth(c.deps, c.conversation.workspace_id, provider)
        .await?
        .ok_or_else(|| HeavyError::expected("AI integration not found"))?;
    let model = match model::native_model(provider, &auth, c.str("model")) {
        Ok(model) => model,
        Err(err) if provider == "openai" => {
            tracing::warn!(err = %err, model_id = c.str("model"), conversation_id = c.conversation.id, "[ai-edit-image] Failed to create model, attempting fallback");
            model::native_model(provider, &auth, AI_EDIT_IMAGE_FALLBACK_OPENAI_MODEL)
                .map_err(HeavyError::transient)?
        }
        Err(_) => {
            return Err(HeavyError::expected(format!(
                "[ai-edit-image] Cannot create image model for provider: {provider}"
            )));
        }
    };
    let input = download_image(&image_url).await?;
    // The bytes decide, not `content-type` (CDNs serve octet-stream).
    if !matches!(sniff_format(&input), Some("png" | "jpeg" | "webp" | "gif")) {
        return Err(HeavyError::expected("Input is not a supported image"));
    }
    let size = c.str("size");
    let quality = c.str("quality");
    let quality = if provider == "openai" && quality != "auto" {
        Some(openai_edit_image_quality(quality)?)
    } else {
        None
    };
    let request = ImageRequest {
        prompt: c.str("prompt"),
        size: (matches!(provider, "openai" | "muse") && size != "auto").then_some(size),
        aspect_ratio: (provider == "gemini").then_some(size),
        quality,
        input: Some(&input),
    };
    let image = image_bytes(provider::generate_image(&model, &request).await?).await?;
    store_generated_image(c, image, &c.execution_id()).await
}

/// `textToSpeechOutput` + `textToSpeechStorageService.saveAudio`.
async fn text_to_speech_output(c: &Compute<'_>) -> Result<String, HeavyError> {
    let model_id = c.str("model");
    let model = native_model(c, "openai", model_id).await?;
    let tone = c.str("voiceTone");
    let instructions = (model_id == "gpt-4o-mini-tts" && !tone.is_empty()).then_some(tone);
    let audio =
        provider::generate_speech(&model, c.str("message"), c.str("voiceType"), instructions)
            .await?;
    if audio.bytes.is_empty() {
        return Err(HeavyError::transient(
            "[ai-text-to-speech] Empty audio payload from provider",
        ));
    }
    let path = format!(
        "public/space/{}/conversations/{}/{}/{}.mp3",
        c.conversation.workspace_id,
        c.conversation.id,
        c.execution_id(),
        chatbotx_db::repo::new_id()
    );
    store(c.deps, &path, audio.bytes, &audio.media_type).await
}

/// `supportedAudioMimeTypes` (OpenAI transcription input).
const SUPPORTED_AUDIO_MIME_TYPES: &[&str] = &[
    "audio/flac",
    "audio/mpeg",
    "audio/mpga",
    "audio/m4a",
    "audio/mp4",
    "audio/x-m4a",
    "audio/wav",
    "audio/webm",
    "audio/ogg",
    "audio/x-wav",
    "audio/mp3",
];

/// `MUSE_FORMAT_HINT`.
const MUSE_FORMAT_HINT: &str =
    "Muse Voice Transcribe requires WAV PCM 16-bit mono 16/24 kHz ≤ 10 min";

/// `speechToTextOutput`: one attempt under its provider deadline
/// (`getSpeechToTextAttemptTimeoutMs`).
async fn speech_to_text_output(c: &Compute<'_>) -> Result<String, HeavyError> {
    let provider = c.str("provider");
    let timeout = if provider == "muse" {
        std::time::Duration::from_millis(muse_transcribe_timeout_ms())
    } else {
        provider::AI_TOTAL
    };
    tokio::time::timeout(timeout, async {
        let auth = integration_auth(c.deps, c.conversation.workspace_id, provider)
            .await?
            .ok_or_else(|| HeavyError::expected("AI integration not found"))?;
        let audio_url = crate::condition::find_custom_field_value(
            &c.deps.db,
            &c.conversation.contact_id.to_string(),
            c.str("inputFieldId"),
        )
        .await
        .map_err(HeavyError::transient)?
        .filter(|url| !url.is_empty())
        .ok_or_else(|| HeavyError::expected("No audio URL provided"))?;
        match provider {
            "openai" => transcribe_openai(c, &auth, &audio_url).await,
            "muse" => transcribe_muse(c, &auth, &audio_url, timeout).await,
            other => Err(HeavyError::transient(format!(
                "Unsupported speech-to-text step: {other}"
            ))),
        }
    })
    .await
    .map_err(|_| HeavyError::transient("Speech to Text attempt timed out"))?
}

/// `transcribeOpenAI`.
async fn transcribe_openai(
    c: &Compute<'_>,
    auth: &Value,
    audio_url: &str,
) -> Result<String, HeavyError> {
    let model =
        model::native_model("openai", auth, c.str("model")).map_err(HeavyError::expected)?;
    let (audio, content_type) = download_limited(audio_url, max_audio_bytes(), "audio").await?;
    let media_type = content_type.split(';').next().unwrap_or("").trim();
    if !SUPPORTED_AUDIO_MIME_TYPES.contains(&media_type) {
        return Err(HeavyError::expected(format!(
            "Unsupported audio format: {}",
            if content_type.is_empty() {
                "unknown"
            } else {
                &content_type
            }
        )));
    }
    provider::transcribe_openai(&model, &audio, media_type).await
}

/// `transcribeMuse`: the bytes decide (storage serves WAV as octet-stream).
async fn transcribe_muse(
    c: &Compute<'_>,
    auth: &Value,
    audio_url: &str,
    timeout: std::time::Duration,
) -> Result<String, HeavyError> {
    let (audio, _) = download_limited(audio_url, muse_max_audio_bytes(), "audio").await?;
    if let Err(provider::WavFormatError(reason, message)) = provider::inspect_muse_wav(&audio) {
        tracing::info!(
            conversation_id = c.conversation.id,
            provider = "muse",
            reason,
            session_id = c.job_id,
            workspace_id = c.conversation.workspace_id,
            "Muse speech-to-text rejected the audio format"
        );
        return Err(HeavyError::expected(format!(
            "{MUSE_FORMAT_HINT}: {message}"
        )));
    }
    let api_key = model::secret_text(auth)
        .ok_or_else(|| HeavyError::expected("Muse integration credentials are invalid"))?;
    let list = |key: &str| c.step[key].as_array().cloned().unwrap_or_default();
    let result = provider::transcribe_with_muse(
        &api_key,
        &audio,
        c.str("model"),
        &list("languageBias"),
        &list("keywords"),
        c.job_id,
        timeout,
    )
    .await?;
    let text = result.text.trim().to_owned();
    if text.is_empty() {
        return Err(HeavyError::expected("Muse returned an empty transcript"));
    }
    tracing::info!(
        audio_duration_ms = result.audio_duration_ms,
        conversation_id = c.conversation.id,
        muse_session_id = result.session_id,
        provider = "muse",
        session_id = c.job_id,
        transcript_chars = text.chars().count(),
        workspace_id = c.conversation.workspace_id,
        "Muse speech-to-text completed"
    );
    Ok(text)
}

/// `resolveMuseApiKey`.
async fn muse_api_key(c: &Compute<'_>) -> Result<String, HeavyError> {
    let auth = integration_auth(c.deps, c.conversation.workspace_id, "muse")
        .await?
        .ok_or_else(|| HeavyError::expected("Muse integration not found"))?;
    model::secret_text(&auth).ok_or_else(|| HeavyError::expected("Muse integration has no API key"))
}

/// `selectCrop`: `largest` keeps the first of the largest boxes; `all`
/// covers every box. `(crop, selected)`.
fn select_crop(
    boxes: &[provider::SegmentationBox],
    all: bool,
) -> ((u32, u32, u32, u32), Vec<&provider::SegmentationBox>) {
    let area =
        |b: &provider::SegmentationBox| u64::from(b.right - b.left) * u64::from(b.bottom - b.top);
    if all {
        let crop = boxes.iter().fold((u32::MAX, u32::MAX, 0, 0), |c, b| {
            (
                c.0.min(b.left),
                c.1.min(b.top),
                c.2.max(b.right),
                c.3.max(b.bottom),
            )
        });
        return (crop, boxes.iter().collect());
    }
    let mut best = &boxes[0];
    for b in &boxes[1..] {
        if area(b) > area(best) {
            best = b;
        }
    }
    ((best.left, best.top, best.right, best.bottom), vec![best])
}

fn crop_json(left: u32, top: u32, right: u32, bottom: u32) -> String {
    format!("\"left\":{left},\"top\":{top},\"right\":{right},\"bottom\":{bottom}")
}

/// The details field JSON, in the TypeScript key order.
fn details_json(
    count: usize,
    selected: &[&provider::SegmentationBox],
    crop: (u32, u32, u32, u32),
    source: (u32, u32),
) -> String {
    let boxes: Vec<String> = selected
        .iter()
        .map(|b| {
            let mut s = crop_json(b.left, b.top, b.right, b.bottom);
            if let Some(confidence) = b.confidence {
                s.push_str(&format!(",\"confidence\":{}", json!(confidence)));
            }
            format!("{{{s}}}")
        })
        .collect();
    format!(
        "{{\"count\":{count},\"boxes\":[{}],\"crop\":{{{}}},\"source\":{{\"width\":{},\"height\":{}}}}}",
        boxes.join(","),
        crop_json(crop.0, crop.1, crop.2, crop.3),
        source.0,
        source.1
    )
}

/// `segmentImageOutput`.
async fn segment_image_output(c: &Compute<'_>) -> Result<HeavyStepResult, HeavyError> {
    let source_url = input_field_url(c)
        .await?
        .ok_or_else(|| HeavyError::expected("Source field has no image URL"))?;
    let api_key = muse_api_key(c).await?;
    let downloaded = download_image(&source_url).await?;
    let image =
        blocking(move || images::normalize_for_segmentation(&downloaded, SEGMENT_MAX_INPUT_PIXELS))
            .await?
            .map_err(|err| match err {
                NormalizeError::NotAnImage => {
                    HeavyError::expected("Input is not a supported image")
                }
                NormalizeError::TooManyPixels(max) => {
                    HeavyError::expected(format!("Input image exceeds {max} pixels"))
                }
                NormalizeError::Other(message) => HeavyError::transient(message),
            })?;
    if image.bytes.len() as u64 > max_image_bytes() {
        return Err(HeavyError::expected(format!(
            "Normalized image too large: {} bytes",
            image.bytes.len()
        )));
    }
    let size = (image.width, image.height);
    let Some(boxes) = provider::segment_image(
        &api_key,
        &image.bytes,
        &image.mime_type,
        size,
        c.str("target"),
    )
    .await?
    else {
        tracing::info!(
            conversation_id = c.conversation.id,
            job_id = c.job_id,
            matched_count = 0,
            "[ai-segment-image] No object matched"
        );
        return Ok(HeavyStepResult::Skip);
    };
    let (crop, selected) = select_crop(&boxes, c.str("matchSelection") == "all");
    let details = details_json(boxes.len(), &selected, crop, size);
    let output = blocking(move || images::render_crop(&image, crop.0, crop.1, crop.2, crop.3))
        .await?
        .map_err(HeavyError::expected)?;
    if output.bytes.len() as u64 > max_image_bytes() {
        return Err(HeavyError::expected(format!(
            "Cropped image too large: {} bytes",
            output.bytes.len()
        )));
    }
    let extension = if output.content_type == "image/png" {
        "png"
    } else {
        "jpg"
    };
    let path = get_ai_generated_image_path(
        &chatbotx_files::get_storage_prefix(&c.conversation.workspace_id.to_string()),
        &format!("{}.{extension}", c.job_id),
        &c.conversation.id.to_string(),
    );
    let url = store(c.deps, &path, output.bytes, &output.content_type).await?;
    let mut output_values = vec![HeavyOutputValue {
        custom_field_id: c.str("outputFieldId").to_owned(),
        value: url,
    }];
    let details_field = c.str("detailsOutputFieldId");
    if !details_field.is_empty() {
        output_values.push(HeavyOutputValue {
            custom_field_id: details_field.to_owned(),
            value: details,
        });
    }
    tracing::info!(
        conversation_id = c.conversation.id,
        job_id = c.job_id,
        matched_count = boxes.len(),
        "[ai-segment-image] Cropped image"
    );
    Ok(HeavyStepResult::Success { output_values })
}

#[cfg(test)]
mod tests;
