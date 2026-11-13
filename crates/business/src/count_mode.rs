//! `packages/business/src/count-mode.ts`: an exact count (under the
//! `EXACT_COUNT_STATEMENT_TIMEOUT`) or a bounded count (stops at
//! `BOUNDED_COUNT_CAP`). The count runs in one write transaction, the only
//! place where the SQLite port installs a statement timeout.

use chatbotx_db::repo::client::CappedCount;
use chatbotx_db::{StatementTimeout, WriteTx};

use crate::errors::Result;

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
    let _ = (db, mode, exact, bounded);
    todo!("P3.6-D2")
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
    let _ = (db, mode, timeout, exact, bounded);
    todo!("P3.6-D2")
}
