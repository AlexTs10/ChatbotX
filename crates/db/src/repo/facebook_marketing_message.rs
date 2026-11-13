//! Port of `repositories/facebook-marketing-messages/repository.ts` (P3.2b): the
//! `facebookMarketingMessagesAuthRepository` (`auth_*`) and the campaign repository.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{FacebookMarketingMessage, FacebookMarketingMessagesAuth};
use super::patch::{self, Patch};
use super::sql::{Sql, opt_int, opt_text};
use super::utils::{
    PaginationInput, SortItem, get_pagination_with_defaults, like_contains, parse_order_by,
};
use super::{Error, Model, Result, id_list, now_ms};

/// `facebookMarketingMessagesAuthRepository.create` input (`status` is `active`).
#[derive(Debug, Clone, PartialEq)]
pub struct AuthCreateInput {
    pub id: i64,
    pub workspace_id: i64,
    pub auth: serde_json::Value,
    pub token_expires_at: Option<i64>,
    pub facebook_user_id: Option<String>,
}

/// `facebookMarketingMessagesAuthRepository.updateAuth` input.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthUpdateInput {
    pub id: i64,
    pub auth: serde_json::Value,
    pub token_expires_at: Option<i64>,
    pub facebook_user_id: Option<String>,
}

/// `MarketingMessageStatisticsListInput` (`sort` ids: `campaignId`, `name`).
#[derive(Debug, Clone, Default)]
pub struct StatisticsListInput {
    pub workspace_id: i64,
    pub ad_account_id: String,
    pub search: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// `listForStatistics` result.
#[derive(Debug, Clone, PartialEq)]
pub struct StatisticsPage {
    pub data: Vec<FacebookMarketingMessage>,
    pub total: i64,
    pub page_count: i64,
}

/// `MarketingMessageAdAccountSummary`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdAccountSummary {
    pub ad_account_id: String,
    pub currency: String,
    pub timezone_name: Option<String>,
}

/// `CreateFacebookMarketingMessageInput` (`status` defaults to the column default).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreateInput {
    pub id: i64,
    pub workspace_id: i64,
    pub name: String,
    pub page_id: String,
    pub ad_account_id: String,
    pub currency: String,
    pub currency_offset: i64,
    pub budget_type: String,
    pub budget_minor_units: i64,
    pub content: serde_json::Value,
    pub target_type: String,
    pub audience_id: Option<i64>,
    pub campaign_id: String,
    pub facebook_user_id: Option<String>,
    pub status: Option<String>,
    pub starts_at: Option<i64>,
}

/// `update` input: `content` is written only when given; `lock_content` guards it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateInput {
    pub id: i64,
    pub workspace_id: i64,
    pub name: String,
    pub budget_type: String,
    pub budget_minor_units: i64,
    pub content: Option<serde_json::Value>,
    pub lock_content: bool,
}

const AUTH: &str = "\"FacebookMarketingMessagesAuth\"";
const TABLE: &str = "\"FacebookMarketingMessage\"";

/// `STATISTICS_SORTABLE_COLUMNS`.
const STATISTICS_SORTABLE_COLUMNS: &[&str] = &["campaignId", "name"];

/// The `transition` patch may not set these columns.
const FROZEN: &[&str] = &["id", "workspaceId", "campaignId"];

