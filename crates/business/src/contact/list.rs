//! `packages/business/src/contact/list.ts` `list` / `count` (P3.6-D2 slice
//! contacts list): one page of contacts over
//! `repo::contact_list_where::build_contact_list_where`, the capped list
//! count, and the count by mode with the `InboxContactStat` shortcut.
//! Private query: the ordered page ids (Postgres NULL order: `NULLS LAST`
//! ascending, `NULLS FIRST` descending).

use chatbotx_db::Db;
use chatbotx_db::repo;
use chatbotx_db::repo::contact::{
    ContactInboxWithInbox, ContactListInput, ContactTableRow, ConversationWithAssignees,
    count_capped, list_table_rows, list_with_inboxes_and_conversation, list_with_relations,
    sum_total_contacts_from_inbox_stats,
};
use chatbotx_db::repo::contact_list_where::{
    ListWhereInput, build_contact_list_where, resolve_contact_order_by,
};
use chatbotx_db::repo::models::{Contact, ContactCustomField, Tag};
use chatbotx_db::repo::queries::contact_filter::{ContactInboxScope, FilterCriteria};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::utils::SortDir;
use chatbotx_db::repo::{PaginationInput, SortItem, get_pagination_with_defaults};
use chatbotx_db::rusqlite::types::Value;
use chatbotx_db::rusqlite::{Connection, params_from_iter};

use crate::count_mode::{CountMode, CountOutcome, count_by_mode};
use crate::errors::Result;
use crate::inbox::broadcast::{BroadcastInboxResolutionInput, resolve_broadcast_inbox_ids};
use crate::tx;

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

/// `requiresRecentInteractionWindow` (`broadcastSubactionAudienceRules`).
pub fn requires_recent_interaction_window(subaction: Option<&str>) -> bool {
    matches!(
        subaction,
        Some(
            "messengerActiveContacts"
                | "whatsappWithin24Hours"
                | "instagramActiveContacts"
                | "tiktokActiveContacts"
        )
    )
}

fn member(scope: &ListScope) -> Option<&ContactListScope> {
    match scope {
        ListScope::Unscoped => None,
        ListScope::Member(scope) => Some(scope),
    }
}

/// `hasInboxKeys`.
fn has_inbox_keys(input: &ListContactsInput) -> bool {
    input.channels.as_ref().is_some_and(|c| !c.is_empty())
        || input.inbox_ids.as_ref().is_some_and(|i| !i.is_empty())
        || input.integration_whatsapp_id.is_some()
        || input.integration_messenger_id.is_some()
}

/// `resolveInboxScope`.
fn resolve_inbox_scope(
    conn: &Connection,
    input: &ListContactsInput,
) -> repo::Result<Option<ContactInboxScope>> {
    let require_recent_interaction = requires_recent_interaction_window(input.subaction.as_deref());
    if !has_inbox_keys(input) {
        return Ok(require_recent_interaction.then_some(ContactInboxScope {
            inbox_ids: None,
            require_recent_interaction,
        }));
    }
    let inbox_ids = resolve_broadcast_inbox_ids(
        conn,
        &BroadcastInboxResolutionInput {
            workspace_id: input.workspace_id,
            channels: input.channels.clone(),
            // An empty list means "no restriction" for a filter.
            inbox_ids: input.inbox_ids.clone().filter(|ids| !ids.is_empty()),
            integration_whatsapp_id: input.integration_whatsapp_id,
            integration_messenger_id: input.integration_messenger_id,
        },
    )?;
    Ok(Some(ContactInboxScope {
        inbox_ids: Some(inbox_ids),
        require_recent_interaction,
    }))
}

