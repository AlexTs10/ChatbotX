//! `features/sequences/api/public.ts` (P3.6-D2 slice contact sequences):
//! `sequences.create` only.

use super::{ApiState, Call, Reply};

/// The routes of this module.
pub const ROUTES: &[&str] = &["sequences.create"];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D2")
}
