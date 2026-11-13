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
        let sql = match self.owner {
            ConditionOwner::Trigger => {
                r#"SELECT c."type", c."sourceId" FROM "Condition" c JOIN "Trigger" t ON t."id" = c."triggerId" WHERE t."workspaceId" = ?1 AND t."active" = 1 ORDER BY t."id", c."id""#
            }
            ConditionOwner::Webhook => {
                r#"SELECT c."type", c."sourceId" FROM "Condition" c JOIN "Webhook" w ON w."id" = c."webhookId" WHERE w."workspaceId" = ?1 AND w."active" = 1 ORDER BY w."id", c."id""#
            }
        };
        let workspace = workspace_id.parse::<i64>();
        let db = Arc::clone(&self.db);
        Box::pin(async move {
            // A non-numeric id matches no row (the bigint column).
            let Ok(workspace) = workspace else {
                return Ok(Vec::new());
            };
            db.read(move |conn| {
                let mut statement = conn.prepare_cached(sql)?;
                let rows = statement
                    .query_map([workspace], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<Result<Vec<(String, Option<String>)>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(|err| err.to_string())
        })
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
        let map = match self.cached(workspace_id) {
            Some(map) => map,
            None => {
                let map = self.build(workspace_id).await?;
                if map.is_empty() {
                    return Ok(false);
                }
                self.store(workspace_id, map.clone());
                map
            }
        };
        Ok(check_event_types(&map, event_types, source_id))
    }

    /// `updateCache`: rebuilds the map of `workspace_id` (an empty map
    /// removes it).
    pub async fn update_cache(&self, workspace_id: &str) -> Result<(), CacheError> {
        let map = self.build(workspace_id).await?;
        if map.is_empty() {
            self.remove_cache(workspace_id);
        } else {
            self.store(workspace_id, map);
        }
        Ok(())
    }

    /// `removeCache`.
    pub fn remove_cache(&self, workspace_id: &str) {
        self.lock().remove(workspace_id);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (ConditionMap, Instant)>> {
        self.ram
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The RAM entry of `workspace_id` while it is younger than the TTL.
    fn cached(&self, workspace_id: &str) -> Option<ConditionMap> {
        let mut ram = self.lock();
        match ram.get(workspace_id) {
            Some((map, at)) if at.elapsed() < self.ram_ttl => Some(map.clone()),
            Some(_) => {
                ram.remove(workspace_id);
                None
            }
            None => None,
        }
    }

    fn store(&self, workspace_id: &str, map: ConditionMap) {
        self.lock()
            .insert(workspace_id.to_string(), (map, Instant::now()));
    }

    /// `buildCacheData`: type -> distinct source ids (`"*"` for none).
    async fn build(&self, workspace_id: &str) -> Result<ConditionMap, CacheError> {
        let mut map = ConditionMap::new();
        for (condition_type, source_id) in self.source.conditions(workspace_id).await? {
            let source = source_id
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| "*".to_string());
            let ids = map.entry(condition_type).or_default();
            if !ids.contains(&source) {
                ids.push(source);
            }
        }
        Ok(map)
    }
}

/// `checkEventTypes`: a listening type admits an evaluator-filtered event,
/// any-source (`"*"`) conditions, or a matching `sourceId`.
fn check_event_types(map: &ConditionMap, event_types: &[&str], source_id: Option<&str>) -> bool {
    event_types.iter().any(|event_type| {
        map.get(*event_type).is_some_and(|ids| {
            super::registry::is_evaluator_filtered(event_type)
                || ids.iter().any(|id| id == "*")
                || source_id
                    .is_some_and(|source| !source.is_empty() && ids.iter().any(|id| id == source))
        })
    })
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
