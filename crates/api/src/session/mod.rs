//! Sessions: the parts of `packages/auth` (better-auth) the builder uses —
//! the sign-up gate, the generic OAuth path rules and (stage 2) the
//! email/password endpoints under `/api/auth`.

mod account;
mod auth_mail;
mod bearer;
mod claim;
mod email_password;
mod gate;
mod jwt;
mod magic_link;
mod mcp_config;
mod mcp_oauth;
mod mobile;
mod oauth;
mod oauth_path;
mod origin;
mod rate_limit;
mod resource;
mod social;

pub use account::*;
pub use auth_mail::*;
pub use bearer::*;
pub use claim::*;
pub use email_password::*;
pub use gate::*;
pub use jwt::*;
pub use mcp_config::*;
pub use mcp_oauth::*;
pub use mobile::*;
pub use oauth::*;
pub use oauth_path::*;
pub use origin::*;
pub use rate_limit::*;
pub use resource::*;
pub use social::*;
