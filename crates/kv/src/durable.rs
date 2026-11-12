//! Durable KV on the SQLite table `kv_entry` (spec 2.8, 2.9): the durable
//! list of spec 1.6. Reads use a reader connection and treat expired rows as
//! absent; every write is one `db.write` transaction (atomic like the Lua
//! scripts; the single writer serializes concurrent callers). `now` comes
//! from the [`Clock`] before the call.
//!
//! The table comes from migration `crates/db/migrations/0005_kv.sql`.
//! A logical error (wrong type, not an integer, bad JSON) is always found
//! before the transaction writes anything.

use std::collections::HashMap;
use std::sync::Arc;

use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use chatbotx_db::{Db, DbError, WriteTx};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::BoxFuture;
use crate::cas::{CasStore, cjson_matches};
use crate::clock::Clock;
use crate::error::{KvError, NotIntegerWhat};
use crate::mem::{expiry_ms, expiry_secs, reserve, reserve_span};
use crate::num::{js_number_string, lua_toint, lua_toint_text, lua_tonumber, redis_int};
use crate::sweeper::SweepSignal;

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

/// `kind` column values.
const KIND_STRING: i64 = 0;
const KIND_HASH: i64 = 1;
const KIND_LIST: i64 = 2;

/// Rows deleted per sweeper transaction (spec 2.9).
pub(crate) const SWEEP_BATCH: usize = 500;

const SELECT_LIVE: &str = r#"SELECT "kind", "value", "expiresAtMs" FROM "kv_entry"
 WHERE "key" = ?1 AND ("expiresAtMs" IS NULL OR "expiresAtMs" > ?2)"#;
const UPSERT: &str = r#"INSERT INTO "kv_entry" ("key", "kind", "value", "expiresAtMs")
 VALUES (?1, ?2, ?3, ?4)
 ON CONFLICT ("key") DO UPDATE SET "kind" = excluded."kind", "value" = excluded."value",
 "expiresAtMs" = excluded."expiresAtMs""#;
const DELETE: &str = r#"DELETE FROM "kv_entry" WHERE "key" = ?1"#;
const SET_EXPIRY: &str = r#"UPDATE "kv_entry" SET "expiresAtMs" = ?2 WHERE "key" = ?1"#;
const SWEEP: &str = r#"DELETE FROM "kv_entry" WHERE "key" IN
 (SELECT "key" FROM "kv_entry" WHERE "expiresAtMs" <= ?1 LIMIT ?2)"#;
const NEXT_EXPIRY: &str =
    r#"SELECT MIN("expiresAtMs") FROM "kv_entry" WHERE "expiresAtMs" IS NOT NULL"#;
const SCAN_FROM: &str = r#"SELECT "key" FROM "kv_entry"
 WHERE "key" >= ?1 AND ("expiresAtMs" IS NULL OR "expiresAtMs" > ?2) ORDER BY "key""#;

const NOT_AN_INTEGER: &str = "value is not an integer or out of range";
const HASH_NOT_AN_INTEGER: &str = "hash value is not an integer";
const OVERFLOW: &str = "increment or decrement would overflow";

/// One live row.
struct Row {
    kind: i64,
    value: String,
    expires_at: Option<i64>,
}

fn read_live(conn: &Connection, key: &str, now: i64) -> Result<Option<Row>, DbError> {
    conn.prepare_cached(SELECT_LIVE)?
        .query_row(params![key, now], |r| {
            Ok(Row {
                kind: r.get(0)?,
                value: r.get(1)?,
                expires_at: r.get(2)?,
            })
        })
        .optional()
        .map_err(DbError::from)
}

fn upsert(
    conn: &Connection,
    key: &str,
    kind: i64,
    value: &str,
    expires_at: Option<i64>,
) -> Result<(), DbError> {
    conn.prepare_cached(UPSERT)?
        .execute(params![key, kind, value, expires_at])?;
    Ok(())
}

fn delete_key(conn: &Connection, key: &str) -> Result<(), DbError> {
    conn.prepare_cached(DELETE)?.execute(params![key])?;
    Ok(())
}

fn ensure_kind(row: &Row, kind: i64, key: &str) -> Result<(), KvError> {
    if row.kind == kind {
        Ok(())
    } else {
        Err(KvError::WrongType {
            key: key.to_owned(),
        })
    }
}

/// The string value of a live row (`GET`); a hash or list is `WRONGTYPE`.
fn string_value(row: Option<Row>, key: &str) -> Result<Option<String>, KvError> {
    match row {
        None => Ok(None),
        Some(row) => {
            ensure_kind(&row, KIND_STRING, key)?;
            Ok(Some(row.value))
        }
    }
}

