//! Port of `repositories/facebook-marketing-messages/repository.ts` (P3.2b): the
//! `facebookMarketingMessagesAuthRepository` (`auth_*`) and the campaign repository.

use rusqlite::Connection;

use super::Result;
use super::models::{FacebookMarketingMessage, FacebookMarketingMessagesAuth};
use super::patch::Patch;
use super::utils::SortItem;

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

/// `facebookMarketingMessagesAuthRepository.findByWorkspaceId`.
pub fn auth_find_by_workspace_id(
    _conn: &Connection,
    _workspace_id: i64,
) -> Result<Option<FacebookMarketingMessagesAuth>> {
    todo!("P3.2b")
}

/// `facebookMarketingMessagesAuthRepository.create`.
pub fn auth_create(
    _conn: &Connection,
    _input: &AuthCreateInput,
) -> Result<FacebookMarketingMessagesAuth> {
    todo!("P3.2b")
}

/// `facebookMarketingMessagesAuthRepository.updateAuth`: replaces the token, status `active`.
pub fn auth_update_auth(
    _conn: &Connection,
    _input: &AuthUpdateInput,
) -> Result<Option<FacebookMarketingMessagesAuth>> {
    todo!("P3.2b")
}

/// `facebookMarketingMessagesAuthRepository.updateStatus`.
pub fn auth_update_status(_conn: &Connection, _workspace_id: i64, _status: &str) -> Result<()> {
    todo!("P3.2b")
}

/// `listByWorkspaceId` (createdAt DESC).
pub fn list_by_workspace_id(
    _conn: &Connection,
    _workspace_id: i64,
) -> Result<Vec<FacebookMarketingMessage>> {
    todo!("P3.2b")
}

/// `listForStatistics`.
pub fn list_for_statistics(
    _conn: &Connection,
    _input: &StatisticsListInput,
) -> Result<StatisticsPage> {
    todo!("P3.2b")
}

/// `listAdAccountsByWorkspaceId`: one entry per ad account, newest-used first.
pub fn list_ad_accounts_by_workspace_id(
    _conn: &Connection,
    _workspace_id: i64,
) -> Result<Vec<AdAccountSummary>> {
    todo!("P3.2b")
}

/// `findForWorkspace`.
pub fn find_for_workspace(
    _conn: &Connection,
    _id: i64,
    _workspace_id: i64,
) -> Result<Option<FacebookMarketingMessage>> {
    todo!("P3.2b")
}

/// `create`.
pub fn create(_conn: &Connection, _input: &CreateInput) -> Result<FacebookMarketingMessage> {
    todo!("P3.2b")
}

/// `update`.
pub fn update(
    _conn: &Connection,
    _input: &UpdateInput,
) -> Result<Option<FacebookMarketingMessage>> {
    todo!("P3.2b")
}

/// `countByAudienceId`.
pub fn count_by_audience_id(
    _conn: &Connection,
    _audience_id: i64,
    _workspace_id: i64,
) -> Result<i64> {
    todo!("P3.2b")
}

/// `remove`.
pub fn remove(_conn: &Connection, _ids: &[i64], _workspace_id: i64) -> Result<()> {
    todo!("P3.2b")
}

/// `listDueToStart`.
pub fn list_due_to_start(_conn: &Connection, _now: i64) -> Result<Vec<FacebookMarketingMessage>> {
    todo!("P3.2b")
}

/// `listByStatus`.
pub fn list_by_status(_conn: &Connection, _status: &str) -> Result<Vec<FacebookMarketingMessage>> {
    todo!("P3.2b")
}

/// `findById` (unscoped).
pub fn find_by_id(_conn: &Connection, _id: i64) -> Result<Option<FacebookMarketingMessage>> {
    todo!("P3.2b")
}

/// `findByCampaignId` (unscoped).
pub fn find_by_campaign_id(
    _conn: &Connection,
    _campaign_id: &str,
) -> Result<Option<FacebookMarketingMessage>> {
    todo!("P3.2b")
}

/// `transition`: conditional update from one of `from`; `None` when another worker won.
pub fn transition(
    _conn: &Connection,
    _id: i64,
    _from: &[&str],
    _patch: &Patch,
) -> Result<Option<FacebookMarketingMessage>> {
    todo!("P3.2b")
}

/// `incrementDaySent`.
pub fn increment_day_sent(_conn: &Connection, _id: i64, _by: i64) -> Result<()> {
    todo!("P3.2b")
}

/// `setFirstSentAt`: only while still NULL.
pub fn set_first_sent_at(_conn: &Connection, _id: i64) -> Result<()> {
    todo!("P3.2b")
}