/// `toListWhereInput` + `buildListWhere`.
fn list_where(
    input: &ListContactsInput,
    scope: Option<&ContactListScope>,
    inbox_scope: Option<ContactInboxScope>,
    now: i64,
) -> Sql {
    build_contact_list_where(&ListWhereInput {
        workspace_id: input.workspace_id,
        keyword: input.keyword.clone(),
        contact_filter: input.contact_filter.clone(),
        inbox_scope,
        restrict_to_assigned_user_id: scope.and_then(|s| s.restrict_to_assigned_user_id),
        restrict_to_team_ids: scope
            .and_then(|s| s.restrict_to_team_ids.clone())
            .unwrap_or_default(),
        include_email_and_phone: scope.is_none_or(|s| s.can_view_email_and_phone),
        now,
    })
}

/// The ordered ids of one page (Postgres NULL order).
fn page_ids(
    conn: &Connection,
    filter: &Sql,
    order_by: &[(String, SortDir)],
    limit: i64,
    offset: i64,
) -> repo::Result<Vec<i64>> {
    let order: Vec<String> = order_by
        .iter()
        .map(|(column, dir)| {
            let nulls = match dir {
                SortDir::Asc => "NULLS LAST",
                SortDir::Desc => "NULLS FIRST",
            };
            format!("\"Contact\".\"{column}\" {} {nulls}", dir.sql())
        })
        .collect();
    let mut q = Sql::raw("SELECT \"Contact\".\"id\" FROM \"Contact\" WHERE ");
    q.append(filter.clone());
    if !order.is_empty() {
        q.push(&format!(" ORDER BY {}", order.join(", ")));
    }
    q.push(" LIMIT ");
    q.push_bind(limit);
    q.push(" OFFSET ");
    q.push_bind(offset);
    let mut stmt = conn.prepare(&q.text)?;
    let rows = stmt.query_map(params_from_iter(q.params.iter()), |r| r.get::<_, i64>(0))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// The repository input of the rows with these ids (unordered).
fn by_ids(ids: &[i64]) -> ContactListInput {
    let marks = vec!["?"; ids.len()].join(", ");
    ContactListInput {
        filter: Sql::new(
            format!("\"Contact\".\"id\" IN ({marks})"),
            ids.iter().copied().map(Value::Integer).collect(),
        ),
        limit: i64::try_from(ids.len()).unwrap_or(i64::MAX),
        offset: 0,
        order_by: Vec::new(),
    }
}

/// Sorts `rows` into the order of `ids`.
fn in_order<T>(ids: &[i64], mut rows: Vec<T>, id_of: impl Fn(&T) -> i64) -> Vec<T> {
    rows.sort_by_key(|row| {
        ids.iter()
            .position(|id| *id == id_of(row))
            .unwrap_or(usize::MAX)
    });
    rows
}

fn full_rows(
    conn: &Connection,
    ids: &[i64],
    include: Option<&[ContactListInclude]>,
) -> repo::Result<Vec<ListedContact>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let wants = |relation: ContactListInclude| include.is_none_or(|list| list.contains(&relation));
    let query = by_ids(ids);
    let rows: Vec<ListedContact> = if include.is_some()
        && !wants(ContactListInclude::Tags)
        && !wants(ContactListInclude::CustomFields)
    {
        list_with_inboxes_and_conversation(conn, &query)?
            .into_iter()
            .map(|row| ListedContact {
                contact: row.contact,
                tags: None,
                contact_custom_fields: None,
                contact_inboxes: Some(row.contact_inboxes),
                conversation: Some(row.conversation),
            })
            .collect()
    } else {
        list_with_relations(conn, &query)?
            .into_iter()
            .map(|row| ListedContact {
                contact: row.contact,
                tags: Some(row.tags),
                contact_custom_fields: Some(row.contact_custom_fields),
                contact_inboxes: Some(row.contact_inboxes),
                conversation: Some(row.conversation),
            })
            .collect()
    };
    let rows = rows
        .into_iter()
        .map(|mut row| {
            // `stripUnrequestedContactRelations`.
            if !wants(ContactListInclude::Tags) {
                row.tags = None;
            }
            if !wants(ContactListInclude::CustomFields) {
                row.contact_custom_fields = None;
            }
            if !wants(ContactListInclude::Inboxes) {
                row.contact_inboxes = None;
            }
            if !wants(ContactListInclude::Conversation) {
                row.conversation = None;
            }
            row
        })
        .collect();
    Ok(in_order(ids, rows, |row| row.contact.id))
}

