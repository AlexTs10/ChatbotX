//! `pipeline/variable-targets.ts`: what the `{{pipelines.<ref>.<attr>}}` and
//! `{{last_opportunity_id}}` variables read for a contact. System context (no
//! access scope), one query. `value` is the stored x 10,000 integer; dates
//! are unix ms.

use chatbotx_db::Db;
use chatbotx_db::repo::partials::pipeline::parse_pipeline_settings;
use chatbotx_db::repo::pipeline_variable_targets as target_repo;

use super::currency::{apply_effective_currency, workspace_currency_default};
use crate::errors::Result;
use crate::tx;

/// `PipelineVariableTarget`: the item a variable reads, one per pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineVariableTarget {
    pub id: i64,
    /// The pipeline's `referenceName`.
    pub reference_name: String,
    /// The item's title.
    pub name: String,
    pub stage_name: String,
    pub status: String,
    /// The pipeline's own label for `status` (e.g. "Closed won").
    pub status_label: String,
    pub priority: String,
    pub value: Option<i64>,
    pub currency: String,
    pub assigned_admins: Vec<String>,
    pub created_at: i64,
    pub due_date: Option<i64>,
    pub updated_at: i64,
}

/// `loadPipelineVariableTargets`: per pipeline the contact's latest open
/// item, else its latest item. Stored settings that do not parse give the
/// default status labels (`pipelineSettingsSchema.parse` threw).
pub async fn load_pipeline_variable_targets(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<PipelineVariableTarget>> {
    let rows = tx::read(db, move |conn| {
        Ok(target_repo::list_for_contact(
            conn,
            workspace_id,
            contact_id,
        )?)
    })
    .await?;
    // The target country comes back with the rows; every row is the same workspace.
    let workspace_currency =
        workspace_currency_default(rows.first().and_then(|row| row.target_country.as_deref()))
            .currency;
    Ok(rows
        .into_iter()
        .map(|row| {
            let labels = parse_pipeline_settings(&row.settings.0)
                .unwrap_or_default()
                .status_labels;
            let status_label = match row.status.as_str() {
                "won" => labels.won,
                "lost" => labels.lost,
                _ => labels.open,
            };
            PipelineVariableTarget {
                id: row.item_id,
                currency: apply_effective_currency(row.currency.as_deref(), &workspace_currency)
                    .currency,
                reference_name: row.reference_name,
                name: row.title,
                stage_name: row.stage_name,
                status: row.status,
                status_label,
                priority: row.priority,
                value: row.value,
                assigned_admins: row
                    .assignees
                    .unwrap_or_default()
                    .into_iter()
                    .flatten()
                    .filter(|name| !name.is_empty())
                    .collect(),
                created_at: row.created_at,
                due_date: row.due_date,
                updated_at: row.updated_at,
            }
        })
        .collect())
}

/// `loadLastPipelineItemId` (`{{last_opportunity_id}}`): the contact's last
/// created or updated item, any pipeline.
pub async fn load_last_pipeline_item_id(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Option<i64>> {
    tx::read(db, move |conn| {
        Ok(target_repo::find_last_touched_item_id(
            conn,
            workspace_id,
            contact_id,
        )?)
    })
    .await
}
