//! CAS semantics of `casStore` (spec 2.10; K11, K13, K14). Implemented by
//! [`crate::MemKv`] and [`crate::DurableKv`]; dyn-compatible so callers and
//! tests can inject either.
//!
//! Match rule (Lua `cjson`): numbers compare by value (`1 == 1.0`), strings by
//! bytes, booleans, `null == null`; a missing field never matches (also not
//! `null`); an object or array in `expected` never matches.

use serde_json::{Map, Value};

use crate::BoxFuture;
use crate::error::KvError;

pub trait CasStore: Send + Sync {
    /// `SET key json PX ttl_ms NX`; an expired key is absent; a second write
    /// can neither overwrite nor extend. `true` when written.
    fn set_if_absent<'a>(
        &'a self,
        key: &'a str,
        value: &'a Value,
        ttl_ms: u64,
    ) -> BoxFuture<'a, Result<bool, KvError>>;

    /// The raw stored text; `None` when absent.
    fn get_raw<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>, KvError>>;

    /// Parsed JSON; `None` when absent or not valid JSON.
    fn get_json<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Value>, KvError>>;

    fn del<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), KvError>>;

    /// `expected: None` = the key must be absent (create-only); `Some(e)` = the
    /// key must exist and every field of `e` must match. On success writes
    /// `next` with a new TTL `ttl_ms`.
    fn compare_and_swap<'a>(
        &'a self,
        key: &'a str,
        expected: Option<&'a Map<String, Value>>,
        next: &'a Value,
        ttl_ms: u64,
    ) -> BoxFuture<'a, Result<bool, KvError>>;

    /// Same match as [`CasStore::compare_and_swap`]; deletes on success.
    fn compare_and_delete<'a>(
        &'a self,
        key: &'a str,
        expected: &'a Map<String, Value>,
    ) -> BoxFuture<'a, Result<bool, KvError>>;
}
