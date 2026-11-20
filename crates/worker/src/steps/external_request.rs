//! `externalRequest` of `tool-handler.ts` (the `callApi` step): the step's
//! `{{variables}}` are resolved (JSON-escaped inside a JSON body), the request
//! runs through `chatbotx_business::external_request` and the mapped answer
//! fields are written. A non-2xx answer or a failure routes to the error
//! state and is logged as an `external-request` provider error.

use std::collections::HashMap;

use chatbotx_business::error_log::{ProviderErrorInput, Thrown, log_provider_error};
use chatbotx_business::external_request::{self, Hop};
use chatbotx_business::net::ssrf_guard::HttpDoh;
use chatbotx_core::variables::text::{extract_variables, interpolate};
use chatbotx_jobs::JobError;
use serde_json::{Map, Value};

use super::{StepCtx, step_error};
use crate::flow::StepStatus;

#[cfg(test)]
thread_local! {
    /// The test transport (tests run on one thread).
    pub(crate) static TEST_HOP: std::cell::RefCell<Option<std::sync::Arc<dyn Hop>>> =
        const { std::cell::RefCell::new(None) };
}

fn hop() -> std::sync::Arc<dyn Hop> {
    #[cfg(test)]
    if let Some(hop) = TEST_HOP.with(|h| h.borrow().clone()) {
        return hop;
    }
    static HOP: std::sync::OnceLock<std::sync::Arc<dyn Hop>> = std::sync::OnceLock::new();
    HOP.get_or_init(|| std::sync::Arc::new(external_request::ReqwestHop::default()))
        .clone()
}

/// `resolveJsonBodyVariables`: each known `{{name}}` is replaced by its value
/// JSON-escaped (`JSON.stringify(v).slice(1, -1)`); an unknown one stays.
async fn resolve_json_body(ctx: &StepCtx<'_, '_>, body: &str) -> Result<String, String> {
    let names = extract_variables(body);
    if names.is_empty() {
        return Ok(body.to_owned());
    }
    let probe: Map<String, Value> = names
        .iter()
        .map(|n| (n.clone(), Value::String(format!("{{{{{n}}}}}"))))
        .collect();
    let resolved = ctx.resolve_variables(&Value::Object(probe), false).await?;
    let mut mapping = HashMap::new();
    for name in &names {
        let Some(value) = resolved.get(name).and_then(Value::as_str) else {
            continue;
        };
        if value == format!("{{{{{name}}}}}") {
            continue;
        }
        let escaped = Value::String(value.to_owned()).to_string();
        mapping.insert(name.clone(), escaped[1..escaped.len() - 1].to_owned());
    }
    Ok(interpolate(body, &mapping, false))
}

async fn log_failure(ctx: &StepCtx<'_, '_>, message: &str, http_code: Option<String>) {
    let run = ctx.run;
    let mut input = ProviderErrorInput::new(
        "external-request",
        &ctx.workspace_id().to_string(),
        Thrown::Error {
            message: message.to_owned(),
            stack: None,
            status: None,
        },
    );
    input.contact_id = Some(ctx.contact_id().to_string());
    input.source_id = Some(run.contact_inbox.source_id.clone());
    input.http_code = Some(http_code);
    log_provider_error(run.deps.events.as_ref(), input).await;
}

pub(super) async fn handle(ctx: &StepCtx<'_, '_>) -> Result<StepStatus, JobError> {
    let mut step = ctx.step.clone();
    let body = step
        .as_object_mut()
        .and_then(|s| s.remove("body"))
        .unwrap_or(Value::Null);
    let resolved = async {
        let step = ctx.resolve_variables(&step, false).await?;
        let body = match body.get("bodyType").and_then(Value::as_str) {
            Some("json") => {
                let mut body = body.clone();
                let text = body["jsonBody"].as_str().unwrap_or("").to_owned();
                body["jsonBody"] = Value::String(resolve_json_body(ctx, &text).await?);
                body
            }
            _ => ctx.resolve_variables(&body, false).await?,
        };
        Ok::<_, String>((step, body))
    }
    .await;
    let (mut step, body) = match resolved {
        Ok(resolved) => resolved,
        Err(error) => return Err(crate::fail(error)),
    };
    if !body.is_null() {
        step["body"] = body;
    }
    let mapping = step.get("mapping").cloned().unwrap_or(Value::Null);
    let outcome = external_request::execute_and_map_with(
        &ctx.run.deps.db,
        hop().as_ref(),
        &HttpDoh,
        ctx.workspace_id(),
        ctx.contact_id(),
        &step,
        &mapping,
    )
    .await;
    match outcome {
        Ok(result) if result.status_code >= 400 => {
            let message = format!("Request failed with status {}", result.status_code);
            log_failure(ctx, &message, Some(result.status_code.to_string())).await;
            Ok(step_error(ctx, &message))
        }
        Ok(_) => Ok(StepStatus::Success),
        Err(error) => {
            tracing::error!(err = %error.message(), workspace_id = ctx.workspace_id(), "[externalRequest] failed");
            log_failure(ctx, &error.message(), None).await;
            Ok(step_error(ctx, &error.message()))
        }
    }
}
