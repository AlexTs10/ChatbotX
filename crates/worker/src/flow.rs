//! `integration/handlers/flow.ts` (`runFlowNode`, `runNodeSteps`,
//! `executeMultipleStepsGenerator`) and `flow-utils.ts`: one flow step per
//! `sendFlow` job; the next step / next node is a new `sendFlow` job.

use chatbotx_core::flow_config::{is_quick_reply_carrier_step, resolve_flow_action_target};
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::contact_activity::{ContactActivityRecord, upsert_activities};
use chatbotx_db::repo::models::{ContactInbox, Conversation, FlowVersion};
use chatbotx_db::repo::now_ms;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use chatbotx_jobs::chat_wait::{ChatWaitContext, wait_for_chat_job_completion};
use chatbotx_jobs::queues::integration::{BROADCAST_SEND_PRIORITY, flow_continuation_job_options};
use chatbotx_jobs::{JobError, JobOptions, QueueName, WaitTarget};
use ring::digest;
use serde_json::{Map, Value, json};

use crate::integration::is_durable_flow_execution_key;
use crate::{Deps, db_fail, fail, id_of, load_contact_inbox, load_conversation, str_of};

/// `MAX_NODE_EXECUTIONS`.
pub const MAX_NODE_EXECUTIONS: u64 = 3;

pub const SEND_FLOW: &str = "sendFlow";
const SEND_FLOW_MESSAGE: &str = "sendFlowMessage";

/// The steps `flowStepHandlers` maps to `sendFlowMessage`.
const FLOW_MESSAGE_STEP_TYPES: &[&str] = &[
    "sendAudio",
    "sendCard",
    "sendCarousel",
    "sendFile",
    "sendGif",
    "sendImage",
    "sendMultipleImages",
    "sendText",
    "sendVideo",
    "sendQuickReply",
    "sendWaTemplateMessage",
    "sendMessengerTemplateMessage",
    "whatsappOptionList",
    "whatsappCallButton",
    "sendViewCatalog",
    "whatsappFlow",
    "sendProducts",
    "sendCollection",
    "whatsappLocation",
];

/// `flowStepHandlers` entries that are `undefined` (implicit success).
const NO_HANDLER_STEP_TYPES: &[&str] = &[
    "cancelContactInput",
    "landingPage",
    "notifyAgent",
    "openWebsite",
    "openMiniApp",
    "performAction",
    "sendMessengerOtn",
    "setDebounce",
    "chooseChannel",
    "addNotes",
    "waitUserReply",
];

/// The `true` half of `STEP_PRODUCES_MESSAGE` (`MESSAGE_PRODUCING_STEP_TYPES`).
pub const MESSAGE_PRODUCING_STEP_TYPES: &[&str] = &[
    "sendText",
    "sendImage",
    "sendMultipleImages",
    "sendCard",
    "sendCarousel",
    "sendVideo",
    "sendGif",
    "sendAudio",
    "sendFile",
    "sendQuickReply",
    "getUserData",
    "callPhoneNumber",
    "copyText",
    "sendWaTemplateMessage",
    "whatsappOptionList",
    "whatsappCallButton",
    "sendViewCatalog",
    "whatsappFlow",
    "sendProducts",
    "sendCollection",
    "whatsappLocation",
    "sendMessengerTemplateMessage",
];

/// `CONTINUES_AFTER_BRANCH_STEP_TYPES`.
const CONTINUES_AFTER_BRANCH_STEP_TYPES: &[&str] = &["getUserData"];

/// `ExecuteStepResult.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Success,
    Error,
    Skip,
    Wait,
    Retry,
}

impl StepStatus {
    fn as_str(self) -> &'static str {
        match self {
            StepStatus::Success => "success",
            StepStatus::Error => "error",
            StepStatus::Skip => "skip",
            StepStatus::Wait => "wait",
            StepStatus::Retry => "retry",
        }
    }

    fn routes(self) -> bool {
        matches!(
            self,
            StepStatus::Success | StepStatus::Error | StepStatus::Skip
        )
    }
}

