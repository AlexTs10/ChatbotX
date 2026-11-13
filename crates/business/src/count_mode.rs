//! `packages/business/src/count-mode.ts`: an exact count (under the
//! `EXACT_COUNT_STATEMENT_TIMEOUT`) or a bounded count (stops at
//! `BOUNDED_COUNT_CAP`). The count runs in one write transaction, the only
//! place where the SQLite port installs a statement timeout.

use chatbotx_db::repo::client::CappedCount;
use chatbotx_db::{StatementTimeout, WriteTx};

use chatbotx_db::repo;

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `BOUNDED_COUNT_CAP`.
pub const BOUNDED_COUNT_CAP: i64 = 100_000;
/// `EXACT_COUNT_STATEMENT_TIMEOUT`.
pub const EXACT_COUNT_STATEMENT_TIMEOUT: &str = "30s";

/// `CountMode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CountMode {
    #[default]
    Exact,
    Bounded,
}

impl CountMode {
    /// `countModes` (`exact` / `bounded`).
    pub fn parse(text: &str) -> Option<CountMode> {
        match text {
            "exact" => Some(CountMode::Exact),
            "bounded" => Some(CountMode::Bounded),
            _ => None,
        }
    }
}

/// `CountOutcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountOutcome {
    pub total: i64,
    pub total_count_capped: bool,
    /// `countCap`: set for a bounded count only.
    pub count_cap: Option<i64>,
}

/// One exact count.
pub type ExactCount = Box<dyn FnOnce(&WriteTx<'_>) -> Result<i64> + Send>;
/// One bounded count (`cap`).
pub type BoundedCount = Box<dyn FnOnce(i64, &WriteTx<'_>) -> Result<CappedCount> + Send>;

/// `countByMode` with the `EXACT_COUNT_STATEMENT_TIMEOUT`.
pub async fn count_by_mode(
    db: &chatbotx_db::Db,
    mode: Option<CountMode>,
    exact: ExactCount,
    bounded: BoundedCount,
) -> Result<CountOutcome> {
    let timeout = StatementTimeout::parse(EXACT_COUNT_STATEMENT_TIMEOUT)
        .map_err(|e| BusinessError::Repo(repo::Error::msg(format!("{e:?}"))))?;
    count_by_mode_with_timeout(db, mode, timeout, exact, bounded).await
}

/// `countByMode` under a given statement timeout: a timed-out statement is
/// the `503 countTimeout` exception; every other error is returned as is.
pub async fn count_by_mode_with_timeout(
    db: &chatbotx_db::Db,
    mode: Option<CountMode>,
    timeout: StatementTimeout,
    exact: ExactCount,
    bounded: BoundedCount,
) -> Result<CountOutcome> {
    let mode = mode.unwrap_or_default();
    let outcome = tx::write(db, move |tx| {
        tx.set_statement_timeout(timeout)?;
        match mode {
            CountMode::Exact => Ok(CountOutcome {
                total: exact(tx)?,
                total_count_capped: false,
                count_cap: None,
            }),
            CountMode::Bounded => {
                let counted = bounded(BOUNDED_COUNT_CAP, tx)?;
                Ok(CountOutcome {
                    total: counted.total,
                    total_count_capped: counted.capped,
                    count_cap: Some(BOUNDED_COUNT_CAP),
                })
            }
        }
    })
    .await;
    outcome.map_err(|error| {
        if is_statement_timeout(&error) {
            count_timeout_exception()
        } else {
            error
        }
    })
}

/// `isStatementTimeoutError`: SQLSTATE `57014`.
fn is_statement_timeout(error: &BusinessError) -> bool {
    matches!(error, BusinessError::Repo(repo::Error::Db(db)) if db.sqlstate() == Some("57014"))
}

/// `countTimeoutException`.
pub fn count_timeout_exception() -> BusinessError {
    BusinessError::exception(
        "Counting took too long. Narrow the filter, or use countMode \"bounded\" for a capped count.",
        "countTimeout",
        503,
    )
}
