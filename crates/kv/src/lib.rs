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
use std::sync::{Arc, Weak};

use chatbotx_db::Db;

pub mod cache;
pub mod cas;
pub mod clock;
pub mod durable;
pub mod error;
mod expiring;
pub mod heavy_provider;
pub mod leaderboard;
pub mod lock;
pub mod mem;
pub mod moosend;
mod num;
pub mod presence;
pub mod rate_limit;
mod sweeper;

pub use cache::{Cache, CacheConfig, CacheOpts};
pub use cas::CasStore;
pub use clock::{Clock, SystemClock, TestClock};
pub use durable::{Admit, AdmitStatus, DurableKv, JsonUpdate};
pub use error::{KvError, NotIntegerWhat};
pub use leaderboard::{
    LeaderboardStats, Leaderboards, MemberValue, RankEntry, RankOf, RanksOf, SetMember, SetScore,
    TopEntry, decode_rank_value, encode_rank_value, tie_seconds_since,
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
#[derive(Debug, Clone, Default)]
pub struct KvConfig {
    pub mem: MemKvConfig,
    pub cache: CacheConfig,
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
    /// Must run inside a tokio runtime. The sweepers stop when the last
    /// `Arc<Kv>` is dropped.
    pub fn start(db: Arc<Db>, clock: Arc<dyn Clock>, cfg: KvConfig) -> Arc<Kv> {
        let volatile = sweeper::SweepSignal::new();
        let kv = Arc::new(Kv {
            locks: LockMap::new(),
            mem: MemKv::build(Arc::clone(&clock), cfg.mem, Arc::clone(&volatile)),
            cache: Cache::build(Arc::clone(&clock), cfg.cache, Arc::clone(&volatile)),
            presence: Presence::build(Arc::clone(&clock), Arc::clone(&volatile)),
            leaderboards: Leaderboards::build(Arc::clone(&clock), Arc::clone(&volatile)),
            durable: DurableKv::new(db, Arc::clone(&clock)),
        });
        let durable_signal = Arc::clone(kv.durable.signal());
        tokio::spawn(sweeper::run_sweeper(
            Arc::new(VolatileTarget(Arc::downgrade(&kv))),
            volatile,
            Arc::clone(&clock),
            sweeper::VOLATILE_MIN_INTERVAL_MS,
        ));
        tokio::spawn(sweeper::run_sweeper(
            Arc::new(DurableTarget(Arc::downgrade(&kv))),
            durable_signal,
            clock,
            sweeper::DURABLE_MIN_INTERVAL_MS,
        ));
        kv
    }

    /// Estimated bytes of the volatile stores (replaces Redis `INFO memory`
    /// for the leaderboard maintenance check).
    pub fn used_memory(&self) -> usize {
        self.mem.used_bytes()
            + self.cache.used_bytes()
            + self.presence.used_bytes()
            + self.leaderboards.used_memory()
    }
}

impl Drop for Kv {
    fn drop(&mut self) {
        self.mem.signal().close();
        self.durable.signal().close();
    }
}

/// The volatile stores of one `Kv` (one sweeper, spec 2.3).
struct VolatileTarget(Weak<Kv>);

impl sweeper::SweepTarget for VolatileTarget {
    fn next_expiry(&self) -> BoxFuture<'_, Option<Option<i64>>> {
        Box::pin(async move {
            let kv = self.0.upgrade()?;
            Some(
                [
                    kv.mem.next_expiry(),
                    kv.cache.next_expiry(),
                    kv.presence.next_expiry(),
                    kv.leaderboards.next_expiry(),
                ]
                .into_iter()
                .flatten()
                .min(),
            )
        })
    }

    fn sweep(&self) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let Some(kv) = self.0.upgrade() else {
                return false;
            };
            kv.mem.sweep_expired();
            kv.cache.sweep_expired();
            kv.presence.sweep_expired();
            kv.leaderboards.sweep_expired();
            true
        })
    }
}

/// The `kv_entry` table of one `Kv` (one sweeper, spec 2.9).
struct DurableTarget(Weak<Kv>);

impl DurableTarget {
    fn durable(&self) -> Option<DurableKv> {
        self.0.upgrade().map(|kv| kv.durable.clone())
    }
}

impl sweeper::SweepTarget for DurableTarget {
    fn next_expiry(&self) -> BoxFuture<'_, Option<Option<i64>>> {
        Box::pin(async move {
            let durable = self.durable()?;
            match durable.next_expiry().await {
                Ok(next) => Some(next),
                Err(err) => {
                    // Retry at the floor (`max(MIN, floor)` = floor).
                    tracing::warn!(err = %err, "kv sweeper: cannot read the next expiry");
                    Some(Some(i64::MIN))
                }
            }
        })
    }

    fn sweep(&self) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let Some(durable) = self.durable() else {
                return false;
            };
            loop {
                match durable.sweep_expired().await {
                    Ok(deleted) if deleted >= durable::SWEEP_BATCH => continue,
                    Ok(_) => break,
                    Err(err) => {
                        tracing::warn!(err = %err, "kv sweeper: cannot delete expired rows");
                        break;
                    }
                }
            }
            true
        })
    }
}