/// The fields of a live hash row (empty when absent).
fn load_hash(row: Option<&Row>, key: &str) -> Result<HashMap<String, String>, KvError> {
    match row {
        None => Ok(HashMap::new()),
        Some(row) => {
            ensure_kind(row, KIND_HASH, key)?;
            Ok(serde_json::from_str(&row.value)?)
        }
    }
}

/// Writes the hash (an empty hash is deleted, as Redis does).
fn store_hash(
    conn: &Connection,
    key: &str,
    fields: &HashMap<String, String>,
    expires_at: Option<i64>,
) -> Result<(), KvError> {
    if fields.is_empty() {
        delete_key(conn, key)?;
    } else {
        upsert(
            conn,
            key,
            KIND_HASH,
            &serde_json::to_string(fields)?,
            expires_at,
        )?;
    }
    Ok(())
}

fn invalid(key: &str, reason: &'static str) -> KvError {
    KvError::InvalidValue {
        key: key.to_owned(),
        reason,
    }
}

#[derive(Clone)]
pub struct DurableKv {
    db: Arc<Db>,
    clock: Arc<dyn Clock>,
    signal: Arc<SweepSignal>,
}

impl DurableKv {
    pub fn new(db: Arc<Db>, clock: Arc<dyn Clock>) -> DurableKv {
        DurableKv {
            db,
            clock,
            signal: SweepSignal::new(),
        }
    }

    pub(crate) fn signal(&self) -> &Arc<SweepSignal> {
        &self.signal
    }

    /// Runs `f` on a reader connection with `now`.
    async fn read<R, F>(&self, f: F) -> Result<R, KvError>
    where
        F: FnOnce(&Connection, i64) -> Result<R, KvError> + Send + 'static,
        R: Send + 'static,
    {
        let now = self.clock.now_ms();
        self.db.read(move |conn| Ok(f(conn, now))).await?
    }

    /// Runs `f` in one write transaction with `now`. `f` returns its result
    /// and the expiry it stored (to wake the sweeper). A database error rolls
    /// back; a logical error is found before any write.
    async fn write<R, F>(&self, f: F) -> Result<R, KvError>
    where
        F: FnOnce(&mut WriteTx<'_>, i64) -> Result<(R, Option<i64>), KvError> + Send + 'static,
        R: Send + 'static,
    {
        let now = self.clock.now_ms();
        let (out, expires_at) = self
            .db
            .write(move |tx| match f(tx, now) {
                Ok(done) => Ok(Ok(done)),
                Err(KvError::Db(err)) => Err(err),
                Err(err) => Ok(Err(err)),
            })
            .await??;
        if let Some(at) = expires_at {
            self.signal.note_expiry(at);
        }
        Ok(out)
    }

    async fn read_string(&self, key: &str) -> Result<Option<String>, KvError> {
        let key = key.to_owned();
        self.read(move |conn, now| string_value(read_live(conn, &key, now)?, &key))
            .await
    }

    async fn read_hash(&self, key: &str) -> Result<HashMap<String, String>, KvError> {
        let key = key.to_owned();
        self.read(move |conn, now| load_hash(read_live(conn, &key, now)?.as_ref(), &key))
            .await
    }

    // ---- strings / JSON (spec 2.9) ----

    pub async fn put<T: Serialize + Sync + ?Sized>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: Option<u64>,
    ) -> Result<(), KvError> {
        let text = serde_json::to_string(value)?;
        let key = key.to_owned();
        self.write(move |tx, now| {
            let expires_at = expiry_secs(now, ttl_secs);
            upsert(tx, &key, KIND_STRING, &text, expires_at)?;
            Ok(((), expires_at))
        })
        .await
    }

    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, KvError> {
        Ok(self
            .read_string(key)
            .await?
            .and_then(|text| serde_json::from_str(&text).ok()))
    }

    pub async fn take<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, KvError> {
        let key = key.to_owned();
        let text = self
            .write(move |tx, now| {
                let text = string_value(read_live(tx, &key, now)?, &key)?;
                if text.is_some() {
                    delete_key(tx, &key)?;
                }
                Ok((text, None))
            })
            .await?;
        Ok(text.and_then(|text| serde_json::from_str(&text).ok()))
    }

    pub async fn exists(&self, key: &str) -> Result<bool, KvError> {
        let key = key.to_owned();
        self.read(move |conn, now| Ok(read_live(conn, &key, now)?.is_some()))
            .await
    }

    pub async fn delete(&self, keys: &[&str]) -> Result<(), KvError> {
        if keys.is_empty() {
            return Ok(());
        }
        let keys: Vec<String> = keys.iter().map(|k| (*k).to_owned()).collect();
        self.write(move |tx, _| {
            for key in &keys {
                delete_key(tx, key)?;
            }
            Ok(((), None))
        })
        .await
    }

