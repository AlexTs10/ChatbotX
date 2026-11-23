//! Meta welcome message flows (Click-to-Messenger ads) mirrored from
//! published flows (Q.P7-V5): the enqueue helpers of
//! `messengerWelcomeMessageFlowService`, the Graph calls of
//! `integrations/messenger/src/apis/welcome-message-flows.ts` and the
//! `default` queue jobs `syncWelcomeMessageFlows` / `deleteWelcomeMessageFlows`
//! (`apps/worker/src/default/handlers/welcome-message-flows.ts`).
//!
//! ponytail: one process-wide lock stands in for the per-flow
//! `distributedLock` (one worker process).

use chatbotx_db::Db;
use chatbotx_db::repo::messenger_welcome_message_flow::{
    self as store, CurrentRowInput, PreviousAction,
};
use chatbotx_db::repo::models::{Flow, IntegrationMessenger, MessengerWelcomeMessageFlow};
use chatbotx_db::repo::{find_many, find_one};
use chatbotx_jobs::{Backoff, Dedup, Enqueuer, JobOptions, QueueName, Retention};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::channel_api::{
    ApiFailure, ChannelHttp, GRAPH_API_URL, META_DEFAULT_VERSION, graph_call,
};
use crate::connection::credentials::UpstreamRequest;
use crate::errors::Result;
use crate::messenger_ads_json::{self as ads, Mode, StartNode};
use crate::tx;

/// `DefaultJobAction.syncWelcomeMessageFlows`.
pub const SYNC: &str = "syncWelcomeMessageFlows";
/// `DefaultJobAction.deleteWelcomeMessageFlows`.
pub const DELETE: &str = "deleteWelcomeMessageFlows";

const DEBOUNCE_MS: u64 = 5000;
const SYNC_ATTEMPTS: u32 = 3;
const SYNC_BACKOFF_MS: u64 = 30_000;
const ELIGIBLE_PLATFORMS: [&str; 1] = ["messenger"];

static FLOW_LOCK: Mutex<()> = Mutex::const_new(());

// ---- producers -------------------------------------------------------------------

/// `enqueueSync` (`reason`: `publish` | `rename`): debounced 5 s per flow and
/// reason, 3 attempts. Never fails the caller.
pub async fn enqueue_sync(enqueuer: &dyn Enqueuer, workspace_id: i64, flow_id: i64, reason: &str) {
    let data = json!({ "workspaceId": workspace_id.to_string(), "flowId": flow_id.to_string(), "reason": reason });
    let opts = JobOptions {
        attempts: Some(SYNC_ATTEMPTS),
        backoff: Some(Backoff::Exponential(SYNC_BACKOFF_MS)),
        delay_ms: Some(DEBOUNCE_MS),
        remove_on_complete: Some(Retention::Remove),
        remove_on_fail: Some(Retention::Remove),
        dedup: Some(Dedup {
            id: format!("welcome-message-flow-{reason}-{flow_id}"),
            ttl_ms: Some(DEBOUNCE_MS),
            extend: true,
            replace: true,
            keep_last_if_active: true,
        }),
        ..JobOptions::default()
    };
    if let Err(error) = enqueuer
        .add(
            QueueName::Default,
            SYNC,
            json!({ "type": SYNC, "data": data }),
            opts,
        )
        .await
    {
        tracing::warn!(flow_id, reason, %error, "welcome message flow: failed to enqueue sync");
    }
}

/// `enqueueDelete`: one attempt; nothing for no flows. Never fails the caller.
pub async fn enqueue_delete(enqueuer: &dyn Enqueuer, workspace_id: i64, flow_ids: &[i64]) {
    if flow_ids.is_empty() {
        return;
    }
    let ids: Vec<String> = flow_ids.iter().map(i64::to_string).collect();
    let data = json!({ "workspaceId": workspace_id.to_string(), "flowIds": ids });
    let opts = JobOptions {
        attempts: Some(1),
        remove_on_complete: Some(Retention::Remove),
        remove_on_fail: Some(Retention::Remove),
        ..JobOptions::default()
    };
    if let Err(error) = enqueuer
        .add(
            QueueName::Default,
            DELETE,
            json!({ "type": DELETE, "data": data }),
            opts,
        )
        .await
    {
        tracing::warn!(?flow_ids, %error, "welcome message flow: failed to enqueue delete");
    }
}

// ---- Graph calls -------------------------------------------------------------------

struct Page {
    token: String,
    endpoint: String,
}

