//! Business services (`packages/business/src/<domain>`), one module per domain.
//! Services take the database handle and plain inputs; the HTTP layer
//! (`crates/api`) maps their results and [`errors::BusinessError`]s to the
//! original JSON shapes.

pub mod analytics;
pub mod audit;
pub mod billing;
pub mod bot_field;
pub mod capabilities;
pub mod connection;
pub mod contact;
pub mod contact_custom_field;
pub mod contact_filter;
pub mod contact_note;
pub mod contact_referral_event;
pub mod contact_sequence;
pub mod copy_name;
pub mod count_mode;
pub mod custom_field;
pub mod discount_code;
pub mod ecommerce_open_hours;
pub mod ecommerce_settings;
pub mod error_log;
pub mod errors;
pub mod events;
pub mod flow;
pub mod flow_version;
pub mod folder;
pub mod import;
pub mod inbox;
pub mod inbox_team;
pub mod integration_api;
pub mod integration_webchat;
pub mod invitation;
pub mod keys;
pub mod license;
pub mod minigame;
pub mod net;
pub mod pipeline;
pub mod place_near_me;
pub mod platform_credential;
pub mod poll;
pub mod product;
pub mod product_category;
pub mod product_message;
pub mod qr_code;
pub mod questionnaire;
pub mod quota_enforcement;
pub mod referral;
pub mod reflink;
pub mod sequence;
pub mod store_source;
pub mod tag;
pub mod tenant;
pub mod tenant_help_item;
pub mod tx;
pub mod user_quota;
pub mod vendor;
pub mod workspace;
pub mod workspace_api_token;
pub mod workspace_member;
pub mod workspace_usage;

pub use errors::{BusinessError, Result};