    /// K8 `MGET` (a key of another kind reads as `None`, as `MGET` does).
    pub async fn get_all<T: DeserializeOwned>(
        &self,
        keys: &[&str],
    ) -> Result<Vec<Option<T>>, KvError> {
        let keys: Vec<String> = keys.iter().map(|k| (*k).to_owned()).collect();
        let texts = self
            .read(move |conn, now| {
                keys.iter()
                    .map(|key| -> Result<Option<String>, KvError> {
                        Ok(read_live(conn, key, now)?
                            .filter(|row| row.kind == KIND_STRING)
                            .map(|row| row.value))
                    })
                    .collect::<Result<Vec<_>, KvError>>()
            })
            .await?;
        Ok(texts
            .into_iter()
            .map(|text| text.and_then(|text| serde_json::from_str(&text).ok()))
            .collect())
    }

    pub async fn set_number(&self, key: &str, value: f64, ttl_secs: u64) -> Result<(), KvError> {
        let key = key.to_owned();
        let text = js_number_string(value);
        self.write(move |tx, now| {
            let expires_at = expiry_secs(now, Some(ttl_secs));
            upsert(tx, &key, KIND_STRING, &text, expires_at)?;
            Ok(((), expires_at))
        })
        .await
    }

    pub async fn set_number_if_not_exists(
        &self,
        key: &str,
        value: f64,
        ttl_secs: u64,
    ) -> Result<bool, KvError> {
        let key = key.to_owned();
        let text = js_number_string(value);
        self.write(move |tx, now| {
            if read_live(tx, &key, now)?.is_some() {
                return Ok((false, None));
            }
            let expires_at = expiry_secs(now, Some(ttl_secs));
            upsert(tx, &key, KIND_STRING, &text, expires_at)?;
            Ok((true, expires_at))
        })
        .await
    }

    pub async fn increment_counter(
        &self,
        key: &str,
        delta: i64,
        ttl_secs: Option<u64>,
    ) -> Result<Option<i64>, KvError> {
        if delta == 0 {
            return Ok(None);
        }
        let key = key.to_owned();
        self.write(move |tx, now| {
            let Some(row) = read_live(tx, &key, now)? else {
                return Ok((None, None));
            };
            ensure_kind(&row, KIND_STRING, &key)?;
            let current = redis_int(&row.value).ok_or_else(|| invalid(&key, NOT_AN_INTEGER))?;
            let next = current
                .checked_add(delta)
                .ok_or_else(|| invalid(&key, OVERFLOW))?;
            let expires_at = expiry_secs(now, ttl_secs).or(row.expires_at);
            upsert(tx, &key, KIND_STRING, &next.to_string(), expires_at)?;
            Ok((Some(next), expires_at))
        })
        .await
    }

    /// `EXPIRE key ttl`; `false` when absent; `ttl` 0 deletes the key.
    pub async fn expire(&self, key: &str, ttl_secs: u64) -> Result<bool, KvError> {
        let key = key.to_owned();
        self.write(move |tx, now| {
            if read_live(tx, &key, now)?.is_none() {
                return Ok((false, None));
            }
            match expiry_secs(now, Some(ttl_secs)) {
                None => {
                    delete_key(tx, &key)?;
                    Ok((true, None))
                }
                Some(at) => {
                    tx.prepare_cached(SET_EXPIRY)
                        .map_err(DbError::from)?
                        .execute(params![key, at])
                        .map_err(DbError::from)?;
                    Ok((true, Some(at)))
                }
            }
        })
        .await
    }

    /// K7 comment pacing; same rule as [`crate::MemKv::reserve_time_window`].
    pub async fn reserve_time_window(&self, key: &str, span_ms: f64) -> Result<i64, KvError> {
        let key = key.to_owned();
        let span = reserve_span(span_ms);
        self.write(move |tx, now| {
            let reserved = string_value(read_live(tx, &key, now)?, &key)?
                .and_then(|text| lua_tonumber(&text))
                .unwrap_or(0.0);
            let (start, finish) = reserve(now, reserved, span);
            let expires_at = finish.saturating_add(60_000);
            upsert(tx, &key, KIND_STRING, &finish.to_string(), Some(expires_at))?;
            Ok((start, Some(expires_at)))
        })
        .await
    }

    /// K44 revoke marker: writes only when `value > existing`; returns the
    /// existing value or 0.
    pub async fn raise_number(&self, key: &str, value: i64, ttl_secs: u64) -> Result<i64, KvError> {
        let key = key.to_owned();
        self.write(move |tx, now| {
            let existing =
                string_value(read_live(tx, &key, now)?, &key)?.and_then(|text| lua_tonumber(&text));
            let mut stored = None;
            if existing.is_none_or(|existing| (value as f64) > existing) {
                stored = expiry_secs(now, Some(ttl_secs));
                upsert(tx, &key, KIND_STRING, &value.to_string(), stored)?;
            }
            Ok((existing.map_or(0, |existing| existing as i64), stored))
        })
        .await
    }

