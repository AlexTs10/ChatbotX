//! Handlers of the public `/v1` routes (`apps/builder/src/features/*/api/public*.ts`),
//! one module per builder router. [`dispatch`] runs the handler of a route after
//! its auth; `None` = the route has no Rust handler yet (the "not ported" answer).

pub mod channels;
pub mod connections;
pub mod contact_notes;
pub mod contacts;
pub mod custom_fields;
pub mod ecommerce_settings;
pub mod flows;
pub mod input;
pub mod open_hours;
pub mod places;
pub mod polls;
pub mod product_categories;
pub mod product_vendors;
pub mod products;
pub mod qr_codes;
pub mod reflinks;
pub mod tags;
pub mod webchats;

use std::sync::Arc;

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chatbotx_business::BusinessError;
use chatbotx_db::Db;
use serde_json::Value;

use crate::orpc::{OrpcError, common_api_error};

/// What the handlers need: the database and the public app URL (later: kv,
/// jobs, realtime).
#[derive(Clone)]
pub struct ApiState {
    pub db: Arc<Db>,
    /// The tenant `appUrl` (`NEXT_PUBLIC_BUILDER_URL`, no trailing `/`).
    pub app_url: String,
}

/// One authenticated call of a public route.
#[derive(Debug, Clone, Copy)]
pub struct Call<'a> {
    /// `context.workspace.id`.
    pub workspace_id: i64,
    /// Path params (percent-decoded).
    pub params: &'a [(&'static str, String)],
    /// The raw query string.
    pub query: Option<&'a str>,
    /// The raw request body.
    pub body: &'a [u8],
}

/// The calling workspace token (`context.apiToken`). The default (no token
/// id, `scopes: None` = every scope) is what [`dispatch`] uses.
#[derive(Debug, Clone, Default)]
pub struct Principal {
    /// `context.apiToken.id`.
    pub token_id: Option<i64>,
    /// `context.apiToken.scopes` (`None` = unrestricted).
    pub scopes: Option<Vec<String>>,
}

/// A handler outcome.
pub type Reply = Result<Response, OrpcError>;

/// The error map entries of a route (`code`, `status`): a thrown error with
/// one of these codes and statuses is `defined: true`.
pub type ErrorMap = &'static [(&'static str, u16)];

/// `possibleErrorsOnFindingResource`.
pub const FINDING: ErrorMap = &[("notFound", 404), ("businessError", 400)];
/// `possibleErrorsOnListingResource`.
pub const LISTING: ErrorMap = &[("businessError", 400)];
/// `possibleErrorsOnMutatingResource` / `possibleErrorsOnDeletingResource`.
pub const MUTATING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];
/// `possibleErrorsOnCreatingResource`.
pub const CREATING: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

/// `200` (or `status`) with a JSON body.
pub fn json(status: u16, body: Value) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// `204` without a body.
pub fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

/// A Date output field: ISO 8601 with milliseconds.
pub fn date(ms: i64) -> Value {
    Value::String(chatbotx_db::repo::iso(ms))
}

/// An optional Date output field.
pub fn opt_date(ms: Option<i64>) -> Value {
    ms.map_or(Value::Null, date)
}

/// A bigint id output field (a string).
pub fn id(value: i64) -> Value {
    Value::String(value.to_string())
}

/// An optional bigint id output field.
pub fn opt_id(value: Option<i64>) -> Value {
    value.map_or(Value::Null, id)
}

/// A JavaScript number: whole values print without a fraction (`10`, not `10.0`).
pub fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        // Whole and in the safe range: the cast is exact (`-0` prints as `0`).
        Value::from(value as i64)
    } else {
        serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
    }
}

