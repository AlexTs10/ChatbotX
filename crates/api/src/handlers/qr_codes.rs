//! `features/qr-codes/api/public.ts`: `qrCodes.*`.

use super::{ApiState, Call, Reply};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "qrCodes.list",
    "qrCodes.create",
    "qrCodes.get",
    "qrCodes.update",
    "qrCodes.delete",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D1")
}
