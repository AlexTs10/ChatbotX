//! Business services (`packages/business/src/<domain>`), one module per domain.
//! Services take the database handle and plain inputs; the HTTP layer
//! (`crates/api`) maps their results and [`errors::BusinessError`]s to the
//! original JSON shapes.

pub mod discount_code;
pub mod ecommerce_open_hours;
pub mod errors;
pub mod inbox;
pub mod place_near_me;
pub mod poll;
pub mod product;
pub mod product_category;
pub mod product_message;
pub mod qr_code;
pub mod referral;
pub mod reflink;
pub mod store_source;
pub mod tx;
pub mod vendor;

pub use errors::{BusinessError, Result};
