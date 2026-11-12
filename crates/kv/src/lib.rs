//! `chatbotx-kv`: the Redis replacement of P2.7 (spec `conversion/specs/P2.7.md`).
//!
//! One process, no network store. Volatile parts live in process memory
//! (`LockMap`, `MemKv`, `Cache`, `Presence`, `Leaderboards`); the durable
//! list of spec 1.6 lives in the SQLite table `kv_entry` (`DurableKv`).
//!
//! Crate order: `db <- kv <- jobs <- core <- api/rt/server` (spec 2.1).
//!
//! The rate-limit helpers of the frozen app tests (`rate_limit`, `moosend`,
//! `heavy_provider`) are declared here for this step; spec 3.1 places their
//! final homes in `crates/api` / `crates/core`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chatbotx_db::Db;

pub mod cache;
pub mod cas;
pub mod clock;
pub mod durable;
pub mod error;
pub mod heavy_provider;
pub mod leaderboard;
pub mod lock;
pub mod mem;
pub mod moosend;
pub mod presence;
pub mod rate_limit;

pub use cache::{Cache, CacheConfig, CacheOpts};
pub use cas::CasStore;
pub use clock::{Clock, SystemClock, TestClock};
pub use durable::{Admit, AdmitStatus, DurableKv, JsonUpdate};
pub use error::{KvError, NotIntegerWhat};
pub use leaderboard::{
    LeaderboardStats, Leaderboards, MemberValue, RankEntry, RankOf, RanksOf, SetMember, SetScore,
    TopEntry, decode_rank_value, encode_rank_value,
};
pub use lock::{
    LockAcquisitionError, LockGuard, LockMap, LockOpts, RunExclusiveError,
    is_lock_acquisition_error,
};
pub use mem::{MemKv, MemKvConfig};
pub use presence::Presence;

/// A boxed, `Send` future (dyn-compatible async trait methods).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Bounds of the volatile stores (spec 2.3, 2.4, 2.12).
#[derive(Debug, Clone)]
pub struct KvConfig {
    pub mem: MemKvConfig,
    pub cache: CacheConfig,
}

impl Default for KvConfig {
    fn default() -> Self {
        todo!("P2.7 spec 2.12: MemKv 50 000 / 4 MiB, Cache 20 000 / 8 MiB")
    }
}

/// The one handle in the app state (spec 2.1).
pub struct Kv {
    pub locks: LockMap,
    pub mem: MemKv,
    pub cache: Cache,
    pub presence: Presence,
    pub leaderboards: Leaderboards,
    pub durable: DurableKv,
}

impl Kv {
    /// Builds every store and spawns the two sweepers (volatile, durable).
    pub fn start(db: Arc<Db>, clock: Arc<dyn Clock>, cfg: KvConfig) -> Arc<Kv> {
        let _ = (db, clock, cfg);
        todo!("P2.7 spec 2.1")
    }
}
