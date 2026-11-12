//! Durable KV on the SQLite table `kv_entry` (spec 2.8, 2.9): the durable
//! list of spec 1.6. Reads use a reader connection and treat expired rows as
//! absent; every write is one `db.write` transaction (atomic like the Lua
//! scripts; the single writer serializes concurrent callers). `now` comes
//! from the [`Clock`] before the call.
//!
//! The table comes from migration `crates/db/migrations/0005_kv.sql`.

use std::collections::HashMap;
use std::sync::Arc;

use chatbotx_db::{Db, WriteTx};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::BoxFuture;
use crate::cas::CasStore;
use crate::clock::Clock;
use crate::error::KvError;

/// Result status of `admitWithinLimit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitStatus {
    Admitted,
    Refused,
    Missing,
}

/// `{ status, value }` of `admitWithinLimit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admit {
    pub status: AdmitStatus,
    pub value: i64,
}

/// Decision of an [`DurableKv::update_json`] closure (K17 heavy-step scripts).
#[derive(Debug, Clone, PartialEq)]
pub enum JsonUpdate {
    Keep,
    /// Write, keep the current TTL (`KEEPTTL`).
    Write(Value),
    /// Write with a new TTL in ms.
    WriteTtl(Value, u64),
    Delete,
}

#[derive(Clone)]
pub struct DurableKv {
    _db: Arc<Db>,
    _clock: Arc<dyn Clock>,
}

impl DurableKv {
    pub fn new(db: Arc<Db>, clock: Arc<dyn Clock>) -> DurableKv {
        let _ = (db, clock);
        todo!("P2.7 spec 2.9")
    }

    // ---- strings / JSON (spec 2.9) ----

    pub async fn put<T: Serialize + Sync + ?Sized>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: Option<u64>,
    ) -> Result<(), KvError> {
        let _ = (key, value, ttl_secs);
        todo!("P2.7 spec 2.9")
    }

    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, KvError> {
        let _ = key;
        todo!("P2.7 spec 2.9")
    }

    pub async fn take<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, KvError> {
        let _ = key;
        todo!("P2.7 spec 2.9")
    }

    pub async fn exists(&self, key: &str) -> Result<bool, KvError> {
        let _ = key;
        todo!("P2.7 spec 2.9")
    }

    pub async fn delete(&self, keys: &[&str]) -> Result<(), KvError> {
        let _ = keys;
        todo!("P2.7 spec 2.9")
    }

    /// K8 `MGET`.
    pub async fn get_all<T: DeserializeOwned>(
        &self,
        keys: &[&str],
    ) -> Result<Vec<Option<T>>, KvError> {
        let _ = keys;
        todo!("P2.7 spec 2.9")
    }

    pub async fn set_number(&self, key: &str, value: f64, ttl_secs: u64) -> Result<(), KvError> {
        let _ = (key, value, ttl_secs);
        todo!("P2.7 spec 2.9")
    }

    pub async fn set_number_if_not_exists(
        &self,
        key: &str,
        value: f64,
        ttl_secs: u64,
    ) -> Result<bool, KvError> {
        let _ = (key, value, ttl_secs);
        todo!("P2.7 spec 2.9")
    }

    pub async fn increment_counter(
        &self,
        key: &str,
        delta: i64,
        ttl_secs: Option<u64>,
    ) -> Result<Option<i64>, KvError> {
        let _ = (key, delta, ttl_secs);
        todo!("P2.7 spec 2.9")
    }

    pub async fn expire(&self, key: &str, ttl_secs: u64) -> Result<bool, KvError> {
        let _ = (key, ttl_secs);
        todo!("P2.7 spec 2.9")
    }

    /// K7 comment pacing; same rule as [`crate::MemKv::reserve_time_window`].
    pub async fn reserve_time_window(&self, key: &str, span_ms: f64) -> Result<i64, KvError> {
        let _ = (key, span_ms);
        todo!("P2.7 spec 2.9")
    }

    /// K44 revoke marker: writes only when `value > existing`; returns the
    /// existing value or 0.
    pub async fn raise_number(&self, key: &str, value: i64, ttl_secs: u64) -> Result<i64, KvError> {
        let _ = (key, value, ttl_secs);
        todo!("P2.7 spec 2.9")
    }

    /// K17: read-modify-write of one JSON value in one transaction.
    pub async fn update_json<R, F>(&self, key: &str, f: F) -> Result<R, KvError>
    where
        F: FnOnce(Option<Value>, i64) -> (JsonUpdate, R) + Send + 'static,
        R: Send + 'static,
    {
        let _ = (key, f);
        todo!("P2.7 spec 2.9")
    }

    // ---- lists (K16 simpleQueue) ----

    /// `RPUSH` + `EXPIRE` on every push.
    pub async fn push<T: Serialize + Sync + ?Sized>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: u64,
    ) -> Result<(), KvError> {
        let _ = (key, value, ttl_secs);
        todo!("P2.7 spec 2.9")
    }

    pub async fn range<T: DeserializeOwned>(&self, key: &str) -> Result<Vec<T>, KvError> {
        let _ = key;
        todo!("P2.7 spec 2.9")
    }

    pub async fn clear(&self, key: &str) -> Result<(), KvError> {
        let _ = key;
        todo!("P2.7 spec 2.9")
    }

    // ---- hashes (K15 merge; spec 2.8 live counters) ----

    /// `HGETALL`; empty map when the key is absent.
    pub async fn get_all_fields(&self, key: &str) -> Result<HashMap<String, String>, KvError> {
        let _ = key;
        todo!("P2.7 spec 2.9")
    }

    /// `merge`: `None` = field untouched; `Some(v)` written as its JSON text
    /// (`false` -> `"false"`, `Null` -> `"null"`). `ttl_secs` sets `EXPIRE`.
    pub async fn merge(
        &self,
        key: &str,
        fields: &[(&str, Option<Value>)],
        ttl_secs: Option<u64>,
    ) -> Result<(), KvError> {
        let _ = (key, fields, ttl_secs);
        todo!("P2.7 spec 2.9")
    }

    pub async fn hget(&self, key: &str, field: &str) -> Result<Option<String>, KvError> {
        let _ = (key, field);
        todo!("P2.7 spec 2.8")
    }

    pub async fn hmget(&self, key: &str, fields: &[&str]) -> Result<Vec<Option<String>>, KvError> {
        let _ = (key, fields);
        todo!("P2.7 spec 2.8")
    }

    pub async fn hsetnx(&self, key: &str, field: &str, value: &str) -> Result<bool, KvError> {
        let _ = (key, field, value);
        todo!("P2.7 spec 2.8")
    }

    pub async fn hset(&self, key: &str, fields: &[(&str, &str)]) -> Result<(), KvError> {
        let _ = (key, fields);
        todo!("P2.7 spec 2.8")
    }

    /// Non-integer field -> error (as Redis).
    pub async fn hincrby(&self, key: &str, field: &str, delta: i64) -> Result<i64, KvError> {
        let _ = (key, field, delta);
        todo!("P2.7 spec 2.8")
    }

    pub async fn del(&self, key: &str) -> Result<(), KvError> {
        let _ = key;
        todo!("P2.7 spec 2.8")
    }

    /// `admitWithinLimit` (spec 2.8). `limit: None` = unlimited.
    pub async fn admit_within_limit(
        &self,
        key: &str,
        field: &str,
        limit: Option<f64>,
    ) -> Result<Admit, KvError> {
        let _ = (key, field, limit);
        todo!("P2.7 spec 2.8")
    }

    /// `decrementFloor` (spec 2.8): missing field -> `None`, no write.
    pub async fn decrement_floor(
        &self,
        key: &str,
        field: &str,
        count: f64,
    ) -> Result<Option<i64>, KvError> {
        let _ = (key, field, count);
        todo!("P2.7 spec 2.8")
    }

    /// `RAISE_LIVE_MAC_LUA`.
    pub async fn raise_live_mac(
        &self,
        key: &str,
        period_field: &str,
        period: &str,
        field: &str,
        value: i64,
    ) -> Result<i64, KvError> {
        let _ = (key, period_field, period, field, value);
        todo!("P2.7 spec 2.8")
    }

    /// K28: live keys with this prefix (primary-key range scan).
    pub async fn scan_prefix(&self, prefix: &str) -> Result<Vec<String>, KvError> {
        let _ = prefix;
        todo!("P2.7 spec 2.8")
    }

    // ---- in-transaction forms (spec 2.9) ----

    /// [`DurableKv::put`] inside a caller's write transaction.
    pub fn put_in<T: Serialize + ?Sized>(
        tx: &mut WriteTx<'_>,
        now_ms: i64,
        key: &str,
        value: &T,
        ttl_secs: Option<u64>,
    ) -> Result<(), KvError> {
        let _ = (tx, now_ms, key, value, ttl_secs);
        todo!("P2.7 spec 2.9")
    }

    /// Deletes up to 500 expired rows; returns how many. Called by the sweeper.
    pub async fn sweep_expired(&self) -> Result<usize, KvError> {
        todo!("P2.7 spec 2.9")
    }
}