fn page_of(row: &IntegrationMessenger) -> Page {
    let auth = &row.auth.0;
    let version = auth["version"]
        .as_str()
        .filter(|v| !v.is_empty())
        .unwrap_or(META_DEFAULT_VERSION);
    let page_id = auth["metadata"]["pageId"].as_str().unwrap_or(&row.page_id);
    Page {
        token: auth["tokens"]["accessToken"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        endpoint: format!("{version}/{page_id}/welcome_message_flows"),
    }
}

async fn call(
    http: &dyn ChannelHttp,
    page: &Page,
    method: &'static str,
    query: Option<(&str, &str)>,
    body: Option<Value>,
) -> std::result::Result<Value, ApiFailure> {
    let mut url = url::Url::parse(&format!("{GRAPH_API_URL}/{}", page.endpoint))
        .map_err(|e| ApiFailure::new(e.to_string()))?;
    if let Some(pair) = query {
        url.query_pairs_mut().append_pair(pair.0, pair.1);
    }
    let request = UpstreamRequest {
        method,
        url: url.to_string(),
        headers: vec![
            ("Content-Type".into(), "application/json".into()),
            ("Authorization".into(), format!("Bearer {}", page.token)),
        ],
        body: body.map(|b| b.to_string()),
    };
    graph_call(http, request, &page.endpoint).await
}

fn flow_body(name: &str, messages: Option<&[Value]>) -> Value {
    let mut body = json!({ "name": name });
    if let Some(messages) = messages {
        body["welcome_message_flow"] = json!(Value::Array(messages.to_vec()).to_string());
        body["eligible_platforms"] = json!(ELIGIBLE_PLATFORMS);
    }
    body
}

/// `createWelcomeMessageFlow`: the Meta `flow_id`.
async fn create(
    http: &dyn ChannelHttp,
    page: &Page,
    name: &str,
    messages: &[Value],
) -> std::result::Result<String, ApiFailure> {
    let reply = call(
        http,
        page,
        "POST",
        None,
        Some(flow_body(name, Some(messages))),
    )
    .await?;
    Ok(match &reply["flow_id"] {
        Value::String(id) => id.clone(),
        other => other.to_string(),
    })
}

/// `updateWelcomeMessageFlow` (`messages: None` = a rename).
async fn update(
    http: &dyn ChannelHttp,
    page: &Page,
    meta_flow_id: &str,
    name: &str,
    messages: Option<&[Value]>,
) -> std::result::Result<(), ApiFailure> {
    let mut body = flow_body(name, messages);
    body["flow_id"] = json!(meta_flow_id);
    call(http, page, "POST", None, Some(body)).await.map(|_| ())
}

async fn delete_on_meta(
    http: &dyn ChannelHttp,
    page: &Page,
    meta_flow_id: &str,
) -> std::result::Result<(), ApiFailure> {
    call(
        http,
        page,
        "DELETE",
        None,
        Some(json!({ "flow_id": meta_flow_id })),
    )
    .await
    .map(|_| ())
}

/// `getWelcomeMessageFlow`: `Some(is_used_in_ad)`; `None` = gone from Meta
/// (code 100, subcode 33).
async fn state(
    http: &dyn ChannelHttp,
    page: &Page,
    meta_flow_id: &str,
) -> std::result::Result<Option<bool>, ApiFailure> {
    match call(http, page, "GET", Some(("flow_id", meta_flow_id)), None).await {
        Ok(reply) => {
            let flow = if reply["data"].is_array() {
                reply["data"][0].clone()
            } else {
                reply
            };
            let has_id =
                flow["id"].as_str().is_some_and(|id| !id.is_empty()) || flow["id"].is_number();
            Ok(has_id.then(|| flow["is_used_in_ad"] == true))
        }
        Err(failure) if failure.code == Some(100) && failure.sub_code == Some(33) => Ok(None),
        Err(failure) => Err(failure),
    }
}

/// `isTransientWelcomeMessageFlowError`: no answer, Meta 5xx, a rate limit
/// or a Graph network code.
pub fn is_transient(failure: &ApiFailure) -> bool {
    const RETRYABLE: &[i64] = &[1, 2, 4, 17, 32, 613, 80_001, 80_006];
    match failure.status {
        None => true,
        Some(status) => status >= 500 || failure.code.is_some_and(|c| RETRYABLE.contains(&c)),
    }
}

/// `formatWelcomeMessageFlowName`.
pub fn format_name(name: &str, version: i64) -> String {
    format!("{name} (v{version})")
}

// ---- rows ---------------------------------------------------------------------------

async fn pages(db: &Db, workspace_id: i64) -> Result<Vec<IntegrationMessenger>> {
    tx::read(db, move |conn| {
        Ok(find_many::<IntegrationMessenger>(
            conn,
            r#"WHERE "workspaceId" = ?1 ORDER BY "id""#,
            &[&workspace_id],
        )?)
    })
    .await
}

async fn current(
    db: &Db,
    flow_id: i64,
    integration_id: i64,
) -> Result<Option<MessengerWelcomeMessageFlow>> {
    tx::read(db, move |conn| {
        Ok(store::find_current(conn, flow_id, integration_id)?)
    })
    .await
}

// ---- `syncWelcomeMessageFlows` --------------------------------------------------------

/// The job failure: `Err(message)` = retry (a transient page failure).
pub type JobResult = std::result::Result<(), String>;

/// `syncWelcomeMessageFlows`: every page of the workspace; a page failure is
/// logged and never stops the others; a transient one fails the job at the end.
pub async fn sync(db: &Db, http: &dyn ChannelHttp, data: &Value) -> JobResult {
    let _guard = FLOW_LOCK.lock().await;
    run_sync(db, http, data).await
}

fn id_of(value: &Value) -> Option<i64> {
    value
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| value.as_i64())
}

