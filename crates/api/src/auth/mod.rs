//! Workspace API token auth: scopes, the read-only rule and the owner access gate.

mod token_scope;
mod workspace_access;

pub use token_scope::*;
pub use workspace_access::*;