/// `toKnownOrpcError` of a service error, `defined` per the route's error map
/// (plus `commonApiErrors`). A repository error is a 500.
pub fn business_error(error: BusinessError, errors: ErrorMap) -> OrpcError {
    match error {
        BusinessError::Exception {
            message,
            code,
            status,
            data,
            ..
        } => {
            let message = display_message(&message, &code, data.as_ref());
            let mut out = OrpcError::new(&code, Some(status), Some(&message));
            out.defined = errors.iter().any(|(c, s)| *c == code && *s == status)
                || common_api_error(&code).is_some_and(|d| d.status == status);
            out.data = if out.defined { data } else { None };
            out
        }
        BusinessError::Repo(error) => internal_error(&error.message()),
    }
}

/// JS `String(value)`.
fn js_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(n) => n
            .as_f64()
            .map_or_else(|| n.to_string(), |f| number(f).to_string()),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                if item.is_null() {
                    String::new()
                } else {
                    js_string(item)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

/// `toDisplayMessage` (`apps/builder/src/orpc.ts`): a `ChatbotXException`
/// with `data` shows it as ` (key=value, ...)`. A 422 `invalidRequestData`
/// carries zod issues (a `ZodError` in the original), so it is left as it is.
fn display_message(message: &str, code: &str, data: Option<&Value>) -> String {
    let Some(Value::Object(map)) = data else {
        return message.to_string();
    };
    if code == "invalidRequestData" || map.is_empty() {
        return message.to_string();
    }
    let params = map
        .iter()
        .map(|(key, value)| format!("{key}={}", js_string(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{message} ({params})")
}

/// `500 INTERNAL_SERVER_ERROR` (the cause is logged, not sent).
pub fn internal_error(cause: &str) -> OrpcError {
    tracing::error!(cause, "public API handler failed");
    let mut out = OrpcError::new(
        "INTERNAL_SERVER_ERROR",
        Some(500),
        Some("Internal server error"),
    );
    out.defined = false;
    out
}

/// Runs the handler of `route_id` with an unrestricted token; `None` = not
/// ported yet.
pub async fn dispatch(state: &ApiState, route_id: &str, call: Call<'_>) -> Option<Response> {
    dispatch_as(state, route_id, call, &Principal::default()).await
}

/// Runs the handler of `route_id` for `principal`; `None` = not ported yet.
pub async fn dispatch_as(
    state: &ApiState,
    route_id: &str,
    call: Call<'_>,
    principal: &Principal,
) -> Option<Response> {
    let reply = if connections::ROUTES.contains(&route_id) {
        connections::handle(state, route_id, call, principal).await?
    } else if contact_notes::ROUTES.contains(&route_id) {
        contact_notes::handle(state, route_id, call).await
    } else if tags::ROUTES.contains(&route_id) {
        tags::handle(state, route_id, call).await
    } else if contacts::ROUTES.contains(&route_id) {
        contacts::handle(state, route_id, call).await
    } else if custom_fields::ROUTES.contains(&route_id) {
        custom_fields::handle(state, route_id, call).await
    } else if ecommerce_settings::ROUTES.contains(&route_id) {
        ecommerce_settings::handle(state, route_id, call).await
    } else if flows::ROUTES.contains(&route_id) {
        flows::handle(state, route_id, call).await
    } else if webchats::ROUTES.contains(&route_id) {
        webchats::handle(state, route_id, call).await
    } else if places::ROUTES.contains(&route_id) {
        places::handle(state, route_id, call).await
    } else if open_hours::ROUTES.contains(&route_id) {
        open_hours::handle(state, route_id, call).await
    } else if polls::ROUTES.contains(&route_id) {
        polls::handle(state, route_id, call).await
    } else if product_categories::ROUTES.contains(&route_id) {
        product_categories::handle(state, route_id, call).await
    } else if product_vendors::ROUTES.contains(&route_id) {
        product_vendors::handle(state, route_id, call).await
    } else if products::ROUTES.contains(&route_id) {
        products::handle(state, route_id, call).await
    } else if qr_codes::ROUTES.contains(&route_id) {
        qr_codes::handle(state, route_id, call).await
    } else if reflinks::ROUTES.contains(&route_id) {
        reflinks::handle(state, route_id, call).await
    } else {
        return None;
    };
    Some(match reply {
        Ok(response) => response,
        Err(error) => error.into_response(),
    })
}
