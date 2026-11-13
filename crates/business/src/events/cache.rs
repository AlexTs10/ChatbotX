//! `base-cache.ts` (`TriggerCache`, `WebhookCache`): per workspace, the
//! condition types of the active triggers / webhooks and their source ids
//! (`"*"` = any source), so an emitter can skip events nobody listens for.
//! The original kept a RAM layer (5 s) over a Redis layer (1 h); one process
//! needs only the RAM layer (see the spec "Decisions").

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chatbotx_db::Db;

use super::BoxFuture;

/// The `(type, sourceId)` pairs of the conditions of one workspace.
pub type Conditions = Vec<(String, Option<String>)>;

/// A condition-source error (a failed query).
pub type CacheError = String;

/// The `(type, sourceId)` of every condition of the active rows of one
/// workspace (`db.query.<table>.findMany({ where: { workspaceId, active:
/// true }, with: { conditions } })`).
pub trait ConditionSource: Send + Sync {
    fn conditions<'a>(
        &'a self,
        workspace_id: &'a str,
    ) -> BoxFuture<'a, Result<Conditions, CacheError>>;
}

/// "Does an active trigger / webhook listen for one of these types?"
/// (`hasActiveTriggers` / `hasActiveWebhooks`).
pub trait ActiveConditions: Send + Sync {
    fn has_active<'a>(
        &'a self,
        workspace_id: &'a str,
        event_types: &'a [&'a str],
        source_id: Option<&'a str>,
    ) -> BoxFuture<'a, Result<bool, CacheError>>;
}

/// The owner table of the conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionOwner {
    /// `"Trigger"` (`Condition.triggerId`).
    Trigger,
    /// `"Webhook"` (`Condition.webhookId`).
    Webhook,
}

/// The SQLite [`ConditionSource`].
pub struct SqlConditions {
    db: Arc<Db>,
    owner: ConditionOwner,
}

impl SqlConditions {
    pub fn new(db: Arc<Db>, owner: ConditionOwner) -> SqlConditions {
        SqlConditions { db, owner }
    }
}

impl ConditionSource for SqlConditions {
    fn conditions<'a>(
        &'a self,
        workspace_id: &'a str,
    ) -> BoxFuture<'a, Result<Conditions, CacheError>> {
        let _ = (&self.db, self.owner, workspace_id);
        todo!("P3.6-X1")
    }
}

/// The type -> source ids map of one workspace.
pub type ConditionMap = HashMap<String, Vec<String>>;

/// `BaseCache`: the RAM layer over a [`ConditionSource`].
pub struct ActiveCache {
    source: Arc<dyn ConditionSource>,
    ram_ttl: Duration,
    ram: Mutex<HashMap<String, (ConditionMap, Instant)>>,
}

/// `ramTTL` of both caches (5 000 ms).
pub const RAM_TTL: Duration = Duration::from_millis(5_000);

impl ActiveCache {
    pub fn new(source: Arc<dyn ConditionSource>, ram_ttl: Duration) -> ActiveCache {
        ActiveCache {
            source,
            ram_ttl,
            ram: Mutex::new(HashMap::new()),
        }
    }

    /// The trigger cache on `db`.
    pub fn triggers(db: Arc<Db>) -> ActiveCache {
        ActiveCache::new(
            Arc::new(SqlConditions::new(db, ConditionOwner::Trigger)),
            RAM_TTL,
        )
    }

    /// The webhook cache on `db`.
    pub fn webhooks(db: Arc<Db>) -> ActiveCache {
        ActiveCache::new(
            Arc::new(SqlConditions::new(db, ConditionOwner::Webhook)),
            RAM_TTL,
        )
    }

    /// `hasActive`: the cached map (or a fresh one; an empty map is not
    /// cached) checked with `checkEventTypes`.
    pub async fn has_active(
        &self,
        workspace_id: &str,
        event_types: &[&str],
        source_id: Option<&str>,
    ) -> Result<bool, CacheError> {
        let _ = (&self.source, self.ram_ttl, &self.ram);
        let _ = (workspace_id, event_types, source_id);
        todo!("P3.6-X1")
    }

    /// `updateCache`: rebuilds the map of `workspace_id` (an empty map
    /// removes it).
    pub async fn update_cache(&self, workspace_id: &str) -> Result<(), CacheError> {
        let _ = workspace_id;
        todo!("P3.6-X1")
    }

    /// `removeCache`.
    pub fn remove_cache(&self, workspace_id: &str) {
        let _ = workspace_id;
        todo!("P3.6-X1")
    }
}

impl ActiveConditions for ActiveCache {
    fn has_active<'a>(
        &'a self,
        workspace_id: &'a str,
        event_types: &'a [&'a str],
        source_id: Option<&'a str>,
    ) -> BoxFuture<'a, Result<bool, CacheError>> {
        Box::pin(ActiveCache::has_active(
            self,
            workspace_id,
            event_types,
            source_id,
        ))
    }
}
