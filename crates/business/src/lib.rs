//! Business services (`packages/business/src/<domain>`), one module per domain.
//! Services take the database handle and plain inputs; the HTTP layer
//! (`crates/api`) maps their results and [`errors::BusinessError`]s to the
//! original JSON shapes.

pub mod connection;
pub mod discount_code;
pub mod ecommerce_open_hours;
pub mod ecommerce_settings;
pub mod errors;
pub mod flow;
pub mod inbox;
pub mod integration_api;
pub mod integration_webchat;
pub mod net;
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
pub mod workspace_api_token;

pub use errors::{BusinessError, Result};
