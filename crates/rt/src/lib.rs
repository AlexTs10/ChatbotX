//! `chatbotx-rt`: the realtime gateway of P2.8 (spec
//! `conversion/specs/P2.8.md`).
//!
//! An axum WebSocket route on the one server and an in-process
//! `tokio::sync::broadcast` fan-out replace `apps/realtime` (uWebSockets.js,
//! Redis Streams) and the Redis part of the producers. Crate order:
//! `db <- kv <- {jobs, rt} <- core <- {api, server}` (decision D1).
//!
//! The socket is server -> client only: the gateway authenticates,
//! replays and pushes. Producers call [`Realtime`] directly.

pub mod config;
pub mod delivery;
pub mod gateway;
pub mod hub;
pub mod presence;
pub mod protocol;
pub mod publish;
pub mod relay_metrics;
pub mod revoke;
pub mod token;

pub use chatbotx_kv::{BoxFuture, Clock, SystemClock, TestClock};
pub use config::{RtConfig, RtConfigError};
pub use gateway::{OpenOutcome, router};
pub use presence::{KvPresence, NoPresence, PresenceSink};
pub use protocol::{Envelope, EventType, Route, Seq};
pub use publish::{PublishError, Realtime, RealtimeDeps};
pub use revoke::{NoRevokeRetry, RevokeArgs, RevokeError, RevokeReason, RevokeRetry, RevokeStore};

/// A boxed error (`Send + Sync`).
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;
