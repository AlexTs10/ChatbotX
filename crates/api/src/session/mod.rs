//! Sessions: the parts of `packages/auth` (better-auth) the builder uses —
//! the sign-up gate, the generic OAuth path rules and (stage 2) the
//! email/password endpoints under `/api/auth`.

mod claim;
mod email_password;
mod gate;
mod mcp_config;
mod oauth_path;

pub use claim::*;
pub use email_password::*;
pub use gate::*;
pub use mcp_config::*;
pub use oauth_path::*;