    /// K17: read-modify-write of one JSON value in one transaction.
    pub async fn update_json<R, F>(&self, key: &str, f: F) -> Result<R, KvError>
    where
        F: FnOnce(Option<Value>, i64) -> (JsonUpdate, R) + Send + 'static,
        R: Send + 'static,
    {
        let key = key.to_owned();
        self.write(move |tx, now| {
            let row = read_live(tx, &key, now)?;
            let kept = row.as_ref().and_then(|row| row.expires_at);
            let current = match string_value(row, &key)? {
                None => None,
                Some(text) => Some(serde_json::from_str::<Value>(&text)?),
            };
            let (update, out) = f(current, now);
            let stored = match update {
                JsonUpdate::Keep => None,
                JsonUpdate::Write(value) => {
                    upsert(tx, &key, KIND_STRING, &value.to_string(), kept)?;
                    kept
                }
                JsonUpdate::WriteTtl(value, ttl_ms) => {
                    let expires_at = expiry_ms(now, ttl_ms);
                    upsert(tx, &key, KIND_STRING, &value.to_string(), expires_at)?;
                    expires_at
                }
                JsonUpdate::Delete => {
                    delete_key(tx, &key)?;
                    None
                }
            };
            Ok((out, stored))
        })
        .await
    }

    // ---- lists (K16 simpleQueue) ----

