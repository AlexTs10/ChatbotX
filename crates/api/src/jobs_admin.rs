//! `/developer/queues`: the jobs admin page that replaces Bull Board
//! (`apps/builder/src/app/developer/queues/[[...path]]/route.ts`). Super
//! admins only (`isSuperAdmin`); everyone else gets 404 "Not found". One
//! server-rendered page: job counts per queue and state, the failed jobs of
//! one queue with their reason, and retry / remove buttons (form posts).

use std::fmt::Write as _;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chatbotx_jobs::{JobInspector, JobState, JobStore, QueueName};

use crate::rpc::find_rpc_session;
use crate::session::AuthState;

const BASE: &str = "/developer/queues";
const FAILED_LIMIT: u32 = 100;
const STATES: [JobState; 6] = [
    JobState::Waiting,
    JobState::Prioritized,
    JobState::Delayed,
    JobState::Active,
    JobState::Completed,
    JobState::Failed,
];

/// The page's state: the session lookup and the job store.
pub struct JobsAdminState {
    pub auth: Arc<AuthState>,
    pub store: Arc<JobStore>,
}

/// The `/developer/queues` routes.
pub fn router(state: Arc<JobsAdminState>) -> Router {
    Router::new()
        .route(BASE, get(page))
        .route(&format!("{BASE}/{{queue}}/{{job_id}}/retry"), post(retry))
        .route(&format!("{BASE}/{{queue}}/{{job_id}}/remove"), post(remove))
        .with_state(state)
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not found").into_response()
}

fn internal(error: impl std::fmt::Display) -> Response {
    tracing::error!(%error, "jobs admin page failed");
    (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response()
}

/// `getCurrentUser()` + `isSuperAdmin(user)`.
async fn is_super_admin(
    state: &JobsAdminState,
    headers: &HeaderMap,
) -> Result<bool, Box<Response>> {
    let Some(session) = find_rpc_session(&state.auth, headers)
        .await
        .map_err(|e| Box::new(internal(e)))?
    else {
        return Ok(false);
    };
    let user_id = session.user_id;
    let row = state
        .auth
        .db
        .read(move |conn| {
            chatbotx_db::repo::platform_admin::find_for_auth(conn, user_id)
                .map_err(|e| chatbotx_db::DbError::other(e.message()))
        })
        .await
        .map_err(|e| Box::new(internal(e)))?;
    Ok(row.is_some())
}

/// A post must come from this host (the session cookie is `SameSite=Lax`;
/// this also refuses a same-site form on another subdomain).
fn same_origin(headers: &HeaderMap) -> bool {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        None => true,
        Some(origin) => {
            let origin_host = origin.split_once("://").map_or(origin, |(_, rest)| rest);
            Some(origin_host) == host
        }
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

async fn page(
    State(state): State<Arc<JobsAdminState>>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    match is_super_admin(&state, &headers).await {
        Ok(true) => {}
        Ok(false) => return not_found(),
        Err(response) => return *response,
    }
    let selected = url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .find(|(key, _)| key == "queue")
        .and_then(|(_, value)| QueueName::parse(&value));
    let mut html = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Queues</title><style>\
         body{font-family:system-ui,sans-serif;margin:24px;color:#111}table{border-collapse:collapse;margin-bottom:24px}\
         th,td{border:1px solid #ccc;padding:4px 8px;text-align:left;vertical-align:top}td.n{text-align:right}\
         pre{white-space:pre-wrap;margin:0;max-width:60em}form{display:inline}</style></head><body><h1>Queues</h1>\
         <table><tr><th>Queue</th>",
    );
    for s in STATES {
        let _ = write!(html, "<th>{}</th>", s.as_str());
    }
    html.push_str("</tr>");
    for queue in QueueName::ALL {
        let counts = match state.store.get_job_counts(queue, &STATES).await {
            Ok(counts) => counts,
            Err(error) => return internal(error),
        };
        let name = queue.as_str();
        let _ = write!(
            html,
            "<tr><td><a href=\"{BASE}?queue={}\">{}</a></td>",
            encode(name),
            escape(name)
        );
        for s in STATES {
            let _ = write!(
                html,
                "<td class=\"n\">{}</td>",
                counts.get(&s).copied().unwrap_or(0)
            );
        }
        html.push_str("</tr>");
    }
    html.push_str("</table>");
    if let Some(queue) = selected {
        let failed = match state.store.failed_jobs(queue, FAILED_LIMIT).await {
            Ok(rows) => rows,
            Err(error) => return internal(error),
        };
        let _ = write!(
            html,
            "<h2>Failed jobs: {}</h2><table><tr><th>Job</th><th>Name</th><th>Attempts</th><th>Failed at</th><th>Reason</th><th></th></tr>",
            escape(queue.as_str())
        );
        for job in failed {
            let action = format!("{BASE}/{}/{}", encode(queue.as_str()), encode(&job.job_id));
            let _ = write!(
                html,
                "<tr><td>{}</td><td>{}</td><td class=\"n\">{}</td><td>{}</td><td><pre>{}</pre></td><td>\
                 <form method=\"post\" action=\"{action}/retry\"><button>Retry</button></form> \
                 <form method=\"post\" action=\"{action}/remove\"><button>Remove</button></form></td></tr>",
                escape(&job.job_id),
                escape(&job.name),
                job.attempts_made,
                job.finished_at_ms
                    .and_then(chatbotx_core::js::to_iso_string)
                    .unwrap_or_default(),
                escape(job.failed_reason.as_deref().unwrap_or("")),
            );
        }
        html.push_str("</table>");
    }
    html.push_str("</body></html>");
    Html(html).into_response()
}

async fn act(
    state: &JobsAdminState,
    headers: &HeaderMap,
    queue: &str,
    job_id: &str,
    retry: bool,
) -> Response {
    match is_super_admin(state, headers).await {
        Ok(true) => {}
        Ok(false) => return not_found(),
        Err(response) => return *response,
    }
    if !same_origin(headers) {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    }
    let Some(queue) = QueueName::parse(queue) else {
        return not_found();
    };
    let done = if retry {
        state.store.retry_failed(queue, job_id).await
    } else {
        state.store.remove(queue, job_id).await
    };
    match done {
        Ok(_) => Redirect::to(&format!("{BASE}?queue={}", encode(queue.as_str()))).into_response(),
        Err(error) => internal(error),
    }
}

async fn retry(
    State(state): State<Arc<JobsAdminState>>,
    headers: HeaderMap,
    Path((queue, job_id)): Path<(String, String)>,
) -> Response {
    act(&state, &headers, &queue, &job_id, true).await
}

async fn remove(
    State(state): State<Arc<JobsAdminState>>,
    headers: HeaderMap,
    Path((queue, job_id)): Path<(String, String)>,
) -> Response {
    act(&state, &headers, &queue, &job_id, false).await
}
