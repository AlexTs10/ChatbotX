//! `db.transaction(...)` / plain reads for services: like `repo::{write, read}`,
//! but a [`BusinessError`] raised inside the closure comes back unchanged (the
//! transaction rolls back).

use std::sync::{Arc, Mutex, PoisonError};

use chatbotx_db::rusqlite::Connection;
use chatbotx_db::{Db, WriteTx, repo};

use crate::errors::{BusinessError, Result};

type Slot = Arc<Mutex<Option<BusinessError>>>;

fn split(slot: &Slot, outcome: Result<(), BusinessError>) -> repo::Result<()> {
    match outcome {
        Ok(()) => Ok(()),
        Err(BusinessError::Repo(error)) => Err(error),
        Err(other) => {
            let message = other.message();
            *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(other);
            Err(repo::Error::msg(message))
        }
    }
}

fn join<R>(slot: &Slot, outcome: repo::Result<R>) -> Result<R> {
    match outcome {
        Ok(value) => Ok(value),
        Err(error) => match slot.lock().unwrap_or_else(PoisonError::into_inner).take() {
            Some(domain) => Err(domain),
            None => Err(BusinessError::Repo(error)),
        },
    }
}

/// One write transaction (`BEGIN IMMEDIATE`); any `Err` rolls back.
pub async fn write<F, R>(db: &Db, f: F) -> Result<R>
where
    F: FnOnce(&mut WriteTx<'_>) -> Result<R> + Send + 'static,
    R: Send + 'static,
{
    let slot: Slot = Arc::new(Mutex::new(None));
    let inner = Arc::clone(&slot);
    let outcome = repo::write(db, move |tx| {
        let mut value = None;
        split(&inner, f(tx).map(|v| value = Some(v)))?;
        value.ok_or_else(|| repo::Error::msg("transaction produced no value"))
    })
    .await;
    join(&slot, outcome)
}

/// One read on a reader connection (one snapshot).
pub async fn read<F, R>(db: &Db, f: F) -> Result<R>
where
    F: FnOnce(&Connection) -> Result<R> + Send + 'static,
    R: Send + 'static,
{
    let slot: Slot = Arc::new(Mutex::new(None));
    let inner = Arc::clone(&slot);
    let outcome = repo::read(db, move |conn| {
        let mut value = None;
        split(&inner, f(conn).map(|v| value = Some(v)))?;
        value.ok_or_else(|| repo::Error::msg("read produced no value"))
    })
    .await;
    join(&slot, outcome)
}