    /// `RPUSH` + `EXPIRE` on every push.
    pub async fn push<T: Serialize + Sync + ?Sized>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: u64,
    ) -> Result<(), KvError> {
        let item = serde_json::to_string(value)?;
        let key = key.to_owned();
        self.write(move |tx, now| {
            let mut items: Vec<String> = match read_live(tx, &key, now)? {
                None => Vec::new(),
                Some(row) => {
                    ensure_kind(&row, KIND_LIST, &key)?;
                    serde_json::from_str(&row.value)?
                }
            };
            items.push(item);
            let text = serde_json::to_string(&items)?;
            match expiry_secs(now, Some(ttl_secs)) {
                // `EXPIRE key 0` deletes the key.
                None => {
                    delete_key(tx, &key)?;
                    Ok(((), None))
                }
                Some(at) => {
                    upsert(tx, &key, KIND_LIST, &text, Some(at))?;
                    Ok(((), Some(at)))
                }
            }
        })
        .await
    }

    pub async fn range<T: DeserializeOwned>(&self, key: &str) -> Result<Vec<T>, KvError> {
        let owned = key.to_owned();
        let items: Vec<String> = self
            .read(move |conn, now| match read_live(conn, &owned, now)? {
                None => Ok(Vec::new()),
                Some(row) => {
                    ensure_kind(&row, KIND_LIST, &owned)?;
                    Ok(serde_json::from_str(&row.value)?)
                }
            })
            .await?;
        items
            .iter()
            .map(|item| serde_json::from_str(item).map_err(KvError::from))
            .collect()
    }

    pub async fn clear(&self, key: &str) -> Result<(), KvError> {
        self.del(key).await
    }

    // ---- hashes (K15 merge; spec 2.8 live counters) ----

    /// `HGETALL`; empty map when the key is absent.
    pub async fn get_all_fields(&self, key: &str) -> Result<HashMap<String, String>, KvError> {
        self.read_hash(key).await
    }

    /// `merge`: `None` = field untouched; `Some(v)` written as its JSON text
    /// (`false` -> `"false"`, `Null` -> `"null"`). `ttl_secs` sets `EXPIRE`.
    pub async fn merge(
        &self,
        key: &str,
        fields: &[(&str, Option<Value>)],
        ttl_secs: Option<u64>,
    ) -> Result<(), KvError> {
        let pairs: Vec<(String, String)> = fields
            .iter()
            .filter_map(|(field, value)| {
                value
                    .as_ref()
                    .map(|value| ((*field).to_owned(), value.to_string()))
            })
            .collect();
        let key = key.to_owned();
        self.write(move |tx, now| {
            let row = read_live(tx, &key, now)?;
            let mut map = load_hash(row.as_ref(), &key)?;
            let kept = row.as_ref().and_then(|row| row.expires_at);
            let exists = row.is_some() || !pairs.is_empty();
            if !exists {
                return Ok(((), None));
            }
            map.extend(pairs);
            let stored = expiry_secs(now, ttl_secs).or(kept);
            store_hash(tx, &key, &map, stored)?;
            Ok(((), stored))
        })
        .await
    }

    pub async fn hget(&self, key: &str, field: &str) -> Result<Option<String>, KvError> {
        Ok(self.read_hash(key).await?.remove(field))
    }

    pub async fn hmget(&self, key: &str, fields: &[&str]) -> Result<Vec<Option<String>>, KvError> {
        let map = self.read_hash(key).await?;
        Ok(fields
            .iter()
            .map(|field| map.get(*field).cloned())
            .collect())
    }

    pub async fn hsetnx(&self, key: &str, field: &str, value: &str) -> Result<bool, KvError> {
        let (key, field, value) = (key.to_owned(), field.to_owned(), value.to_owned());
        self.write(move |tx, now| {
            let row = read_live(tx, &key, now)?;
            let mut map = load_hash(row.as_ref(), &key)?;
            if map.contains_key(&field) {
                return Ok((false, None));
            }
            map.insert(field, value);
            store_hash(tx, &key, &map, row.and_then(|row| row.expires_at))?;
            Ok((true, None))
        })
        .await
    }

    pub async fn hset(&self, key: &str, fields: &[(&str, &str)]) -> Result<(), KvError> {
        if fields.is_empty() {
            return Ok(());
        }
        let pairs: Vec<(String, String)> = fields
            .iter()
            .map(|(f, v)| ((*f).to_owned(), (*v).to_owned()))
            .collect();
        let key = key.to_owned();
        self.write(move |tx, now| {
            let row = read_live(tx, &key, now)?;
            let mut map = load_hash(row.as_ref(), &key)?;
            map.extend(pairs);
            store_hash(tx, &key, &map, row.and_then(|row| row.expires_at))?;
            Ok(((), None))
        })
        .await
    }

    /// Non-integer field -> error (as Redis).
    pub async fn hincrby(&self, key: &str, field: &str, delta: i64) -> Result<i64, KvError> {
        let (key, field) = (key.to_owned(), field.to_owned());
        self.write(move |tx, now| {
            let row = read_live(tx, &key, now)?;
            let mut map = load_hash(row.as_ref(), &key)?;
            let current = match map.get(&field) {
                None => 0,
                Some(text) => redis_int(text).ok_or_else(|| invalid(&key, HASH_NOT_AN_INTEGER))?,
            };
            let next = current
                .checked_add(delta)
                .ok_or_else(|| invalid(&key, OVERFLOW))?;
            map.insert(field, next.to_string());
            store_hash(tx, &key, &map, row.and_then(|row| row.expires_at))?;
            Ok((next, None))
        })
        .await
    }

    pub async fn del(&self, key: &str) -> Result<(), KvError> {
        self.delete(&[key]).await
    }

    /// `admitWithinLimit` (spec 2.8). `limit: None` = unlimited.
    pub async fn admit_within_limit(
        &self,
        key: &str,
        field: &str,
        limit: Option<f64>,
    ) -> Result<Admit, KvError> {
        let (key, field) = (key.to_owned(), field.to_owned());
        // `String(limit ?? -1)`.
        let limit = limit.unwrap_or(-1.0);
        self.write(move |tx, now| {
            let row = read_live(tx, &key, now)?;
            let mut map = load_hash(row.as_ref(), &key)?;
            let Some(text) = map.get(&field) else {
                return Ok((
                    Admit {
                        status: AdmitStatus::Missing,
                        value: 0,
                    },
                    None,
                ));
            };
            let current = lua_toint_text(text).ok_or(KvError::NotInteger {
                what: NotIntegerWhat::Field,
            })?;
            let limit = lua_toint(limit).ok_or(KvError::NotInteger {
                what: NotIntegerWhat::Argument,
            })?;
            let normalized = current.max(0.0);
            if limit >= 0.0 && normalized + 1.0 > limit {
                let admit = Admit {
                    status: AdmitStatus::Refused,
                    value: normalized as i64,
                };
                return Ok((admit, None));
            }
            let next = normalized + 1.0;
            map.insert(field, js_number_string(next));
            store_hash(tx, &key, &map, row.and_then(|row| row.expires_at))?;
            let admit = Admit {
                status: AdmitStatus::Admitted,
                value: next as i64,
            };
            Ok((admit, None))
        })
        .await
    }

    /// `decrementFloor` (spec 2.8): missing field -> `None`, no write.
    pub async fn decrement_floor(
        &self,
        key: &str,
        field: &str,
        count: f64,
    ) -> Result<Option<i64>, KvError> {
        let (key, field) = (key.to_owned(), field.to_owned());
        self.write(move |tx, now| {
            let row = read_live(tx, &key, now)?;
            let mut map = load_hash(row.as_ref(), &key)?;
            let text = map.get(&field);
            let current = text.and_then(|text| lua_toint_text(text));
            if text.is_some() && current.is_none() {
                return Err(KvError::NotInteger {
                    what: NotIntegerWhat::Field,
                });
            }
            let count = lua_toint(count).ok_or(KvError::NotInteger {
                what: NotIntegerWhat::Argument,
            })?;
            let Some(current) = current else {
                return Ok((None, None));
            };
            let next = (current - count).max(0.0);
            map.insert(field, js_number_string(next));
            store_hash(tx, &key, &map, row.and_then(|row| row.expires_at))?;
            Ok((Some(next as i64), None))
        })
        .await
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
        let (key, period_field, period, field) = (
            key.to_owned(),
            period_field.to_owned(),
            period.to_owned(),
            field.to_owned(),
        );
        self.write(move |tx, now| {
            let row = read_live(tx, &key, now)?;
            let mut map = load_hash(row.as_ref(), &key)?;
            if map.get(&period_field) != Some(&period) {
                return Ok((0, None));
            }
            let current = map.get(&field).and_then(|text| lua_tonumber(text));
            if current.is_some_and(|current| (value as f64) <= current) {
                return Ok((0, None));
            }
            map.insert(field, value.to_string());
            store_hash(tx, &key, &map, row.and_then(|row| row.expires_at))?;
            Ok((1, None))
        })
        .await
    }

    /// K28: live keys with this prefix (primary-key range scan).
    pub async fn scan_prefix(&self, prefix: &str) -> Result<Vec<String>, KvError> {
        let prefix = prefix.to_owned();
        self.read(move |conn, now| {
            let mut stmt = conn.prepare_cached(SCAN_FROM).map_err(DbError::from)?;
            let mut rows = stmt.query(params![prefix, now]).map_err(DbError::from)?;
            let mut keys = Vec::new();
            while let Some(row) = rows.next().map_err(DbError::from)? {
                let key: String = row.get(0).map_err(DbError::from)?;
                if !key.starts_with(&prefix) {
                    break;
                }
                keys.push(key);
            }
            Ok(keys)
        })
        .await
    }

    // ---- in-transaction forms (spec 2.9) ----

    /// [`DurableKv::put`] inside a caller's write transaction. (The sweeper
    /// learns of the expiry at its next planned wake-up.)
    pub fn put_in<T: Serialize + ?Sized>(
        tx: &mut WriteTx<'_>,
        now_ms: i64,
        key: &str,
        value: &T,
        ttl_secs: Option<u64>,
    ) -> Result<(), KvError> {
        let text = serde_json::to_string(value)?;
        upsert(tx, key, KIND_STRING, &text, expiry_secs(now_ms, ttl_secs))?;
        Ok(())
    }

    /// Deletes up to 500 expired rows; returns how many. Called by the sweeper.
    pub async fn sweep_expired(&self) -> Result<usize, KvError> {
        self.write(move |tx, now| {
            let limit = i64::try_from(SWEEP_BATCH).unwrap_or(i64::MAX);
            let deleted = tx
                .prepare_cached(SWEEP)
                .map_err(DbError::from)?
                .execute(params![now, limit])
                .map_err(DbError::from)?;
            Ok((deleted, None))
        })
        .await
    }

    /// The earliest stored expiry (for the sweeper).
    pub(crate) async fn next_expiry(&self) -> Result<Option<i64>, KvError> {
        self.read(|conn, _| {
            conn.query_row(NEXT_EXPIRY, [], |r| r.get::<_, Option<i64>>(0))
                .map_err(|e| KvError::Db(e.into()))
        })
        .await
    }
}