async fn run_sync(db: &Db, http: &dyn ChannelHttp, data: &Value) -> JobResult {
    let db_err = |e: crate::errors::BusinessError| e.message();
    let (Some(workspace_id), Some(flow_id)) = (id_of(&data["workspaceId"]), id_of(&data["flowId"]))
    else {
        return Ok(());
    };
    let flow = tx::read(db, move |conn| {
        Ok(find_one::<Flow>(
            conn,
            r#""id" = ?1 AND "workspaceId" = ?2"#,
            &[&flow_id, &workspace_id],
        )?)
    })
    .await
    .map_err(db_err)?;
    let Some(flow) = flow else { return Ok(()) };
    let integrations = pages(db, workspace_id).await.map_err(db_err)?;
    if integrations.is_empty() {
        return Ok(());
    }
    let mut name: Option<String> = None;
    let mut transient = 0usize;

    if data["reason"] == "rename" {
        for integration in &integrations {
            let Some(row) = current(db, flow_id, integration.id).await.map_err(db_err)? else {
                continue;
            };
            if name.is_none() {
                name = Some(format_name(
                    &flow.name,
                    crate::flow_version::published_version_number(db, workspace_id, flow_id)
                        .await
                        .map_err(db_err)?,
                ));
            }
            let page = page_of(integration);
            if let Err(failure) = update(
                http,
                &page,
                &row.meta_flow_id,
                name.as_deref().unwrap_or_default(),
                None,
            )
            .await
            {
                tracing::warn!(flow_id, page_id = integration.page_id, error = %failure.message, "welcome message flow: rename failed");
                transient += usize::from(is_transient(&failure));
            }
        }
        return retry_if(transient);
    }

    let published = ads::start_node(db, workspace_id, flow_id)
        .await
        .map_err(db_err)?;
    let (flow_version_id, messages) = match published {
        StartNode::NotPublished => return Ok(()),
        StartNode::NoStartNode => (None, Vec::new()),
        StartNode::Ok {
            flow_version_id,
            start_node,
        } => {
            let messages = ads::convert(
                &start_node,
                &flow_id.to_string(),
                Some(&flow_version_id.to_string()),
                Mode::Lenient,
            )
            .unwrap_or_default();
            (Some(flow_version_id), messages)
        }
    };
    for integration in &integrations {
        let outcome = publish_on_page(
            db,
            http,
            &flow,
            integration,
            flow_version_id,
            &messages,
            &mut name,
        )
        .await;
        match outcome {
            Ok(()) => {}
            Err(PageError::Provider(failure)) => {
                tracing::warn!(flow_id, page_id = integration.page_id, error = %failure.message, "welcome message flow: sync failed");
                transient += usize::from(is_transient(&failure));
            }
            Err(PageError::Db(message)) => {
                tracing::warn!(flow_id, page_id = integration.page_id, error = %message, "welcome message flow: sync failed");
                transient += 1;
            }
        }
    }
    retry_if(transient)
}

fn retry_if(transient: usize) -> JobResult {
    if transient > 0 {
        return Err("welcome message flow: transient failure, retrying".into());
    }
    Ok(())
}

enum PageError {
    Provider(ApiFailure),
    Db(String),
}

impl From<ApiFailure> for PageError {
    fn from(failure: ApiFailure) -> PageError {
        PageError::Provider(failure)
    }
}

impl From<crate::errors::BusinessError> for PageError {
    fn from(error: crate::errors::BusinessError) -> PageError {
        PageError::Db(error.message())
    }
}

