//! oRPC-compatible errors: the `ORPCError` body (`{ defined, code, status,
//! message, data? }`) and the codes every public route declares
//! (`commonApiErrors` of `apps/builder/src/lib/orpc/orpc-error-helper.ts`).

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

/// One declared error code: its default message and HTTP status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommonApiError {
    pub message: &'static str,
    pub status: u16,
}

/// `commonApiErrors`: `(code, default)` in declaration order.
pub fn common_api_errors() -> &'static [(&'static str, CommonApiError)] {
    COMMON_API_ERRORS
}

/// The declared default of `code`, if any.
pub fn common_api_error(code: &str) -> Option<CommonApiError> {
    COMMON_API_ERRORS
        .iter()
        .find(|(name, _)| *name == code)
        .map(|(_, error)| *error)
}

/// `ORPCError`.
#[derive(Debug, Clone, PartialEq)]
pub struct OrpcError {
    pub code: String,
    pub status: u16,
    pub message: String,
    pub data: Option<Value>,
    /// The code is declared for the route (here: one of the common codes).
    pub defined: bool,
}

impl OrpcError {
    /// `new ORPCError(code, { status?, message? })`.
    pub fn new(code: &str, status: Option<u16>, message: Option<&str>) -> OrpcError {
        let declared = common_api_error(code);
        let fallback = orpc_default(code);
        let status = status
            .or(declared.map(|d| d.status))
            .or(fallback.map(|(s, _)| s))
            .unwrap_or(500);
        let message = message
            .map(str::to_string)
            .or_else(|| declared.map(|d| d.message.to_string()))
            .or_else(|| fallback.map(|(_, m)| m.to_string()))
            .unwrap_or_else(|| code.to_string());
        OrpcError {
            code: code.to_string(),
            status,
            message,
            data: None,
            defined: declared.is_some(),
        }
    }

    /// The JSON body oRPC sends for this error.
    pub fn to_json(&self) -> Value {
        let mut body = Map::new();
        body.insert("defined".into(), json!(self.defined));
        body.insert("code".into(), json!(self.code));
        body.insert("status".into(), json!(self.status));
        body.insert("message".into(), json!(self.message));
        if let Some(data) = &self.data {
            body.insert("data".into(), data.clone());
        }
        Value::Object(body)
    }
}

impl IntoResponse for OrpcError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = self.to_json().to_string();
        (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
    }
}

const fn declared(message: &'static str, status: u16) -> CommonApiError {
    CommonApiError { message, status }
}

/// `commonApiErrors`, in declaration order.
const COMMON_API_ERRORS: &[(&str, CommonApiError)] = &[
    ("UNAUTHORIZED", declared("Authentication required", 401)),
    (
        "INVALID_CHATBOT_TOKEN",
        declared("Invalid or missing workspace API token", 401),
    ),
    (
        "FORBIDDEN",
        declared("You do not have permission to perform this action", 403),
    ),
    (
        "WORKSPACE_SELECTION_REQUIRED",
        declared("Workspace selection is required", 400),
    ),
    ("trialExpired", declared("Trial expired", 403)),
    (
        "macLimitReached",
        declared("Monthly active contact limit reached", 403),
    ),
    (
        "invalidRequestData",
        declared("Input validation failed", 422),
    ),
    ("validation", declared("Validation error", 422)),
    (
        "systemError",
        declared("The request could not be processed", 400),
    ),
    ("tooManyRequests", declared("Too many requests", 429)),
    (
        "INTERNAL_SERVER_ERROR",
        declared("An unexpected error occurred", 500),
    ),
];

/// oRPC's default status and message of a code it knows.
fn orpc_default(code: &str) -> Option<(u16, &'static str)> {
    Some(match code {
        "BAD_REQUEST" => (400, "Bad Request"),
        "UNAUTHORIZED" => (401, "Unauthorized"),
        "FORBIDDEN" => (403, "Forbidden"),
        "NOT_FOUND" => (404, "Not Found"),
        "METHOD_NOT_SUPPORTED" => (405, "Method Not Supported"),
        "NOT_ACCEPTABLE" => (406, "Not Acceptable"),
        "TIMEOUT" => (408, "Request Timeout"),
        "CONFLICT" => (409, "Conflict"),
        "PRECONDITION_FAILED" => (412, "Precondition Failed"),
        "PAYLOAD_TOO_LARGE" => (413, "Payload Too Large"),
        "UNSUPPORTED_MEDIA_TYPE" => (415, "Unsupported Media Type"),
        "UNPROCESSABLE_CONTENT" => (422, "Unprocessable Content"),
        "TOO_MANY_REQUESTS" => (429, "Too Many Requests"),
        "CLIENT_CLOSED_REQUEST" => (499, "Client Closed Request"),
        "INTERNAL_SERVER_ERROR" => (500, "Internal Server Error"),
        "NOT_IMPLEMENTED" => (501, "Not Implemented"),
        "BAD_GATEWAY" => (502, "Bad Gateway"),
        "SERVICE_UNAVAILABLE" => (503, "Service Unavailable"),
        "GATEWAY_TIMEOUT" => (504, "Gateway Timeout"),
        _ => return None,
    })
}
