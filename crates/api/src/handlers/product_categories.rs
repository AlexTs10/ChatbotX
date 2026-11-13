//! `features/product-categories/api/public.ts`: `productCategories.*`.

use super::{ApiState, Call, Reply};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "productCategories.list",
    "productCategories.create",
    "productCategories.setVisible",
    "productCategories.update",
    "productCategories.delete",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D1")
}