/// What one flow run shares (`ExecuteStepsAndQuickRepliesProps`).
pub(crate) struct Run<'a> {
    pub(crate) deps: &'a Deps,
    pub(crate) data: &'a Value,
    pub(crate) conversation: Conversation,
    pub(crate) contact_inbox: ContactInbox,
    pub(crate) flow_version: FlowVersion,
    pub(crate) use_latest_flow_version: bool,
    pub(crate) target_type: &'static str,
    pub(crate) target_id: String,
    pub(crate) target_node_id: String,
    pub(crate) flow_execution_key: String,
    pub(crate) is_bulk_broadcast: Option<bool>,
}

/// `seekConnectedNode`: the target of the edge leaving `source_id`.
pub fn seek_connected_node(edges: &Value, source_id: &str) -> Option<String> {
    edges
        .as_array()?
        .iter()
        .find(|edge| str_of(edge, "sourceHandle") == Some(source_id))
        .and_then(|edge| str_of(edge, "target"))
        .map(str::to_owned)
}

/// `getStateContinuationJobId`.
pub fn state_continuation_job_id(
    flow_execution_key: &str,
    node_visits: Option<&Value>,
    source_step_id: &str,
    state_id: &str,
    target_node_id: &str,
) -> Option<String> {
    if !is_durable_flow_execution_key(Some(flow_execution_key)) {
        return None;
    }
    // Keys in `JSON.stringify` insertion order (already alphabetical).
    let mut identity = Map::new();
    identity.insert("flowExecutionKey".into(), json!(flow_execution_key));
    if let Some(visits) = node_visits.filter(|v| !v.is_null()) {
        identity.insert("nodeVisits".into(), visits.clone());
    }
    identity.insert("sourceStepId".into(), json!(source_step_id));
    identity.insert("stateId".into(), json!(state_id));
    identity.insert("targetNodeId".into(), json!(target_node_id));
    let hash = digest::digest(
        &digest::SHA256,
        Value::Object(identity).to_string().as_bytes(),
    );
    let hex: String = hash.as_ref().iter().map(|b| format!("{b:02x}")).collect();
    Some(format!("flow-state-{hex}"))
}

fn copy_keys(from: &Value, into: &mut Map<String, Value>, keys: &[&str]) {
    for key in keys {
        if let Some(value) = from.get(*key).filter(|v| !v.is_null()) {
            into.insert((*key).into(), value.clone());
        }
    }
}

/// The run fields every continuation carries forward.
const CARRIED: &[&str] = &[
    "metadata",
    "appointmentId",
    "trackingContext",
    "sendFrom",
    "sentByUserId",
    "triggerMessageId",
    "triggerMessageCreatedAt",
    "startedByContactMessage",
];

