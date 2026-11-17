//! `features/conversations/queries/list-conversations.query.ts`
//! (`listConversations`, `findConversation`) and `build-conversation-where.ts`
//! (`buildConversationWhere`) for the public `/v1/conversations` routes.
//!
//! The outer `Conversation` row is aliased `c`; the contact is joined as
//! `"Contact"` so the shared contact-filter / keyword SQL applies unchanged.
//! No member scope (workspace tokens only; P3.6-D2 decision).

use chatbotx_db::Db;
use chatbotx_db::repo::contact::ContactInboxWithInbox;
use chatbotx_db::repo::message::{
    FindLastByConversationOptions, MessageWithAttachments, SinceTimeInput,
    find_last_by_conversation, get_safe_since_time,
};
use chatbotx_db::repo::models::{Contact, ContactInbox, Conversation, Inbox, InboxTeam, User};
use chatbotx_db::repo::queries::contact_filter::{
    FilterCriteria, SmartKeywordOptions, apply_contact_filter, build_smart_keyword_where,
    parse_conversation_assignee_values, prune_email_phone_filter_conditions,
};
use chatbotx_db::repo::queries::conversation_unread::conversation_unread_where;
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::{Model, contact_inbox, now_ms};
use chatbotx_db::rusqlite::Connection;

use crate::errors::{BusinessError, Result};
use crate::tx;

const DEFAULT_PER_PAGE: i64 = 20;
const HOUR_MS: i64 = 60 * 60 * 1000;

/// `conversationCursorSchema`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListCursor {
    pub last_activity_at: Option<i64>,
    pub id: i64,
}

/// `ListConversationsRequest` (without `view`; `status` does not filter).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListInput {
    pub workspace_id: i64,
    /// `all` | `bot` | `human`.
    pub bot_category: Option<String>,
    pub assigned_id: Option<String>,
    pub channel: Option<String>,
    pub keyword: Option<String>,
    pub bot_enabled: Option<bool>,
    pub tags: Vec<String>,
    pub contact_filter: Option<FilterCriteria>,
    pub per_page: Option<i64>,
    pub cursor: Option<ListCursor>,
    /// `!includeEmailAndPhone`: a member without `emailAndPhone` (the private
    /// list) cannot search or filter by email / phone. Tokens see them.
    pub hide_email_and_phone: bool,
    /// `conversationMemberScope.where` of a mobile member (Q.A1 §3); `None`
    /// = workspace-level. A `none` scope matches nothing (callers 404 first).
    pub member_scope: Option<crate::member_scope::ConversationMemberScope>,
}

/// One conversation with `contact`, `contactInboxes.inbox`, the assignees
/// and the last message (`findLastByConversation`, attachment count only).
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationRow {
    pub conversation: Conversation,
    pub contact: Option<Contact>,
    pub contact_inboxes: Vec<ContactInboxWithInbox>,
    pub assigned_user: Option<User>,
    pub assigned_inbox_team: Option<InboxTeam>,
    pub last_message: Option<MessageWithAttachments>,
}

/// One page of [`list`].
#[derive(Debug, Clone, PartialEq)]
pub struct ListPage {
    pub rows: Vec<ConversationRow>,
    pub next_cursor: Option<ListCursor>,
    pub prev_cursor: Option<ListCursor>,
}

fn by_id<M: Model>(conn: &Connection, id: Option<i64>) -> Result<Option<M>> {
    let Some(id) = id else {
        return Ok(None);
    };
    let mut q = Sql::raw(format!(
        "SELECT {} FROM {} WHERE \"id\" = ",
        M::columns_sql(None),
        M::TABLE
    ));
    q.push_bind(id);
    Ok(q.model(conn)?)
}

fn inboxes_of(conn: &Connection, contact_id: i64) -> Result<Vec<ContactInboxWithInbox>> {
    let rows: Vec<ContactInbox> = contact_inbox::list_by_contact_id(conn, contact_id)?;
    let mut out = Vec::with_capacity(rows.len());
    for contact_inbox in rows {
        if let Some(inbox) = by_id::<Inbox>(conn, Some(contact_inbox.inbox_id))? {
            out.push(ContactInboxWithInbox {
                contact_inbox,
                inbox,
            });
        }
    }
    Ok(out)
}

/// `resolveLastMessageSinceTime`: epoch 0 without an anchor.
fn last_message_since(anchor: Option<i64>, end_of_hour: bool) -> i64 {
    anchor
        .and_then(|ms| {
            let ms = if end_of_hour {
                ms.div_euclid(HOUR_MS) * HOUR_MS + HOUR_MS - 1
            } else {
                ms
            };
            get_safe_since_time(Some(SinceTimeInput::Date(ms)), None)
        })
        .unwrap_or(0)
}

