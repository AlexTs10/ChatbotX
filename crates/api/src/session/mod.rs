//! Sessions: the parts of `packages/auth` (better-auth) the builder uses —
//! the sign-up gate, the generic OAuth path rules and (stage 2) the
//! email/password endpoints under `/api/auth`.

mod gate;
mod oauth_path;

pub use gate::*;
pub use oauth_path::*;