/// Test-only inspection helpers (spec 3.2).
#[cfg(any(test, feature = "test-util"))]
impl DurableKv {
    /// Remaining TTL in ms; `None` when absent or no expiry.
    pub async fn pttl_ms(&self, key: &str) -> Result<Option<i64>, KvError> {
        let key = key.to_owned();
        self.read(move |conn, now| {
            Ok(read_live(conn, &key, now)?
                .and_then(|row| row.expires_at)
                .map(|at| at - now))
        })
        .await
    }

    /// Writes `raw` text as a string value (for malformed-JSON cases).
    pub async fn put_raw(&self, key: &str, raw: &str, ttl_ms: Option<u64>) -> Result<(), KvError> {
        let (key, raw) = (key.to_owned(), raw.to_owned());
        self.write(move |tx, now| {
            let expires_at = ttl_ms.and_then(|ttl| expiry_ms(now, ttl));
            upsert(tx, &key, KIND_STRING, &raw, expires_at)?;
            Ok(((), expires_at))
        })
        .await
    }
}

impl DurableKv {
    async fn cas_set_if_absent(
        &self,
        key: &str,
        value: &Value,
        ttl_ms: u64,
    ) -> Result<bool, KvError> {
        let (key, text) = (key.to_owned(), value.to_string());
        self.write(move |tx, now| {
            if read_live(tx, &key, now)?.is_some() {
                return Ok((false, None));
            }
            let expires_at = expiry_ms(now, ttl_ms);
            upsert(tx, &key, KIND_STRING, &text, expires_at)?;
            Ok((true, expires_at))
        })
        .await
    }