fn load_row(
    conn: &Connection,
    conversation: Conversation,
    message_types: &[&str],
    end_of_hour: bool,
) -> Result<ConversationRow> {
    let last_message = find_last_by_conversation(
        conn,
        conversation.id,
        &FindLastByConversationOptions {
            attachment_count_only: true,
            limit: Some(1),
            message_types: message_types.iter().map(|t| (*t).to_owned()).collect(),
            since_time: Some(last_message_since(
                conversation.last_activity_at,
                end_of_hour,
            )),
            workspace_id: Some(conversation.workspace_id),
            ..FindLastByConversationOptions::default()
        },
    )?
    .into_iter()
    .next();
    Ok(ConversationRow {
        contact: by_id(conn, Some(conversation.contact_id))?,
        contact_inboxes: inboxes_of(conn, conversation.contact_id)?,
        assigned_user: by_id(conn, conversation.assigned_user_id)?,
        assigned_inbox_team: by_id(conn, conversation.assigned_inbox_team_id)?,
        last_message,
        conversation,
    })
}

fn bind(text: &str, value: impl Into<chatbotx_db::rusqlite::types::Value>) -> Option<Sql> {
    Some(Sql::bind(text, value))
}

/// `buildConversationWhere` over `c`.
pub fn build_conversation_where(input: &ListInput, now: i64) -> Sql {
    let has = |tag: &str| input.tags.iter().any(|t| t == tag);
    let mut parts: Vec<Option<Sql>> = vec![bind("c.\"workspaceId\" = ", input.workspace_id)];
    if let Some(scope) = &input.member_scope {
        use crate::member_scope::ChatScope;
        match scope.chat_scope {
            ChatScope::All => {}
            ChatScope::None => parts.push(Some(Sql::raw("1 = 0"))),
            ChatScope::Assigned => {
                let teams = serde_json::to_string(&scope.team_ids).unwrap_or_else(|_| "[]".into());
                parts.push(Some(Sql::or([
                    bind("c.\"assignedUserId\" = ", scope.user_id),
                    Some(
                        Sql::bind(
                            "c.\"assignedInboxTeamId\" IN (SELECT value FROM json_each(",
                            teams,
                        )
                        .closed("))"),
                    ),
                ])));
            }
        }
    }
    parts.push(Some(Sql::raw(if has("archived") {
        "c.\"archivedAt\" IS NOT NULL"
    } else {
        "c.\"archivedAt\" IS NULL"
    })));
    let filter_mentions_blocked = input.contact_filter.as_ref().is_some_and(|f| {
        f.conditions
            .iter()
            .any(|c| c.get("field").and_then(|v| v.as_str()) == Some("blocked"))
    });
    if has("blocked") {
        parts.push(Some(Sql::raw("\"Contact\".\"blockedAt\" IS NOT NULL")));
    } else if !filter_mentions_blocked {
        parts.push(Some(Sql::raw("\"Contact\".\"blockedAt\" IS NULL")));
    }
    if let Some(cursor) = input.cursor {
        parts.push(Some(match cursor.last_activity_at {
            Some(at) => Sql::new(
                "c.\"lastActivityAt\" < ? OR c.\"lastActivityAt\" IS NULL OR (c.\"lastActivityAt\" = ? AND c.\"id\" < ?)",
                vec![at.into(), at.into(), cursor.id.into()],
            ),
            None => Sql::bind("c.\"lastActivityAt\" IS NULL AND c.\"id\" < ", cursor.id),
        }));
    }
    let mut bot_enabled = match input.bot_category.as_deref() {
        Some("bot") => Some(true),
        Some("human") => Some(false),
        _ => None,
    };
    if input.bot_enabled.is_some() {
        bot_enabled = input.bot_enabled;
    }
    if let Some(enabled) = bot_enabled {
        parts.push(bind("c.\"botEnabled\" = ", enabled));
    }
    if let Some(assigned) = input.assigned_id.as_deref() {
        let selection = parse_conversation_assignee_values(&[assigned]);
        let id = |ids: &[String]| ids.first().and_then(|id| id.parse::<i64>().ok());
        if assigned == "unassigned" {
            parts.push(Some(Sql::raw(
                "c.\"assignedUserId\" IS NULL AND c.\"assignedInboxTeamId\" IS NULL",
            )));
        } else if let Some(user) = id(&selection.user_ids) {
            parts.push(bind("c.\"assignedUserId\" = ", user));
        } else if let Some(team) = id(&selection.inbox_team_ids) {
            parts.push(bind("c.\"assignedInboxTeamId\" = ", team));
        }
    }
    if let Some(channel) = input.channel.as_deref().filter(|c| *c != "omnichannel") {
        parts.push(Some(
            Sql::bind(
                "EXISTS (SELECT 1 FROM \"ContactInbox\" WHERE \"ContactInbox\".\"contactId\" = c.\"contactId\" AND \"ContactInbox\".\"channel\" = ",
                channel.to_owned(),
            )
            .closed(")"),
        ));
    }
    if let Some(keyword) = input.keyword.as_deref() {
        parts.push(build_smart_keyword_where(
            keyword,
            SmartKeywordOptions {
                include_email_and_phone: !input.hide_email_and_phone,
                ..SmartKeywordOptions::default()
            },
        ));
    }
    if has("noAdminReply") {
        parts.push(Some(Sql::raw(
            "c.\"contactRepliedAt\" > c.\"adminRepliedAt\"",
        )));
    }
    if has("unread") {
        parts.push(Some(Sql::raw(conversation_unread_where("c"))));
    }
    if has("followUp") {
        parts.push(Some(Sql::raw("c.\"followed\" = 1")));
    }
    if has("slaBreached") {
        parts.push(bind("c.\"responseDueAt\" <= ", now));
    }
    if has("slaBreaching") {
        parts.push(bind("c.\"responseSlaBreachingAt\" <= ", now));
        parts.push(bind("c.\"responseDueAt\" > ", now));
    }
    if let Some(filter) = prune_email_phone_filter_conditions(
        input.contact_filter.as_ref(),
        !input.hide_email_and_phone,
    ) {
        parts.push(apply_contact_filter(&filter, Some(input.workspace_id), now));
    }
    Sql::and(parts)
}

