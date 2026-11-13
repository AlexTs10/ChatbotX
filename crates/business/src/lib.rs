//! Business services (`packages/business/src/<domain>`), one module per domain.
//! Services take the database handle and plain inputs; the HTTP layer
//! (`crates/api`) maps their results and [`errors::BusinessError`]s to the
//! original JSON shapes.

pub mod errors;
pub mod place_near_me;
pub mod tx;

pub use errors::{BusinessError, Result};
