//! `packages/business/src/contact/list.ts` `list` / `count` (P3.6-D2 slice
//! contacts list): one page of contacts over
//! `repo::contact_list_where::build_contact_list_where`, the capped list
//! count, and the count by mode with the `InboxContactStat` shortcut.
//! Private query: the ordered page ids (Postgres NULL order: `NULLS LAST`
//! ascending, `NULLS FIRST` descending).

use chatbotx_db::Db;
use chatbotx_db::repo::SortItem;
use chatbotx_db::repo::contact::{
    ContactInboxWithInbox, ContactTableRow, ConversationWithAssignees,
};
use chatbotx_db::repo::models::{Contact, ContactCustomField, Tag};
use chatbotx_db::repo::queries::contact_filter::FilterCriteria;

use crate::count_mode::{CountMode, CountOutcome};
use crate::errors::Result;

/// `CONTACTS_DEFAULT_PER_PAGE`.
pub const CONTACTS_DEFAULT_PER_PAGE: i64 = 50;
/// `CONTACT_LIST_COUNT_CAP`.
pub const CONTACT_LIST_COUNT_CAP: i64 = 10_000;

/// `ContactListScope` (`ContactAccessScope` + `canViewEmailAndPhone`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactListScope {
    pub restrict_to_assigned_user_id: Option<i64>,
    pub restrict_to_team_ids: Option<Vec<i64>>,
    pub can_view_email_and_phone: bool,
}

/// `ContactListScopeInput`: `UNSCOPED` (workspace token) or a member scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ListScope {
    #[default]
    Unscoped,
    Member(ContactListScope),
}

/// `ListContactsInput`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListContactsInput {
    pub workspace_id: i64,
    pub keyword: Option<String>,
    pub contact_filter: Option<FilterCriteria>,
    pub channels: Option<Vec<String>>,
    pub inbox_ids: Option<Vec<i64>>,
    pub integration_whatsapp_id: Option<i64>,
    pub integration_messenger_id: Option<i64>,
    /// `BroadcastSubaction`.
    pub subaction: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<SortItem>>,
}

/// `projection`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Projection {
    #[default]
    Full,
    Table,
}

/// `ContactListInclude`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactListInclude {
    Tags,
    CustomFields,
    Inboxes,
    Conversation,
}

impl ContactListInclude {
    /// `tags` / `customFields` / `inboxes` / `conversation`.
    pub fn parse(text: &str) -> Option<ContactListInclude> {
        match text {
            "tags" => Some(ContactListInclude::Tags),
            "customFields" => Some(ContactListInclude::CustomFields),
            "inboxes" => Some(ContactListInclude::Inboxes),
            "conversation" => Some(ContactListInclude::Conversation),
            _ => None,
        }
    }
}

/// The options of [`list`] (`scope`, `projection`, `include`, `withCount`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListOptions {
    pub scope: ListScope,
    pub projection: Projection,
    /// `None` = every relation.
    pub include: Option<Vec<ContactListInclude>>,
    pub with_count: bool,
}

impl Default for ListOptions {
    fn default() -> Self {
        ListOptions {
            scope: ListScope::Unscoped,
            projection: Projection::Full,
            include: None,
            with_count: true,
        }
    }
}

/// One "full" row; a relation the caller did not request is `None`
/// (`stripUnrequestedContactRelations`).
#[derive(Debug, Clone, PartialEq)]
pub struct ListedContact {
    pub contact: Contact,
    pub tags: Option<Vec<Tag>>,
    pub contact_custom_fields: Option<Vec<ContactCustomField>>,
    pub contact_inboxes: Option<Vec<ContactInboxWithInbox>>,
    pub conversation: Option<Option<ConversationWithAssignees>>,
}

/// The rows of one page.
#[derive(Debug, Clone, PartialEq)]
pub enum ListRows {
    Full(Vec<ListedContact>),
    Table(Vec<ContactTableRow>),
}

/// `ContactListResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactListResult {
    pub data: ListRows,
    pub page_count: i64,
    pub total_count: i64,
    pub total_count_capped: bool,
}

/// `requiresRecentInteractionWindow`.
pub fn requires_recent_interaction_window(subaction: Option<&str>) -> bool {
    let _ = subaction;
    todo!("P3.6-D2")
}

/// `list`: one page (`now` = unix ms of the filter and inbox windows).
pub async fn list(
    db: &Db,
    input: &ListContactsInput,
    options: &ListOptions,
    now: i64,
) -> Result<ContactListResult> {
    let _ = (db, input, options, now);
    todo!("P3.6-D2")
}

/// `count`: the `InboxContactStat` sum without keyword / filter / inbox
/// keys / member restriction, else a count by `mode`.
pub async fn count(
    db: &Db,
    input: &ListContactsInput,
    scope: &ListScope,
    mode: Option<CountMode>,
    now: i64,
) -> Result<CountOutcome> {
    let _ = (db, input, scope, mode, now);
    todo!("P3.6-D2")
}
