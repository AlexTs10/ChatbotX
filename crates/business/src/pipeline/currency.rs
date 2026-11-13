//! `pipeline/currency.ts`, `currency-rules.ts`, `currency-refresh.ts`.

use chatbotx_db::Db;
use chatbotx_db::repo::models::Pipeline;

use super::events::PipelineItemChanged;
use crate::errors::Result;

/// `PIPELINE_FALLBACK_CURRENCY`.
pub const PIPELINE_FALLBACK_CURRENCY: &str = "USD";

/// `PIPELINE_PAGE_SIZE` of the currency refresh.
pub const PIPELINE_PAGE_SIZE: i64 = 200;

/// `WorkspaceCurrencyDefault`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceCurrencyDefault {
    pub currency: String,
    pub country: Option<String>,
}

/// The `currency` / `currencyOverride` pair of `WithEffectiveCurrency`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveCurrency {
    pub currency: String,
    pub currency_override: Option<String>,
}

/// `PipelineWithCurrency`.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineView {
    pub pipeline: Pipeline,
    pub currency: String,
    pub currency_override: Option<String>,
}

/// `workspaceCurrencyDefault`.
pub fn workspace_currency_default(target_country: Option<&str>) -> WorkspaceCurrencyDefault {
    let _ = target_country;
    todo!("P3.6-D1p")
}

/// `applyEffectiveCurrency`.
pub fn apply_effective_currency(
    stored: Option<&str>,
    workspace_currency: &str,
) -> EffectiveCurrency {
    let _ = (stored, workspace_currency);
    todo!("P3.6-D1p")
}

/// `resolveWorkspaceDefault` (a missing workspace has no country).
pub async fn resolve_workspace_default(
    db: &Db,
    workspace_id: i64,
) -> Result<WorkspaceCurrencyDefault> {
    let _ = (db, workspace_id);
    todo!("P3.6-D1p")
}

/// `resolveWorkspaceCurrency`.
pub async fn resolve_workspace_currency(db: &Db, workspace_id: i64) -> Result<String> {
    let _ = (db, workspace_id);
    todo!("P3.6-D1p")
}

/// `withEffectiveCurrency`: an override never reads the workspace.
pub async fn with_effective_currency(
    db: &Db,
    workspace_id: i64,
    stored: Option<&str>,
) -> Result<EffectiveCurrency> {
    let _ = (db, workspace_id, stored);
    todo!("P3.6-D1p")
}

/// `withEffectiveCurrencies`: reads the workspace at most once.
pub async fn with_effective_currencies(
    db: &Db,
    workspace_id: i64,
    stored: &[Option<String>],
) -> Result<Vec<EffectiveCurrency>> {
    let _ = (db, workspace_id, stored);
    todo!("P3.6-D1p")
}

/// [`PipelineView`] of one row.
pub async fn view_of(db: &Db, workspace_id: i64, pipeline: Pipeline) -> Result<PipelineView> {
    let _ = (db, workspace_id, pipeline);
    todo!("P3.6-D1p")
}

/// `notifyWorkspaceCurrencyChange`: one `updated` event per pipeline of the
/// workspace, paged by id (`PIPELINE_PAGE_SIZE`). A failed read is logged and
/// ends the walk (the events so far are returned).
pub async fn notify_workspace_currency_change(
    db: &Db,
    workspace_id: i64,
) -> Vec<PipelineItemChanged> {
    let _ = (db, workspace_id);
    todo!("P3.6-D1p")
}