impl Run<'_> {
    pub(crate) fn metadata(&self) -> Option<&Value> {
        self.data.get("metadata").filter(|m| !m.is_null())
    }

    pub(crate) fn base_job(
        &self,
        node_id: &str,
        node_visits: Option<&Value>,
        anchor: Option<&Value>,
    ) -> Map<String, Value> {
        let mut data = Map::new();
        data.insert(
            "conversationId".into(),
            json!(self.conversation.id.to_string()),
        );
        data.insert(
            "contactInboxId".into(),
            json!(self.contact_inbox.id.to_string()),
        );
        data.insert(
            "flowId".into(),
            json!(self.flow_version.flow_id.to_string()),
        );
        data.insert("nodeId".into(), json!(node_id));
        copy_keys(self.data, &mut data, CARRIED);
        if let Some(bulk) = self.is_bulk_broadcast {
            data.insert("isBulkBroadcast".into(), json!(bulk));
        }
        if let Some(visits) = node_visits.filter(|v| !v.is_null()) {
            data.insert("nodeVisits".into(), visits.clone());
        }
        if let Some(anchor) = anchor.filter(|v| !v.is_null()) {
            data.insert("commentAnchor".into(), anchor.clone());
        }
        data.insert("origin".into(), json!("webhook"));
        data
    }

    pub(crate) fn version_id(&self) -> Option<String> {
        (!self.use_latest_flow_version).then(|| self.flow_version.id.to_string())
    }

    pub(crate) async fn enqueue_send_flow(
        &self,
        data: Map<String, Value>,
        job_id: Option<String>,
    ) -> Result<(), JobError> {
        let metadata = data.get("metadata").cloned();
        let opts = JobOptions {
            job_id,
            ..flow_continuation_job_options(metadata.as_ref())
        };
        self.deps
            .jobs
            .add(
                QueueName::Integration,
                SEND_FLOW,
                json!({ "type": SEND_FLOW, "data": Value::Object(data) }),
                opts,
            )
            .await
            .map(|_| ())
            .map_err(db_fail)
    }

    /// `createContinuationChain(thenSendFlow).enqueue(data)` without a
    /// deferred branch: the pending `thenSendFlow` rides on the first
    /// continuation.
    async fn enqueue_continuation(
        &self,
        mut data: Map<String, Value>,
        then: &mut Option<Value>,
    ) -> Result<(), JobError> {
        if let Some(then) = then.take() {
            data.insert("thenSendFlow".into(), then);
        }
        self.enqueue_send_flow(data, None).await
    }

    /// `chain.finish()`: a run that stops here releases the held
    /// continuation.
    async fn finish(&self, then: &mut Option<Value>) -> Result<(), JobError> {
        match then.take() {
            Some(Value::Object(then)) => self.enqueue_send_flow(then, None).await,
            _ => Ok(()),
        }
    }
}

/// `detectFlowVersion`.
async fn detect_flow_version(
    deps: &Deps,
    flow_id: i64,
    flow_version_id: Option<i64>,
    workspace_id: i64,
) -> Result<(FlowVersion, bool), JobError> {
    let version = deps
        .db
        .read(move |conn| {
            let version_id = match flow_version_id {
                Some(id) => Some(id),
                None => conn
                    .query_row(
                        r#"SELECT "currentVersionId" FROM "Flow"
                           WHERE "id" = ?1 AND "workspaceId" = ?2 AND "active" = 1"#,
                        params![flow_id, workspace_id],
                        |row| row.get::<_, Option<i64>>(0),
                    )
                    .optional()?
                    .flatten(),
            };
            let Some(version_id) = version_id else {
                return Ok(None);
            };
            Ok(conn
                .query_row(
                    &format!(
                        r#"{} WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                        FlowVersion::select_sql()
                    ),
                    params![version_id, workspace_id],
                    FlowVersion::from_row,
                )
                .optional()?)
        })
        .await
        .map_err(db_fail)?;
    match version {
        Some(version) => Ok((version, flow_version_id.is_none())),
        None => Err(fail("FlowVersion not found")),
    }
}

/// `broadcastService.findSendableBroadcast`.
async fn broadcast_is_sendable(deps: &Deps, broadcast_id: i64) -> Result<bool, JobError> {
    deps.db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT 1 FROM "Broadcast"
                       WHERE "id" = ?1 AND "status" = 'sending' AND "deletedAt" IS NULL"#,
                    params![broadcast_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some())
        })
        .await
        .map_err(db_fail)
}

