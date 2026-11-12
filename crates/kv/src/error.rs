//! Store errors (spec 2.13).

use std::fmt;

use chatbotx_db::DbError;

/// Which value of a live-counter script is not an integer (spec 2.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotIntegerWhat {
    /// The stored hash field: `ERR live counter field is not an integer`.
    Field,
    /// The caller's argument (limit / count): `ERR live counter argument is not an integer`.
    Argument,
}

#[derive(Debug)]
pub enum KvError {
    /// Display text is the exact TS / Lua message (spec 3.2).
    NotInteger {
        what: NotIntegerWhat,
    },
    /// The key holds another kind of value (string vs hash vs list), or a
    /// CAS record is a JSON scalar that a Lua script cannot index.
    WrongType {
        key: String,
    },
    /// A Redis command error on a stored value (`INCRBY` / `HINCRBY` on a
    /// value that is not an integer, or an overflow). `reason` is the Redis
    /// text without the `ERR ` prefix.
    InvalidValue {
        key: String,
        reason: &'static str,
    },
    Json(serde_json::Error),
    Db(DbError),
}

impl fmt::Display for KvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KvError::NotInteger {
                what: NotIntegerWhat::Field,
            } => f.write_str("ERR live counter field is not an integer"),
            KvError::NotInteger {
                what: NotIntegerWhat::Argument,
            } => f.write_str("ERR live counter argument is not an integer"),
            KvError::WrongType { key } => write!(
                f,
                "WRONGTYPE Operation against a key holding the wrong kind of value (key {key:?})"
            ),
            KvError::InvalidValue { key, reason } => write!(f, "ERR {reason} (key {key:?})"),
            KvError::Json(err) => write!(f, "kv JSON error: {err}"),
            KvError::Db(err) => write!(f, "kv store: {err}"),
        }
    }
}

impl std::error::Error for KvError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            KvError::Json(err) => Some(err),
            KvError::Db(err) => Some(err),
            _ => None,
        }
    }
}

impl From<DbError> for KvError {
    fn from(err: DbError) -> KvError {
        KvError::Db(err)
    }
}

impl From<serde_json::Error> for KvError {
    fn from(err: serde_json::Error) -> KvError {
        KvError::Json(err)
    }
}
