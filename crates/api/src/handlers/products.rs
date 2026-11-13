//! `features/products/api/public.ts`: the catalog routes of `products.*`
//! (the Meta catalog and import routes are not ported yet).

use super::{ApiState, Call, Reply};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "products.list",
    "products.listSendable",
    "products.get",
    "products.create",
    "products.update",
    "products.delete",
    "products.setActive",
    "products.deleteMany",
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let _ = (state, route_id, call);
    todo!("P3.6-D1")
}