/// `runFlowNode` (`IntegrationJobAction.sendFlow`).
pub async fn run_flow_node(
    deps: &Deps,
    data: &Value,
    flow_execution_key: String,
) -> Result<(), JobError> {
    let Some(flow_id) = data.get("flowId").and_then(id_of) else {
        tracing::debug!("runFlowNode is called without flowId");
        return Ok(());
    };
    let metadata = data.get("metadata").filter(|m| !m.is_null());
    if let Some(meta) = metadata.filter(|m| str_of(m, "type") == Some("broadcast"))
        && let Some(broadcast_id) = meta.get("broadcastId").and_then(id_of)
        && !broadcast_is_sendable(deps, broadcast_id).await?
    {
        // ponytail: `resetContactForResume` on the first dispatch comes with
        // the broadcast unit (Q.W2); a skip without reset under-delivers,
        // never duplicates.
        tracing::debug!(
            broadcast_id,
            "runFlowNode: broadcast no longer sendable, skipping"
        );
        return Ok(());
    }
    let conversation_id = data
        .get("conversationId")
        .and_then(id_of)
        .ok_or_else(|| fail("Conversation not found"))?;
    let conversation = load_conversation(&deps.db, conversation_id)
        .await?
        .ok_or_else(|| fail("Conversation not found"))?;
    let contact_inbox_id = data
        .get("contactInboxId")
        .and_then(id_of)
        .ok_or_else(|| fail("Contact inbox not found"))?;
    let contact_inbox = load_contact_inbox(&deps.db, contact_inbox_id, conversation.contact_id)
        .await?
        .ok_or_else(|| fail("Contact inbox not found"))?;
    let (flow_version, use_latest) = detect_flow_version(
        deps,
        flow_id,
        data.get("flowVersionId").and_then(id_of),
        conversation.workspace_id,
    )
    .await?;
    let nodes = flow_version.nodes.0.as_array().cloned().unwrap_or_default();

    let target_type_in = str_of(data, "targetType");
    let (details, target_type, target_id, target_node_id) =
        if matches!(target_type_in, Some("button" | "quickReply")) {
            let resolved =
                str_of(data, "targetId").and_then(|id| resolve_flow_action_target(&nodes, id));
            let Some(resolved) = resolved else {
                return release_or_fail(
                    deps,
                    data,
                    "FlowVersion does not contain the button/quickReply target",
                )
                .await;
            };
            let target_id = str_of(&resolved.details, "id")
                .unwrap_or_default()
                .to_owned();
            let node_id = resolved
                .node_id
                .clone()
                .or_else(|| str_of(data, "nodeId").map(str::to_owned))
                .unwrap_or_default();
            (resolved.details, resolved.target_type, target_id, node_id)
        } else {
            let node = match str_of(data, "nodeId") {
                Some(node_id) => nodes.iter().find(|n| str_of(n, "id") == Some(node_id)),
                None => nodes
                    .iter()
                    .find(|n| n["data"]["isStartNode"] == json!(true)),
            };
            let Some(node) = node else {
                return release_or_fail(deps, data, "FlowVersion does not contain start node")
                    .await;
            };
            let id = str_of(node, "id").unwrap_or_default().to_owned();
            (node["data"]["details"].clone(), "node", id.clone(), id)
        };

    let is_bulk_broadcast = data
        .get("isBulkBroadcast")
        .and_then(Value::as_bool)
        .or_else(|| (data["initialBroadcastDispatch"] == json!(true)).then_some(true));
    let run = Run {
        deps,
        data,
        conversation,
        contact_inbox,
        flow_version,
        use_latest_flow_version: use_latest,
        target_type,
        target_id,
        target_node_id,
        flow_execution_key,
        is_bulk_broadcast,
    };
    let mut then = data.get("thenSendFlow").filter(|v| v.is_object()).cloned();
    let result = run_node_steps(&run, &details, &mut then).await;
    let result = match result {
        Ok(()) => run.finish(&mut then).await,
        Err(err) => Err(err),
    };
    if let Err(err) = result {
        if let Some(meta) = metadata.filter(|m| str_of(m, "type") == Some("broadcast"))
            && let Some(broadcast_id) = meta.get("broadcastId").and_then(id_of)
        {
            let contact_id = run.conversation.contact_id;
            if let Err(db_err) = deps
                .db
                .write(move |tx| {
                    tx.execute(
                        r#"UPDATE "ContactsOnBroadcasts" SET "failedAt" = ?1
                           WHERE "broadcastId" = ?2 AND "contactId" = ?3"#,
                        params![now_ms(), broadcast_id, contact_id],
                    )?;
                    Ok(())
                })
                .await
            {
                tracing::error!(err = %db_err, "Failed to mark broadcast contact as failed");
            }
        }
        return Err(err);
    }
    Ok(())
}