fn returning<M: Model>(
    conn: &Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Option<M>> {
    Ok(super::returning::<M>(conn, sql, params)?.into_iter().next())
}

/// `facebookMarketingMessagesAuthRepository.findByWorkspaceId`.
pub fn auth_find_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Option<FacebookMarketingMessagesAuth>> {
    super::find_one(conn, "\"workspaceId\" = ?1", &[&workspace_id])
}

/// `facebookMarketingMessagesAuthRepository.create`.
pub fn auth_create(
    conn: &Connection,
    input: &AuthCreateInput,
) -> Result<FacebookMarketingMessagesAuth> {
    returning(
        conn,
        &format!(
            "INSERT INTO {AUTH} (\"id\", \"workspaceId\", \"auth\", \"tokenExpiresAt\", \
             \"facebookUserId\", \"status\") VALUES (?1, ?2, ?3, ?4, ?5, 'active')"
        ),
        &[
            &input.id,
            &input.workspace_id,
            &patch::json(&input.auth),
            &input.token_expires_at,
            &input.facebook_user_id,
        ],
    )?
    .ok_or_else(|| Error::msg("Failed to create FacebookMarketingMessagesAuth"))
}

/// `facebookMarketingMessagesAuthRepository.updateAuth`: replaces the token, status `active`.
pub fn auth_update_auth(
    conn: &Connection,
    input: &AuthUpdateInput,
) -> Result<Option<FacebookMarketingMessagesAuth>> {
    returning(
        conn,
        &format!(
            "UPDATE {AUTH} SET \"auth\" = ?1, \"tokenExpiresAt\" = ?2, \"facebookUserId\" = ?3, \
             \"status\" = 'active', \"updatedAt\" = ?4 WHERE \"id\" = ?5"
        ),
        &[
            &patch::json(&input.auth),
            &input.token_expires_at,
            &input.facebook_user_id,
            &now_ms(),
            &input.id,
        ],
    )
}

/// `facebookMarketingMessagesAuthRepository.updateStatus`.
pub fn auth_update_status(conn: &Connection, workspace_id: i64, status: &str) -> Result<()> {
    conn.execute(
        &format!(
            "UPDATE {AUTH} SET \"status\" = ?1, \"updatedAt\" = ?2 WHERE \"workspaceId\" = ?3"
        ),
        rusqlite::params![status, now_ms(), workspace_id],
    )?;
    Ok(())
}

/// `listByWorkspaceId` (createdAt DESC, id DESC).
pub fn list_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<FacebookMarketingMessage>> {
    super::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 ORDER BY \"createdAt\" DESC, \"id\" DESC",
        &[&workspace_id],
    )
}

/// `listForStatistics`.
pub fn list_for_statistics(
    conn: &Connection,
    input: &StatisticsListInput,
) -> Result<StatisticsPage> {
    let search = input
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let filter = Sql::and([
        Some(Sql::bind("\"workspaceId\" = ", input.workspace_id)),
        Some(Sql::bind("\"adAccountId\" = ", input.ad_account_id.clone())),
        search.map(|s| {
            Sql::or([
                Some(Sql::bind("\"campaignId\" = ", s.to_owned())),
                Some(Sql::bind("ilike(\"name\", ", like_contains(s)).closed(")")),
            ])
        }),
    ]);
    let pagination = get_pagination_with_defaults(PaginationInput {
        page: input.page,
        per_page: input.per_page,
    });
    let order = parse_order_by(
        FacebookMarketingMessage::COLUMNS,
        input.sort.as_deref(),
        Some(STATISTICS_SORTABLE_COLUMNS),
    );
    let mut order_sql: Vec<String> = order
        .iter()
        .map(|(c, d)| format!("\"{c}\" {}", d.sql()))
        .collect();
    if order_sql.is_empty() {
        order_sql.push("\"createdAt\" DESC".to_owned());
    }
    // Tiebreaker so equal names / timestamps cannot repeat or skip rows across pages.
    order_sql.push("\"id\" DESC".to_owned());
    let mut q = Sql::raw(format!(
        "SELECT {} FROM {TABLE} WHERE ",
        FacebookMarketingMessage::columns_sql(None)
    ));
    q.append(filter.clone());
    q.push(&format!(" ORDER BY {} LIMIT ", order_sql.join(", ")));
    q.push_bind(pagination.limit);
    q.push(" OFFSET ");
    q.push_bind(pagination.offset);
    let data = q.models(conn)?;
    let mut c = Sql::raw(format!("SELECT count(*) FROM {TABLE} WHERE "));
    c.append(filter);
    let total: i64 = c
        .query_map(conn, |r| r.get(0))?
        .into_iter()
        .next()
        .unwrap_or(0);
    let page_count = if pagination.limit > 0 {
        (total + pagination.limit - 1) / pagination.limit
    } else {
        0
    };
    Ok(StatisticsPage {
        data,
        total,
        page_count,
    })
}

