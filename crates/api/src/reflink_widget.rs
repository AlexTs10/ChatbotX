//! `app/api/reflink-widget/[reflinkId]/route.ts`: the public chat-widget
//! read of a ref link (CORS, guest rate limit, visitor language), and
//! `lib/rate-limit/guest-rate-limit.ts`.

use axum::http::HeaderMap;
use axum::response::Response;

use crate::handlers::ApiState;

/// `UNKNOWN_CLIENT_IP`.
pub const UNKNOWN_CLIENT_IP: &str = "unknown";

/// `getGuestClientIp`: the first `x-forwarded-for` entry, else `x-real-ip`,
/// else [`UNKNOWN_CLIENT_IP`].
pub fn guest_client_ip(_headers: &HeaderMap) -> String {
    todo!("P3.6-D1")
}

/// `checkGuestRateLimit` result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestRateLimit {
    pub limited: bool,
    /// Seconds until the next 10 s window.
    pub retry_after: i64,
}

/// `checkGuestRateLimit`: 60 requests per IP (20 per guest conversation) in
/// a fixed 10 s window, on the kv store (a local counter when it is off).
pub fn check_guest_rate_limit(
    _state: &ApiState,
    _webchat_id: &str,
    _client_ip: &str,
    _guest_conversation_id: Option<&str>,
    _now_ms: i64,
) -> GuestRateLimit {
    todo!("P3.6-D1")
}

/// `OPTIONS`: 204 with the CORS headers.
pub fn options(_headers: &HeaderMap) -> Response {
    todo!("P3.6-D1")
}

/// `GET /api/reflink-widget/{reflinkId}`.
pub async fn get(_state: &ApiState, _reflink_id: &str, _headers: &HeaderMap) -> Response {
    todo!("P3.6-D1")
}