    async fn cas_swap(
        &self,
        key: &str,
        expected: Option<Map<String, Value>>,
        next: &Value,
        ttl_ms: u64,
    ) -> Result<bool, KvError> {
        let (key, text) = (key.to_owned(), next.to_string());
        self.write(move |tx, now| {
            let current = string_value(read_live(tx, &key, now)?, &key)?;
            let matches = match (&expected, &current) {
                (None, current) => current.is_none(),
                (Some(_), None) => false,
                (Some(expected), Some(current)) => cjson_matches(&key, current, expected)?,
            };
            if !matches {
                return Ok((false, None));
            }
            let expires_at = expiry_ms(now, ttl_ms);
            upsert(tx, &key, KIND_STRING, &text, expires_at)?;
            Ok((true, expires_at))
        })
        .await
    }

    async fn cas_delete(&self, key: &str, expected: Map<String, Value>) -> Result<bool, KvError> {
        let key = key.to_owned();
        self.write(move |tx, now| {
            let Some(current) = string_value(read_live(tx, &key, now)?, &key)? else {
                return Ok((false, None));
            };
            if !cjson_matches(&key, &current, &expected)? {
                return Ok((false, None));
            }
            delete_key(tx, &key)?;
            Ok((true, None))
        })
        .await
    }
}