/// `releaseContinuationOfMissingTarget`: the run's `thenSendFlow` still
/// runs when its target was removed; else the job fails.
async fn release_or_fail(deps: &Deps, data: &Value, message: &str) -> Result<(), JobError> {
    let Some(then) = data.get("thenSendFlow").filter(|v| v.is_object()) else {
        return Err(fail(message));
    };
    tracing::warn!(
        flow_id = %data["flowId"],
        "Flow run target is missing from the flow version; releasing its continuation"
    );
    deps.jobs
        .add(
            QueueName::Integration,
            SEND_FLOW,
            json!({ "type": SEND_FLOW, "data": then }),
            flow_continuation_job_options(then.get("metadata")),
        )
        .await
        .map(|_| ())
        .map_err(db_fail)
}

/// `contactActivityService.record` for `flowExecuted` + `flowNodeExecuted`.
async fn record_flow_activity(run: &Run<'_>) -> Result<(), JobError> {
    let (workspace_id, contact_id, flow_id) = (
        run.conversation.workspace_id,
        run.conversation.contact_id,
        run.flow_version.flow_id,
    );
    let occurred_at = now_ms();
    let mut records = vec![ContactActivityRecord {
        activity_type: "flowExecuted".into(),
        contact_id,
        occurred_at,
        parent_reference_id: 0,
        reference_id: flow_id,
        reference_type: "flow".into(),
    }];
    // ponytail: the integer `referenceId` column only holds numeric node
    // ids; a text node id skips `flowNodeExecuted` (filter "executed step").
    if let Ok(node_id) = run.target_node_id.parse::<i64>() {
        records.push(ContactActivityRecord {
            activity_type: "flowNodeExecuted".into(),
            contact_id,
            occurred_at,
            parent_reference_id: flow_id,
            reference_id: node_id,
            reference_type: "flowNode".into(),
        });
    }
    run.deps
        .db
        .write(move |tx| upsert_activities(tx, workspace_id, &records).map_err(crate::repo_err))
        .await
        .map(|_| ())
        .map_err(db_fail)
}

/// The outcome of one step (`ExecuteMultipleStepsResult`).
struct StepOutcome {
    status: StepStatus,
    branched: bool,
    branch_target_node_id: Option<String>,
    comment_anchor: Option<Value>,
}

