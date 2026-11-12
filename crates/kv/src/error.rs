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
    /// The key holds another kind of value (string vs hash vs list).
    WrongType {
        key: String,
    },
    Json(serde_json::Error),
    Db(DbError),
}

impl fmt::Display for KvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!("P2.7 spec 2.13: exact TS messages")
    }
}

impl std::error::Error for KvError {}

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