/// Test-only inspection helpers (spec 3.2).
#[cfg(any(test, feature = "test-util"))]
impl DurableKv {
    /// Remaining TTL in ms; `None` when absent or no expiry.
    pub async fn pttl_ms(&self, key: &str) -> Result<Option<i64>, KvError> {
        let _ = key;
        todo!("P2.7 spec 3.2")
    }

    /// Writes `raw` text as a string value (for malformed-JSON cases).
    pub async fn put_raw(&self, key: &str, raw: &str, ttl_ms: Option<u64>) -> Result<(), KvError> {
        let _ = (key, raw, ttl_ms);
        todo!("P2.7 spec 3.2")
    }
}

impl CasStore for DurableKv {
    fn set_if_absent<'a>(
        &'a self,
        key: &'a str,
        value: &'a Value,
        ttl_ms: u64,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        let _ = (key, value, ttl_ms);
        todo!("P2.7 spec 2.10")
    }

    fn get_raw<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>, KvError>> {
        let _ = key;
        todo!("P2.7 spec 2.10")
    }

    fn get_json<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Value>, KvError>> {
        let _ = key;
        todo!("P2.7 spec 2.10")
    }

    fn del<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), KvError>> {
        let _ = key;
        todo!("P2.7 spec 2.10")
    }

    fn compare_and_swap<'a>(
        &'a self,
        key: &'a str,
        expected: Option<&'a Map<String, Value>>,
        next: &'a Value,
        ttl_ms: u64,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        let _ = (key, expected, next, ttl_ms);
        todo!("P2.7 spec 2.10")
    }

    fn compare_and_delete<'a>(
        &'a self,
        key: &'a str,
        expected: &'a Map<String, Value>,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        let _ = (key, expected);
        todo!("P2.7 spec 2.10")
    }
}