/// `runNodeSteps`.
async fn run_node_steps(
    run: &Run<'_>,
    details: &Value,
    then: &mut Option<Value>,
) -> Result<(), JobError> {
    let data = run.data;
    let start_from = str_of(data, "startFromStepId");
    let steps = details
        .get("steps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let quick_replies = details
        .get("quickReplies")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let carrier_id = steps
        .iter()
        .rev()
        .find(|s| is_quick_reply_carrier_step(&run.contact_inbox.channel, s))
        .and_then(|s| str_of(s, "id"))
        .map(str::to_owned);
    if !quick_replies.is_empty() && carrier_id.is_none() && start_from.is_none() {
        tracing::warn!(
            flow_id = run.flow_version.flow_id,
            conversation_id = run.conversation.id,
            channel = %run.contact_inbox.channel,
            target_node_id = %run.target_node_id,
            "Flow node has quick replies but no attachable carrier step for this channel; skipping quick replies"
        );
    }

    let is_entry = start_from.is_none() && !run.target_node_id.is_empty();
    let mut node_visits = data.get("nodeVisits").filter(|v| v.is_object()).cloned();
    if is_entry {
        let count = node_visits
            .as_ref()
            .and_then(|v| v.get(&run.target_node_id))
            .and_then(Value::as_u64)
            .unwrap_or(0)
            + 1;
        if count > MAX_NODE_EXECUTIONS {
            tracing::warn!(
                node_id = %run.target_node_id,
                count,
                max_node_executions = MAX_NODE_EXECUTIONS,
                flow_id = run.flow_version.flow_id,
                conversation_id = run.conversation.id,
                "Flow node exceeded max executions in one run; stopping to prevent an infinite loop"
            );
            return Ok(());
        }
        let mut visits = node_visits
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        visits.insert(run.target_node_id.clone(), json!(count));
        node_visits = Some(Value::Object(visits));
        record_flow_activity(run).await?;
        // ponytail: `clearQuickReplyPendingOnFlowEntry` and
        // `armQuickReplySettings` come with the quick-reply unit.
    }

    let before_step = details.get("beforeStep").filter(|s| s.is_object());
    let before_type = before_step.and_then(|s| str_of(s, "stepType"));
    let skip_before = matches!(run.target_type, "button" | "quickReply")
        && matches!(before_type, Some("startAnotherNode" | "openMiniApp"));
    let mut anchor = data.get("commentAnchor").filter(|v| v.is_object()).cloned();
    let mut branch_target = str_of(data, "branchedNodeId").map(str::to_owned);

    if let Some(before) = before_step
        && start_from.is_none()
        && !skip_before
    {
        let outcome = execute_step(
            run,
            before,
            node_visits.as_ref(),
            anchor.clone(),
            None,
            then,
        )
        .await?;
        anchor = outcome.comment_anchor;
    }

    if !steps.is_empty() {
        let start = match start_from {
            Some(id) => steps.iter().position(|s| str_of(s, "id") == Some(id)),
            None => Some(0),
        };
        let Some(start) = start else {
            tracing::warn!(start_from_step_id = start_from, target_node_id = %run.target_node_id, "startFromStepId not found in node steps");
            return Ok(());
        };
        let step = &steps[start];
        let continues =
            CONTINUES_AFTER_BRANCH_STEP_TYPES.contains(&str_of(step, "stepType").unwrap_or(""));
        let step_quick_replies = (carrier_id.as_deref() == str_of(step, "id")
            && !quick_replies.is_empty())
        .then(|| Value::Array(quick_replies.clone()));
        let outcome = execute_step(
            run,
            step,
            node_visits.as_ref(),
            anchor.clone(),
            step_quick_replies,
            then,
        )
        .await?;
        anchor = outcome.comment_anchor;
        if outcome.branch_target_node_id.is_some() {
            branch_target = outcome.branch_target_node_id.clone();
        }
        if matches!(outcome.status, StepStatus::Wait | StepStatus::Retry) {
            return Ok(());
        }
        if outcome.branched && !continues {
            return Ok(());
        }
        if let Some(next) = steps.get(start + 1) {
            let mut job = run.base_job(&run.target_node_id, node_visits.as_ref(), anchor.as_ref());
            if let Some(version) = run.version_id() {
                job.insert("flowVersionId".into(), json!(version));
            }
            if matches!(run.target_type, "button" | "quickReply") {
                job.insert("targetType".into(), json!(run.target_type));
                job.insert("targetId".into(), json!(run.target_id));
            }
            job.insert("startFromStepId".into(), next["id"].clone());
            if let Some(branch) = &branch_target {
                job.insert("branchedNodeId".into(), json!(branch));
            }
            if is_durable_flow_execution_key(Some(&run.flow_execution_key)) {
                job.insert("flowExecutionKey".into(), json!(run.flow_execution_key));
            }
            return run.enqueue_continuation(job, then).await;
        }
    }

    let follows_edge = run.target_type == "node"
        || (matches!(run.target_type, "button" | "quickReply")
            && (before_step.is_none()
                || matches!(before_type, Some("startAnotherNode" | "openMiniApp"))));
    if !follows_edge {
        return Ok(());
    }
    let Some(next_node_id) = seek_connected_node(&run.flow_version.edges.0, &run.target_id) else {
        return Ok(());
    };
    let exists = run.flow_version.nodes.0.as_array().is_some_and(|nodes| {
        nodes
            .iter()
            .any(|n| str_of(n, "id") == Some(next_node_id.as_str()))
    });
    if !exists || branch_target.as_deref() == Some(next_node_id.as_str()) {
        return Ok(());
    }
    let mut job = run.base_job(&next_node_id, node_visits.as_ref(), anchor.as_ref());
    if let Some(version) = run.version_id() {
        job.insert("flowVersionId".into(), json!(version));
    }
    run.enqueue_continuation(job, then).await
}

/// One `executeMultipleStepsGenerator` iteration: the handler, the
/// comment-anchor claim, then the state routing.
async fn execute_step(
    run: &Run<'_>,
    step: &Value,
    node_visits: Option<&Value>,
    anchor: Option<Value>,
    quick_replies: Option<Value>,
    then: &mut Option<Value>,
) -> Result<StepOutcome, JobError> {
    let step_type = str_of(step, "stepType").unwrap_or("");
    let mut step = step.clone();
    if step.get("nodeId").is_none_or(Value::is_null) {
        step["nodeId"] = json!(run.target_node_id);
    }
    let produces_message = MESSAGE_PRODUCING_STEP_TYPES.contains(&step_type);
    let step_anchor = if produces_message {
        anchor.clone()
    } else {
        None
    };
    let status = run_step_handler(run, &step, node_visits, step_anchor, quick_replies).await?;

    let mut anchor = anchor;
    if produces_message
        && status != StepStatus::Error
        && let Some(current) = &mut anchor
        && str_of(current, "replyChannel") == Some("private")
        && current["spent"] != json!(true)
    {
        current["spent"] = json!(true);
    }

    let mut outcome = StepOutcome {
        status,
        branched: false,
        branch_target_node_id: None,
        comment_anchor: anchor.clone(),
    };
    if !status.routes() {
        return Ok(outcome);
    }
    let state = step
        .get("states")
        .and_then(Value::as_array)
        .and_then(|states| {
            states
                .iter()
                .find(|s| str_of(s, "stateType") == Some(status.as_str()))
        });
    let Some(state_id) = state.and_then(|s| str_of(s, "id")) else {
        return Ok(outcome);
    };
    let Some(connected) = seek_connected_node(&run.flow_version.edges.0, state_id) else {
        return Ok(outcome);
    };
    let mut job = run.base_job(&connected, node_visits, anchor.as_ref());
    if let Some(version) = run.version_id() {
        job.insert("flowVersionId".into(), json!(version));
    }
    if is_durable_flow_execution_key(Some(&run.flow_execution_key)) {
        job.insert("flowExecutionKey".into(), json!(run.flow_execution_key));
    }
    let continues = CONTINUES_AFTER_BRANCH_STEP_TYPES.contains(&step_type);
    // ponytail: a continuing branch (getUserData) is enqueued directly
    // instead of being deferred behind the node's continuation; getUserData
    // itself is not ported yet (Q.W2).
    if !continues && let Some(then) = then.take() {
        job.insert("thenSendFlow".into(), then);
    }
    let job_id = state_continuation_job_id(
        &run.flow_execution_key,
        node_visits,
        str_of(&step, "id").unwrap_or(""),
        state_id,
        &connected,
    );
    run.enqueue_send_flow(job, job_id).await?;
    outcome.branched = true;
    outcome.branch_target_node_id = Some(connected);
    Ok(outcome)
}

/// `flowStepHandlers[step.stepType]`: a missing handler is an implicit
/// success.
async fn run_step_handler(
    run: &Run<'_>,
    step: &Value,
    node_visits: Option<&Value>,
    anchor: Option<Value>,
    quick_replies: Option<Value>,
) -> Result<StepStatus, JobError> {
    let step_type = str_of(step, "stepType").unwrap_or("");
    if FLOW_MESSAGE_STEP_TYPES.contains(&step_type) {
        send_flow_message(run, step, anchor, quick_replies).await?;
        return Ok(StepStatus::Success);
    }
    match step_type {
        "startAnotherNode" => {
            let mut job = run.base_job(
                str_of(step, "nodeId").unwrap_or(""),
                node_visits,
                anchor.as_ref(),
            );
            job.insert(
                "flowVersionId".into(),
                json!(run.flow_version.id.to_string()),
            );
            run.enqueue_send_flow(job, None).await?;
        }
        "startExternalFlow" | "startExternalNode" => {
            let mut job = run.base_job("", node_visits, anchor.as_ref());
            job.insert("flowId".into(), step["flowId"].clone());
            match str_of(step, "nodeId").filter(|_| step_type == "startExternalNode") {
                Some(node_id) => {
                    job.insert("nodeId".into(), json!(node_id));
                }
                None => {
                    job.remove("nodeId");
                }
            }
            run.enqueue_send_flow(job, None).await?;
        }
        other if NO_HANDLER_STEP_TYPES.contains(&other) => {}
        _ => return crate::steps::run(run, step, node_visits, anchor).await,
    }
    Ok(StepStatus::Success)
}

/// `sendFlowMessage` + `enqueueFlowStepMessage`: the `chat` job, then a
/// bounded wait so the node's steps stay in order.
pub(crate) async fn send_flow_message(
    run: &Run<'_>,
    step: &Value,
    anchor: Option<Value>,
    quick_replies: Option<Value>,
) -> Result<(), JobError> {
    let mut extras = Map::new();
    if let Some(replies) = quick_replies {
        extras.insert("quickReplies".into(), replies);
    }
    send_flow_message_with(run, step, anchor, extras).await
}

/// [`send_flow_message`] with extra job fields (e.g. `presetQuickReplies`).
pub(crate) async fn send_flow_message_with(
    run: &Run<'_>,
    step: &Value,
    anchor: Option<Value>,
    extras: Map<String, Value>,
) -> Result<(), JobError> {
    let mut data = Map::new();
    data.insert(
        "conversationId".into(),
        json!(run.conversation.id.to_string()),
    );
    data.insert(
        "contactInboxId".into(),
        json!(run.contact_inbox.id.to_string()),
    );
    data.insert("flowId".into(), json!(run.flow_version.flow_id.to_string()));
    if let Some(version) = run.version_id() {
        data.insert("flowVersionId".into(), json!(version));
    }
    data.insert(
        "executedFlowVersionId".into(),
        json!(run.flow_version.id.to_string()),
    );
    data.insert("step".into(), step.clone());
    copy_keys(
        run.data,
        &mut data,
        &[
            "trackingContext",
            "metadata",
            "sendFrom",
            "sentByUserId",
            "appointmentId",
        ],
    );
    if let Some(bulk) = run.is_bulk_broadcast {
        data.insert("isBulkBroadcast".into(), json!(bulk));
    }
    data.extend(extras);
    if let Some(anchor) = anchor {
        data.insert("commentAnchor".into(), anchor);
    }
    // ponytail: `statusBranches` (delivery-status routing) come with the
    // message-status unit.
    let priority = (run.metadata().and_then(|m| str_of(m, "type")) == Some("broadcast"))
        .then_some(BROADCAST_SEND_PRIORITY);
    let added = run
        .deps
        .jobs
        .add(
            QueueName::Chat,
            SEND_FLOW_MESSAGE,
            json!({ "type": SEND_FLOW_MESSAGE, "data": Value::Object(data) }),
            JobOptions {
                priority,
                ..JobOptions::default()
            },
        )
        .await
        .map_err(db_fail)?;
    let target = WaitTarget {
        queue: Some(QueueName::Chat),
        job_id: added.job_id,
        ..WaitTarget::default()
    };
    let context = ChatWaitContext {
        conversation_id: Some(run.conversation.id.to_string()),
        step_id: str_of(step, "id").map(str::to_owned),
    };
    wait_for_chat_job_completion(run.deps.waiter.as_ref(), Some(&target), &context).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeks_the_target_of_the_edge_leaving_a_handle() {
        let edges = json!([{ "sourceHandle": "a", "target": "n2" }, { "sourceHandle": "b", "target": "n3" }]);
        assert_eq!(seek_connected_node(&edges, "b").as_deref(), Some("n3"));
        assert_eq!(seek_connected_node(&edges, "c"), None);
    }

    #[test]
    fn state_continuation_ids_need_a_durable_key() {
        assert_eq!(
            state_continuation_job_id("flow-inline-x", None, "s", "st", "n"),
            None
        );
        let id = state_continuation_job_id("42:1700", Some(&json!({ "n1": 1 })), "s", "st", "n")
            .unwrap();
        assert!(id.starts_with("flow-state-"));
        assert_eq!(id.len(), "flow-state-".len() + 64);
        assert_ne!(
            Some(id),
            state_continuation_job_id("42:1700", None, "s", "st", "n")
        );
    }
}
