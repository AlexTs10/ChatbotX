//! oRPC-compatible errors: the `ORPCError` body (`{ defined, code, status,
//! message, data? }`) and the codes every public route declares
//! (`commonApiErrors` of `apps/builder/src/lib/orpc/orpc-error-helper.ts`).

use serde_json::Value;

/// One declared error code: its default message and HTTP status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommonApiError {
    pub message: &'static str,
    pub status: u16,
}

/// `commonApiErrors`: `(code, default)` in declaration order.
pub fn common_api_errors() -> &'static [(&'static str, CommonApiError)] {
    todo!("P3.5a")
}

/// The declared default of `code`, if any.
pub fn common_api_error(code: &str) -> Option<CommonApiError> {
    let _ = code;
    todo!("P3.5a")
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
        let _ = (code, status, message);
        todo!("P3.5a")
    }

    /// The JSON body oRPC sends for this error.
    pub fn to_json(&self) -> Value {
        todo!("P3.5a")
    }
}
