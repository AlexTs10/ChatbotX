//! `features/product-vendors/api/public.ts`: `productVendors.*`.

use super::{ApiState, Call, Reply};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "productVendors.list",
    "productVendors.create",
    "productVendors.update",
    "productVendors.delete",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D1")
}