/// `publishOnPage`.
async fn publish_on_page(
    db: &Db,
    http: &dyn ChannelHttp,
    flow: &Flow,
    integration: &IntegrationMessenger,
    flow_version_id: Option<i64>,
    messages: &[Value],
    name: &mut Option<String>,
) -> std::result::Result<(), PageError> {
    let row = current(db, flow.id, integration.id).await?;
    let page = page_of(integration);
    let Some(flow_version_id) = flow_version_id.filter(|_| !messages.is_empty()) else {
        if let Some(row) = row {
            // `removeCurrent`: keep the row as superseded if Meta refuses.
            let id = row.id;
            match delete_on_meta(http, &page, &row.meta_flow_id).await {
                Ok(()) => tx::write(db, move |conn| Ok(store::delete_by_ids(conn, &[id])?)).await?,
                Err(failure) => {
                    tracing::warn!(flow_id = flow.id, error = %failure.message, "welcome message flow: delete failed");
                    tx::write(db, move |conn| Ok(store::mark_superseded(conn, id)?)).await?;
                }
            }
        }
        return Ok(());
    };
    if name.is_none() {
        *name = Some(format_name(
            &flow.name,
            crate::flow_version::published_version_number(db, flow.workspace_id, flow.id).await?,
        ));
    }
    let name = name.clone().unwrap_or_default();

    let mut previous: Option<(i64, PreviousAction)> = None;
    if let Some(row) = &row {
        match update(http, &page, &row.meta_flow_id, &name, Some(messages)).await {
            Ok(()) => {
                let id = row.id;
                tx::write(db, move |conn| {
                    Ok(store::set_flow_version(conn, id, flow_version_id)?)
                })
                .await?;
                return Ok(());
            }
            Err(error) => match state(http, &page, &row.meta_flow_id).await? {
                // Still editable: the update failed for another reason.
                Some(false) => return Err(error.into()),
                Some(true) => {
                    tracing::info!(
                        flow_id = flow.id,
                        "welcome message flow: used in an ad, creating a new one"
                    );
                    previous = Some((row.id, PreviousAction::Supersede));
                }
                None => {
                    tracing::info!(
                        flow_id = flow.id,
                        "welcome message flow: gone from Meta, creating a new one"
                    );
                    previous = Some((row.id, PreviousAction::Delete));
                }
            },
        }
    }
    let meta_flow_id = create(http, &page, &name, messages).await?;
    let next = CurrentRowInput {
        workspace_id: flow.workspace_id,
        flow_id: flow.id,
        integration_messenger_id: integration.id,
        page_id: integration.page_id.clone(),
        meta_flow_id: meta_flow_id.clone(),
        flow_version_id,
    };
    let recorded = tx::write(db, move |conn| {
        match previous {
            Some((id, action)) => store::replace_current(conn, id, action, &next)?,
            None => store::insert_current(conn, &next)?,
        }
        Ok(())
    })
    .await;
    if let Err(error) = recorded {
        // Never leave a Meta flow untracked: a brand-new one is not in an ad yet.
        if let Err(failure) = delete_on_meta(http, &page, &meta_flow_id).await {
            tracing::warn!(meta_flow_id, error = %failure.message, "welcome message flow: cleanup of unrecorded flow failed");
        }
        return Err(error.into());
    }
    Ok(())
}

// ---- `deleteWelcomeMessageFlows` ---------------------------------------------------

/// `deleteWelcomeMessageFlows`: best-effort Meta delete of every recorded flow
/// of the deleted flows; a row whose delete fails (in an ad) stays.
pub async fn delete(db: &Db, http: &dyn ChannelHttp, data: &Value) -> Result<()> {
    let Some(workspace_id) = id_of(&data["workspaceId"]) else {
        return Ok(());
    };
    let integrations = pages(db, workspace_id).await?;
    for flow_id in data["flowIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(id_of)
    {
        let _guard = FLOW_LOCK.lock().await;
        let rows = tx::read(db, move |conn| {
            Ok(store::list_all_by_flow(conn, workspace_id, flow_id)?)
        })
        .await?;
        let mut deleted = Vec::new();
        for row in rows {
            // Workspace-scoped: a row of another workspace's page is skipped.
            let Some(integration) = integrations
                .iter()
                .find(|i| i.id == row.integration_messenger_id)
            else {
                continue;
            };
            match delete_on_meta(http, &page_of(integration), &row.meta_flow_id).await {
                Ok(()) => deleted.push(row.id),
                Err(failure) => {
                    tracing::warn!(flow_id, meta_flow_id = row.meta_flow_id, error = %failure.message, "welcome message flow: delete failed");
                }
            }
        }
        tx::write(db, move |conn| Ok(store::delete_by_ids(conn, &deleted)?)).await?;
    }
    Ok(())
}