fn list_in(conn: &Connection, input: &ListInput, now: i64) -> Result<ListPage> {
    let limit = input.per_page.unwrap_or(DEFAULT_PER_PAGE);
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"Conversation\" c JOIN \"Contact\" ON \"Contact\".\"id\" = c.\"contactId\" WHERE ",
        Conversation::columns_sql(Some("c"))
    ));
    q.append(build_conversation_where(input, now));
    q.push(
        " ORDER BY c.\"lastActivityAt\" IS NULL, c.\"lastActivityAt\" DESC, c.\"id\" DESC LIMIT ",
    );
    q.push_bind(limit + 1);
    let mut conversations: Vec<Conversation> = q.models(conn)?;
    let has_more = conversations.len() as i64 > limit;
    conversations.truncate(usize::try_from(limit).unwrap_or(0));
    let cursor_of = |c: &Conversation| ListCursor {
        last_activity_at: c.last_activity_at,
        id: c.id,
    };
    let next_cursor = if has_more {
        conversations.last().map(cursor_of)
    } else {
        None
    };
    // `page[0]?.lastActivityAt ?? new Date()`, `page[0]?.id ?? "0"`.
    let prev_cursor = input.cursor.map(|_| ListCursor {
        last_activity_at: Some(
            conversations
                .first()
                .and_then(|c| c.last_activity_at)
                .unwrap_or(now),
        ),
        id: conversations.first().map_or(0, |c| c.id),
    });
    let rows = conversations
        .into_iter()
        .map(|c| load_row(conn, c, &[], false))
        .collect::<Result<Vec<_>>>()?;
    Ok(ListPage {
        rows,
        next_cursor,
        prev_cursor,
    })
}

/// `listConversations`: `lastActivityAt` desc (NULL last), `id` desc;
/// `perPage` + 1 rows decide `nextCursor`.
pub async fn list(db: &Db, input: ListInput) -> Result<ListPage> {
    let now = now_ms();
    tx::read(db, move |conn| list_in(conn, &input, now)).await
}

/// `conversationService.findBy({ id, workspaceId })`.
pub(crate) fn find_in(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Option<Conversation>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"Conversation\" WHERE \"workspaceId\" = ",
        Conversation::columns_sql(None)
    ));
    q.push_bind(workspace_id);
    q.push(" AND \"id\" = ");
    q.push_bind(id);
    Ok(q.model(conn)?)
}

/// `findByOrFail` (404 "Conversation not found").
pub(crate) fn find_or_fail_in(
    conn: &Connection,
    workspace_id: i64,
    id: i64,
) -> Result<Conversation> {
    find_in(conn, workspace_id, id)?
        .ok_or_else(|| BusinessError::not_found("Conversation not found"))
}

/// `assertConversationAccessForCaller` of a workspace token: the conversation
/// of the workspace, else 404 "Conversation not found".
pub async fn find_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<Conversation> {
    tx::read(db, move |conn| find_or_fail_in(conn, workspace_id, id)).await
}

/// `findConversation`: the full row; the last incoming / outgoing message
/// (window anchored on `endOfHour(lastActivityAt)`).
pub async fn find(db: &Db, workspace_id: i64, id: i64) -> Result<ConversationRow> {
    tx::read(db, move |conn| {
        let conversation = find_or_fail_in(conn, workspace_id, id)?;
        load_row(conn, conversation, &["incoming", "outgoing"], true)
    })
    .await
}