/// `listAdAccountsByWorkspaceId`: one entry per ad account (from its newest campaign),
/// newest-used first.
pub fn list_ad_accounts_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<AdAccountSummary>> {
    Sql::bind(
        format!(
            "SELECT \"adAccountId\", \"currency\", \"timezoneName\" FROM (SELECT \"adAccountId\", \
             \"currency\", \"timezoneName\", \"createdAt\", ROW_NUMBER() OVER (PARTITION BY \
             \"adAccountId\" ORDER BY \"createdAt\" DESC) AS rn FROM {TABLE} WHERE \"workspaceId\" = "
        ),
        workspace_id,
    )
    .closed(") WHERE rn = 1 ORDER BY \"createdAt\" DESC")
    .query_map(conn, |r| {
        Ok(AdAccountSummary {
            ad_account_id: r.get(0)?,
            currency: r.get(1)?,
            timezone_name: r.get(2)?,
        })
    })
}

/// `findForWorkspace`.
pub fn find_for_workspace(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<FacebookMarketingMessage>> {
    super::find_one(
        conn,
        "\"id\" = ?1 AND \"workspaceId\" = ?2",
        &[&id, &workspace_id],
    )
}

/// `create`.
pub fn create(conn: &Connection, input: &CreateInput) -> Result<FacebookMarketingMessage> {
    let mut values: Patch = vec![
        ("id", Value::Integer(input.id)),
        ("workspaceId", Value::Integer(input.workspace_id)),
        ("name", Value::Text(input.name.clone())),
        ("pageId", Value::Text(input.page_id.clone())),
        ("adAccountId", Value::Text(input.ad_account_id.clone())),
        ("currency", Value::Text(input.currency.clone())),
        ("currencyOffset", Value::Integer(input.currency_offset)),
        ("budgetType", Value::Text(input.budget_type.clone())),
        ("budgetMinorUnits", Value::Integer(input.budget_minor_units)),
        ("content", patch::json(&input.content)),
        ("targetType", Value::Text(input.target_type.clone())),
        ("audienceId", opt_int(input.audience_id)),
        ("campaignId", Value::Text(input.campaign_id.clone())),
        (
            "facebookUserId",
            opt_text(input.facebook_user_id.as_deref()),
        ),
    ];
    if let Some(status) = &input.status {
        values.push(("status", Value::Text(status.clone())));
    }
    values.push(("startsAt", opt_int(input.starts_at)));
    let mut q = Sql::raw(format!("INSERT INTO {TABLE} "));
    q.append(patch::insert_clause(
        FacebookMarketingMessage::COLUMNS,
        &values,
    )?);
    q.push(&format!(
        " RETURNING {}",
        FacebookMarketingMessage::columns_sql(None)
    ));
    q.model(conn)?
        .ok_or_else(|| Error::msg("Failed to create FacebookMarketingMessage"))
}

/// `update`.
pub fn update(conn: &Connection, input: &UpdateInput) -> Result<Option<FacebookMarketingMessage>> {
    let mut set: Patch = vec![
        ("name", Value::Text(input.name.clone())),
        ("budgetType", Value::Text(input.budget_type.clone())),
        ("budgetMinorUnits", Value::Integer(input.budget_minor_units)),
    ];
    if let Some(content) = &input.content {
        set.push(("content", patch::json(content)));
    }
    set.push(("updatedAt", Value::Integer(now_ms())));
    let mut q = Sql::raw(format!("UPDATE {TABLE} SET "));
    q.append(patch::set_clause(FacebookMarketingMessage::COLUMNS, &set)?);
    q.push(" WHERE \"id\" = ");
    q.push_bind(input.id);
    q.push(" AND \"workspaceId\" = ");
    q.push_bind(input.workspace_id);
    if input.lock_content {
        q.push(" AND \"firstSentAt\" IS NULL AND \"status\" IN ('notSent', 'scheduled')");
    }
    q.push(&format!(
        " RETURNING {}",
        FacebookMarketingMessage::columns_sql(None)
    ));
    q.model(conn)
}

/// `countByAudienceId`.
pub fn count_by_audience_id(conn: &Connection, audience_id: i64, workspace_id: i64) -> Result<i64> {
    Ok(conn.query_row(
        &format!("SELECT count(*) FROM {TABLE} WHERE \"audienceId\" = ?1 AND \"workspaceId\" = ?2"),
        [audience_id, workspace_id],
        |r| r.get(0),
    )?)
}

/// `remove`.
pub fn remove(conn: &Connection, ids: &[i64], workspace_id: i64) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        &format!(
            "DELETE FROM {TABLE} WHERE \"id\" IN (SELECT value FROM json_each(?1)) \
             AND \"workspaceId\" = ?2"
        ),
        rusqlite::params![id_list(ids), workspace_id],
    )?;
    Ok(())
}