impl CasStore for DurableKv {
    fn set_if_absent<'a>(
        &'a self,
        key: &'a str,
        value: &'a Value,
        ttl_ms: u64,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        Box::pin(self.cas_set_if_absent(key, value, ttl_ms))
    }

    fn get_raw<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>, KvError>> {
        Box::pin(self.read_string(key))
    }

    fn get_json<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<Value>, KvError>> {
        Box::pin(async move {
            Ok(self
                .read_string(key)
                .await?
                .and_then(|text| serde_json::from_str(&text).ok()))
        })
    }

    fn del<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<(), KvError>> {
        Box::pin(DurableKv::del(self, key))
    }

    fn compare_and_swap<'a>(
        &'a self,
        key: &'a str,
        expected: Option<&'a Map<String, Value>>,
        next: &'a Value,
        ttl_ms: u64,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        Box::pin(self.cas_swap(key, expected.cloned(), next, ttl_ms))
    }

    fn compare_and_delete<'a>(
        &'a self,
        key: &'a str,
        expected: &'a Map<String, Value>,
    ) -> BoxFuture<'a, Result<bool, KvError>> {
        Box::pin(self.cas_delete(key, expected.clone()))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::clock::TestClock;

    async fn store() -> (DurableKv, Arc<TestClock>, Arc<Db>) {
        let db = Arc::new(Db::open_in_memory().unwrap());
        db.migrate().await.unwrap();
        let clock = Arc::new(TestClock::new(1_000_000));
        (DurableKv::new(db.clone(), clock.clone()), clock, db)
    }

    async fn stored_rows(db: &Db) -> i64 {
        db.read(|conn| {
            conn.query_row(r#"SELECT count(*) FROM "kv_entry""#, [], |r| r.get(0))
                .map_err(DbError::from)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn reads_filter_expired_rows_that_the_sweeper_has_not_removed() {
        let (kv, clock, db) = store().await;
        kv.put("s", &json!("v"), Some(1)).await.unwrap();
        kv.hset("h", &[("f", "1")]).await.unwrap();
        kv.expire("h", 1).await.unwrap();
        kv.push("l", &1, 1).await.unwrap();
        clock.advance(1_000);
        assert_eq!(stored_rows(&db).await, 3, "rows are still on disk");
        assert_eq!(kv.get::<Value>("s").await.unwrap(), None);
        assert!(!kv.exists("s").await.unwrap());
        assert!(kv.get_all_fields("h").await.unwrap().is_empty());
        assert!(kv.range::<i64>("l").await.unwrap().is_empty());
        assert_eq!(kv.scan_prefix("").await.unwrap(), Vec::<String>::new());
        // An expired key is absent for writes too: NX writes succeed.
        assert!(kv.set_number_if_not_exists("s", 1.0, 60).await.unwrap());
        assert_eq!(kv.hincrby("h", "f", 5).await.unwrap(), 5);
        assert_eq!(kv.increment_counter("l", 1, None).await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_sweeper_deletes_expired_rows_in_batches_of_500() {
        let (kv, clock, db) = store().await;
        for i in 0..1_200 {
            kv.put(&format!("k{i:04}"), &i, Some(1)).await.unwrap();
        }
        kv.put("forever", &1, None).await.unwrap();
        assert_eq!(kv.next_expiry().await.unwrap(), Some(1_001_000));
        clock.advance(1_000);
        assert_eq!(kv.sweep_expired().await.unwrap(), 500);
        assert_eq!(kv.sweep_expired().await.unwrap(), 500);
        assert_eq!(kv.sweep_expired().await.unwrap(), 200);
        assert_eq!(kv.sweep_expired().await.unwrap(), 0);
        assert_eq!(stored_rows(&db).await, 1);
        assert_eq!(kv.next_expiry().await.unwrap(), None);
    }

    #[tokio::test]
    async fn kinds_do_not_mix_and_errors_write_nothing() {
        let (kv, _clock, _db) = store().await;
        kv.hset("h", &[("n", "x")]).await.unwrap();
        assert!(matches!(
            kv.get::<Value>("h").await,
            Err(KvError::WrongType { .. })
        ));
        assert!(matches!(
            kv.push("h", &1, 60).await,
            Err(KvError::WrongType { .. })
        ));
        assert!(matches!(
            kv.hincrby("h", "n", 1).await,
            Err(KvError::InvalidValue { .. })
        ));
        assert_eq!(kv.hget("h", "n").await.unwrap().as_deref(), Some("x"));
        kv.put_raw("s", "abc", None).await.unwrap();
        assert!(matches!(
            kv.increment_counter("s", 1, None).await,
            Err(KvError::InvalidValue { .. })
        ));
        assert!(matches!(
            kv.hget("s", "f").await,
            Err(KvError::WrongType { .. })
        ));
        // MGET reads another kind as None.
        assert_eq!(
            kv.get_all::<Value>(&["h", "s", "missing"]).await.unwrap(),
            [None, None, None]
        );
    }

    #[tokio::test]
    async fn merge_raise_and_update_json_follow_the_scripts() {
        let (kv, clock, _db) = store().await;
        // merge with only untouched fields on a missing key creates nothing.
        kv.merge("ctx", &[("a", None)], Some(60)).await.unwrap();
        assert!(!kv.exists("ctx").await.unwrap());
        kv.merge("ctx", &[("a", Some(json!("x")))], Some(60))
            .await
            .unwrap();
        assert_eq!(kv.hget("ctx", "a").await.unwrap().as_deref(), Some("\"x\""));
        assert_eq!(kv.pttl_ms("ctx").await.unwrap(), Some(60_000));

        // raise_number: only upward; returns the existing value.
        assert_eq!(kv.raise_number("rv", 5, 65).await.unwrap(), 0);
        assert_eq!(kv.raise_number("rv", 3, 65).await.unwrap(), 5);
        assert_eq!(kv.raise_number("rv", 9, 65).await.unwrap(), 5);
        assert_eq!(kv.get::<i64>("rv").await.unwrap(), Some(9));

        // raise_live_mac: period must match; only a larger value is written.
        kv.hset("q", &[("p", "2026-11"), ("mac", "4")])
            .await
            .unwrap();
        assert_eq!(
            kv.raise_live_mac("q", "p", "2026-10", "mac", 9)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            kv.raise_live_mac("q", "p", "2026-11", "mac", 3)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            kv.raise_live_mac("q", "p", "2026-11", "mac", 9)
                .await
                .unwrap(),
            1
        );
        assert_eq!(kv.hget("q", "mac").await.unwrap().as_deref(), Some("9"));

        // update_json: Write keeps the TTL (KEEPTTL); WriteTtl replaces it.
        kv.put("o", &json!({"state": "pending"}), Some(100))
            .await
            .unwrap();
        clock.advance(40_000);
        let seen = kv
            .update_json("o", |current, _now| {
                (JsonUpdate::Write(json!({"state": "writing"})), current)
            })
            .await
            .unwrap();
        assert_eq!(seen, Some(json!({"state": "pending"})));
        assert_eq!(kv.pttl_ms("o").await.unwrap(), Some(60_000));
        kv.update_json("o", |_, _| (JsonUpdate::WriteTtl(json!(1), 5_000), ()))
            .await
            .unwrap();
        assert_eq!(kv.pttl_ms("o").await.unwrap(), Some(5_000));
        kv.update_json("o", |_, _| (JsonUpdate::Delete, ()))
            .await
            .unwrap();
        assert!(!kv.exists("o").await.unwrap());

        // scan_prefix: ordered, prefix only.
        kv.put("user-quota-live:2", &1, None).await.unwrap();
        kv.put("user-quota-live:1", &1, None).await.unwrap();
        kv.put("user-quota-livf", &1, None).await.unwrap();
        assert_eq!(
            kv.scan_prefix("user-quota-live:").await.unwrap(),
            ["user-quota-live:1", "user-quota-live:2"]
        );
    }
}