/// `list`: one page (`now` = unix ms of the filter and inbox windows).
pub async fn list(
    db: &Db,
    input: &ListContactsInput,
    options: &ListOptions,
    now: i64,
) -> Result<ContactListResult> {
    let input = input.clone();
    let options = options.clone();
    tx::read(db, move |conn| {
        let scope = member(&options.scope);
        let inbox_scope = resolve_inbox_scope(conn, &input)?;
        let filter = list_where(&input, scope, inbox_scope, now);
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: Some(input.per_page.unwrap_or(CONTACTS_DEFAULT_PER_PAGE)),
        });
        let order_by = resolve_contact_order_by(input.sort.as_deref());
        let ids = page_ids(
            conn,
            &filter,
            &order_by,
            pagination.limit,
            pagination.offset,
        )?;
        // Unscoped (token) callers see PII; scoped members only when permitted.
        let masks_pii = scope.is_some_and(|s| !s.can_view_email_and_phone);
        let data = match options.projection {
            Projection::Table => {
                let rows = if ids.is_empty() {
                    Vec::new()
                } else {
                    list_table_rows(conn, &by_ids(&ids))?
                };
                let mut rows = in_order(&ids, rows, |row| row.id);
                if masks_pii {
                    for row in &mut rows {
                        row.email = None;
                        row.phone_number = None;
                    }
                }
                ListRows::Table(rows)
            }
            Projection::Full => {
                let mut rows = full_rows(conn, &ids, options.include.as_deref())?;
                if masks_pii {
                    for row in &mut rows {
                        row.contact.email = None;
                        row.contact.phone_number = None;
                    }
                }
                ListRows::Full(rows)
            }
        };
        let (total, capped) = if options.with_count {
            let counted = count_capped(conn, &filter, CONTACT_LIST_COUNT_CAP)?;
            (counted.total, counted.capped)
        } else {
            (0, false)
        };
        let page_count = if options.with_count && pagination.limit > 0 {
            (total + pagination.limit - 1) / pagination.limit
        } else {
            0
        };
        Ok(ContactListResult {
            data,
            page_count,
            total_count: total,
            total_count_capped: capped,
        })
    })
    .await
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
    let reader_input = input.clone();
    let inbox_scope = tx::read(db, move |conn| {
        Ok(resolve_inbox_scope(conn, &reader_input)?)
    })
    .await?;
    let member_scope = member(scope);
    let filtered = input.keyword.as_ref().is_some_and(|k| !k.is_empty())
        || input.contact_filter.is_some()
        || inbox_scope.is_some()
        || member_scope.is_some_and(|s| {
            s.restrict_to_assigned_user_id.is_some() || s.restrict_to_team_ids.is_some()
        });
    if !filtered {
        // The stats shortcut is an O(1) read, so it is exact in either mode.
        let workspace_id = input.workspace_id;
        let total = tx::read(db, move |conn| {
            Ok(sum_total_contacts_from_inbox_stats(conn, workspace_id)?)
        })
        .await
        .unwrap_or(0);
        return Ok(CountOutcome {
            total,
            total_count_capped: false,
            count_cap: None,
        });
    }
    let filter = list_where(input, member_scope, inbox_scope, now);
    let bounded_filter = filter.clone();
    count_by_mode(
        db,
        mode,
        Box::new(move |tx| Ok(repo::contact::count(tx, &filter)?)),
        Box::new(move |cap, tx| Ok(count_capped(tx, &bounded_filter, cap)?)),
    )
    .await
}
