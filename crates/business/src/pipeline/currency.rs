//! `pipeline/currency.ts`, `currency-rules.ts`, `currency-refresh.ts`.

use chatbotx_core::utils::country_currency::primary_currency_for_country;
use chatbotx_db::Db;
use chatbotx_db::repo::models::Pipeline;
use chatbotx_db::rusqlite::OptionalExtension;

use super::events::PipelineItemChanged;
use crate::errors::Result;
use crate::tx;

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
    let country = target_country.map(|c| c.trim().to_uppercase());
    match (
        country.as_deref(),
        primary_currency_for_country(country.as_deref()),
    ) {
        (Some(country), Some(currency)) => WorkspaceCurrencyDefault {
            currency: currency.to_string(),
            country: Some(country.to_string()),
        },
        _ => WorkspaceCurrencyDefault {
            currency: PIPELINE_FALLBACK_CURRENCY.to_string(),
            country: None,
        },
    }
}

/// `applyEffectiveCurrency`.
pub fn apply_effective_currency(
    stored: Option<&str>,
    workspace_currency: &str,
) -> EffectiveCurrency {
    EffectiveCurrency {
        currency: stored.unwrap_or(workspace_currency).to_string(),
        currency_override: stored.map(str::to_string),
    }
}

/// `resolveWorkspaceDefault` (a missing workspace has no country).
pub async fn resolve_workspace_default(
    db: &Db,
    workspace_id: i64,
) -> Result<WorkspaceCurrencyDefault> {
    let country = tx::read(db, move |conn| {
        let row: Option<Option<String>> = conn
            .query_row(
                r#"SELECT "targetCountry" FROM "Workspace" WHERE "id" = ?1"#,
                [workspace_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(chatbotx_db::repo::Error::from)?;
        Ok(row.flatten())
    })
    .await?;
    Ok(workspace_currency_default(country.as_deref()))
}

/// `resolveWorkspaceCurrency`.
pub async fn resolve_workspace_currency(db: &Db, workspace_id: i64) -> Result<String> {
    Ok(resolve_workspace_default(db, workspace_id).await?.currency)
}

/// `withEffectiveCurrency`: an override never reads the workspace.
pub async fn with_effective_currency(
    db: &Db,
    workspace_id: i64,
    stored: Option<&str>,
) -> Result<EffectiveCurrency> {
    match stored {
        Some(currency) => Ok(apply_effective_currency(
            Some(currency),
            PIPELINE_FALLBACK_CURRENCY,
        )),
        None => Ok(apply_effective_currency(
            None,
            &resolve_workspace_currency(db, workspace_id).await?,
        )),
    }
}

/// `withEffectiveCurrencies`: reads the workspace at most once.
pub async fn with_effective_currencies(
    db: &Db,
    workspace_id: i64,
    stored: &[Option<String>],
) -> Result<Vec<EffectiveCurrency>> {
    let workspace_currency = if stored.iter().any(Option::is_none) {
        resolve_workspace_currency(db, workspace_id).await?
    } else {
        PIPELINE_FALLBACK_CURRENCY.to_string()
    };
    Ok(stored
        .iter()
        .map(|currency| apply_effective_currency(currency.as_deref(), &workspace_currency))
        .collect())
}

/// [`PipelineView`] of one row.
pub async fn view_of(db: &Db, workspace_id: i64, pipeline: Pipeline) -> Result<PipelineView> {
    let effective = with_effective_currency(db, workspace_id, pipeline.currency.as_deref()).await?;
    Ok(PipelineView {
        pipeline,
        currency: effective.currency,
        currency_override: effective.currency_override,
    })
}

/// [`PipelineView`]s of many rows (one workspace read at most).
pub async fn views_of(
    db: &Db,
    workspace_id: i64,
    pipelines: Vec<Pipeline>,
) -> Result<Vec<PipelineView>> {
    let stored: Vec<Option<String>> = pipelines.iter().map(|p| p.currency.clone()).collect();
    let effective = with_effective_currencies(db, workspace_id, &stored).await?;
    Ok(pipelines
        .into_iter()
        .zip(effective)
        .map(|(pipeline, effective)| PipelineView {
            pipeline,
            currency: effective.currency,
            currency_override: effective.currency_override,
        })
        .collect())
}

/// `notifyWorkspaceCurrencyChange`: one `updated` event per pipeline of the
/// workspace, paged by id (`PIPELINE_PAGE_SIZE`). A failed read is logged and
/// ends the walk (the events so far are returned).
pub async fn notify_workspace_currency_change(
    db: &Db,
    workspace_id: i64,
) -> Vec<PipelineItemChanged> {
    let mut events = Vec::new();
    let mut after_id = None;
    loop {
        let page = tx::read(db, move |conn| {
            Ok(chatbotx_db::repo::pipeline::list_ids_page(
                conn,
                workspace_id,
                after_id,
                PIPELINE_PAGE_SIZE,
            )?)
        })
        .await;
        let ids = match page {
            Ok(ids) => ids,
            Err(error) => {
                tracing::error!(
                    workspace_id,
                    error = %error.message(),
                    "Failed to notify pipelines of a workspace currency change"
                );
                return events;
            }
        };
        events.extend(ids.iter().map(|&pipeline_id| PipelineItemChanged {
            workspace_id,
            pipeline_id,
            item_id: None,
            contact_id: None,
            change: "updated",
        }));
        if (ids.len() as i64) < PIPELINE_PAGE_SIZE {
            return events;
        }
        after_id = ids.last().copied();
    }
}
