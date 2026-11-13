//! Business services (`packages/business/src/<domain>`), one module per domain.
//! Services take the database handle and plain inputs; the HTTP layer
//! (`crates/api`) maps their results and [`errors::BusinessError`]s to the
//! original JSON shapes.

pub mod ecommerce_open_hours;
pub mod errors;
pub mod inbox;
pub mod place_near_me;
pub mod poll;
pub mod referral;
pub mod tx;

pub use errors::{BusinessError, Result};