/// `listDueToStart`: scheduled and due (past any `nextDayAt` backoff), or waiting for the
/// next day.
pub fn list_due_to_start(conn: &Connection, now: i64) -> Result<Vec<FacebookMarketingMessage>> {
    super::find_many(
        conn,
        "WHERE (\"status\" = 'scheduled' AND \"startsAt\" <= ?1 AND (\"nextDayAt\" IS NULL \
         OR \"nextDayAt\" <= ?1)) OR (\"status\" = 'waitingNextDay' AND \"nextDayAt\" <= ?1) \
         ORDER BY \"id\"",
        &[&now],
    )
}

/// `listByStatus`.
pub fn list_by_status(conn: &Connection, status: &str) -> Result<Vec<FacebookMarketingMessage>> {
    super::find_many(conn, "WHERE \"status\" = ?1 ORDER BY \"id\"", &[&status])
}

/// `findById` (unscoped).
pub fn find_by_id(conn: &Connection, id: i64) -> Result<Option<FacebookMarketingMessage>> {
    super::find_one(conn, "\"id\" = ?1", &[&id])
}

/// `findByCampaignId` (unscoped).
pub fn find_by_campaign_id(
    conn: &Connection,
    campaign_id: &str,
) -> Result<Option<FacebookMarketingMessage>> {
    super::find_one(conn, "\"campaignId\" = ?1", &[&campaign_id])
}

/// `transition`: conditional update from one of `from`; `None` when another worker won.
pub fn transition(
    conn: &Connection,
    id: i64,
    from: &[&str],
    patch: &Patch,
) -> Result<Option<FacebookMarketingMessage>> {
    if from.is_empty() {
        return Ok(None);
    }
    if let Some((col, _)) = patch.iter().find(|(c, _)| FROZEN.contains(c)) {
        return Err(Error::msg(format!("transition may not set {col}")));
    }
    let mut set = patch.clone();
    if !set.iter().any(|(c, _)| *c == "updatedAt") {
        set.push(("updatedAt", Value::Integer(now_ms())));
    }
    let mut q = Sql::raw(format!("UPDATE {TABLE} SET "));
    q.append(patch::set_clause(FacebookMarketingMessage::COLUMNS, &set)?);
    q.push(" WHERE \"id\" = ");
    q.push_bind(id);
    q.append(Sql::new(
        format!(" AND \"status\" IN ({})", vec!["?"; from.len()].join(", ")),
        from.iter().map(|s| Value::Text((*s).to_owned())).collect(),
    ));
    q.push(&format!(
        " RETURNING {}",
        FacebookMarketingMessage::columns_sql(None)
    ));
    q.model(conn)
}

/// `incrementDaySent`.
pub fn increment_day_sent(conn: &Connection, id: i64, by: i64) -> Result<()> {
    conn.execute(
        &format!("UPDATE {TABLE} SET \"daySent\" = \"daySent\" + ?1, \"updatedAt\" = ?2 WHERE \"id\" = ?3"),
        [by, now_ms(), id],
    )?;
    Ok(())
}

/// `setFirstSentAt`: only while still NULL.
pub fn set_first_sent_at(conn: &Connection, id: i64) -> Result<()> {
    let now = now_ms();
    conn.execute(
        &format!(
            "UPDATE {TABLE} SET \"firstSentAt\" = ?1, \"updatedAt\" = ?1 WHERE \"id\" = ?2 \
             AND \"firstSentAt\" IS NULL"
        ),
        [now, id],
    )?;
    Ok(())
}
