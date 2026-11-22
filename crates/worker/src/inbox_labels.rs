//! `integration/handlers/inbox_labels` (`channelLabelChange` job): Messenger
//! page labels mirrored into workspace tags (Q.I1b).
//!
//! ponytail: the Zalo channel of this job is not ported (Zalo unit); its
//! jobs log "unsupported channel" until then. The `tagApplied` /
//! `tagRemoved` events are not emitted (as the other Rust tag writes, Q.W2).

use chatbotx_db::repo::models::{IntegrationMessenger, TagChannel};
use chatbotx_db::repo::tag_channel::{self, TagChannelKey};
use chatbotx_db::repo::{find_one, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use chatbotx_jobs::JobError;
use serde_json::Value;

use crate::channel::IncomingContact;
use crate::{Deps, str_of};

/// `IntegrationJobAction.channelLabelChange`.
pub const CHANNEL_LABEL_CHANGE: &str = "channelLabelChange";

/// `LabelEvent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelEvent {
    Assign {
        label_id: String,
        label_name: String,
        user_ids: Vec<String>,
    },
    Unassign {
        label_id: String,
        user_ids: Vec<String>,
    },
    DeleteLabel {
        label_id: String,
    },
}

/// `LabelContext`.
#[derive(Debug, Clone)]
struct LabelContext {
    workspace_id: i64,
    integration_id: i64,
    inbox_id: i64,
}

const CHANNEL: &str = "messenger";

/// `messengerChannel.toEvents`: `None` = invalid payload.
pub fn messenger_events(payload: &Value) -> Option<Vec<LabelEvent>> {
    let entries = payload.get("entry")?.as_array()?;
    let Some(changes) = entries.first().and_then(|e| e.get("changes")) else {
        return Some(Vec::new());
    };
    let Some(change) = changes
        .as_array()?
        .iter()
        .find(|c| c.get("field").and_then(Value::as_str) == Some("inbox_labels"))
    else {
        return Some(Vec::new());
    };
    let value = change.get("value")?;
    let label = value.get("label")?;
    let label_id = label.get("id")?.as_str()?.to_owned();
    let user = value
        .pointer("/user/id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let action = value.get("action").and_then(Value::as_str);
    Some(match (action, user) {
        (Some("add"), Some(user)) => vec![LabelEvent::Assign {
            label_id,
            label_name: label
                .get("page_label_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            user_ids: vec![user],
        }],
        (Some("remove"), Some(user)) => vec![LabelEvent::Unassign {
            label_id,
            user_ids: vec![user],
        }],
        (action, user) => {
            tracing::info!(
                channel = CHANNEL,
                ?action,
                has_user = user.is_some(),
                "inbox labels: unsupported label event skipped"
            );
            Vec::new()
        }
    })
}

/// `handleChannelLabelWebhook`.
pub async fn handle(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let channel = str_of(data, "integrationType").unwrap_or_default();
    if channel != CHANNEL {
        tracing::warn!(channel, "inbox labels: unsupported channel");
        return Ok(());
    }
    let page_id = str_of(data, "integrationIdentifier")
        .unwrap_or_default()
        .to_owned();
    let Some(ctx) = load_context(deps, page_id.clone()).await? else {
        return Ok(());
    };
    let Some(events) = messenger_events(data.get("payload").unwrap_or(&Value::Null)) else {
        tracing::warn!(channel, "inbox labels: invalid payload");
        return Ok(());
    };
    for event in events {
        apply_event(deps, &ctx, &page_id, event).await?;
    }
    Ok(())
}

fn db_error(error: impl std::fmt::Display) -> JobError {
    JobError::retryable(error.to_string())
}

/// `messengerChannel.loadContext`.
async fn load_context(deps: &Deps, page_id: String) -> Result<Option<LabelContext>, JobError> {
    let row = deps
        .db
        .read(move |conn| {
            find_one::<IntegrationMessenger>(conn, r#""pageId" = ?1"#, &[&page_id])
                .map_err(crate::repo_err)
        })
        .await
        .map_err(db_error)?;
    let Some(row) = row else {
        tracing::info!(
            channel = CHANNEL,
            "inbox labels: page not connected, label event skipped"
        );
        return Ok(None);
    };
    if row.sync_tag_enabled_at.is_none() {
        tracing::info!(
            channel = CHANNEL,
            workspace_id = row.workspace_id,
            integration_id = row.id,
            "inbox labels: tag sync off for page, label event skipped"
        );
        return Ok(None);
    }
    Ok(Some(LabelContext {
        workspace_id: row.workspace_id,
        integration_id: row.id,
        inbox_id: row.inbox_id,
    }))
}

/// `findTagChannel`.
fn find_tag_channel(
    conn: &Connection,
    ctx: &LabelContext,
    label_id: &str,
) -> chatbotx_db::repo::Result<Option<TagChannel>> {
    find_one::<TagChannel>(
        conn,
        r#""workspaceId" = ?1 AND "channelType" = ?2 AND "integrationId" = ?3 AND "externalLabelId" = ?4"#,
        &[&ctx.workspace_id, &CHANNEL, &ctx.integration_id, &label_id],
    )
}

/// `listIdsByInboxAndSourceIds`: `(contactInboxId, contactId, sourceId)`.
fn find_inboxes(
    conn: &Connection,
    inbox_id: i64,
    source_ids: &[String],
) -> chatbotx_db::rusqlite::Result<Vec<(i64, i64, String)>> {
    let marks = vec!["?"; source_ids.len()].join(",");
    let sql = format!(
        r#"SELECT "id", "contactId", "sourceId" FROM "ContactInbox" WHERE "inboxId" = ? AND "sourceId" IN ({marks})"#
    );
    let mut args: Vec<chatbotx_db::rusqlite::types::Value> = vec![inbox_id.into()];
    args.extend(source_ids.iter().cloned().map(Into::into));
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(args), |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    rows.collect()
}

async fn apply_event(
    deps: &Deps,
    ctx: &LabelContext,
    page_id: &str,
    event: LabelEvent,
) -> Result<(), JobError> {
    match event {
        LabelEvent::Assign {
            label_id,
            label_name,
            user_ids,
        } => {
            let c = ctx.clone();
            let label = label_id.clone();
            let mapping = deps
                .db
                .write(move |tx| {
                    ensure_tag_channel(tx, &c, &label, &label_name).map_err(crate::repo_err)
                })
                .await
                .map_err(db_error)?;
            let Some((tag_id, tag_channel_id)) = mapping else {
                tracing::info!(
                    label_id,
                    "inbox labels: no tag for label (unknown label without a name), assign skipped"
                );
                return Ok(());
            };
            if user_ids.is_empty() {
                return Ok(());
            }
            let inboxes = find_or_create_inboxes(deps, ctx, page_id, user_ids).await?;
            if inboxes.is_empty() {
                tracing::info!(
                    label_id,
                    "inbox labels: no contact for labelled user, assign skipped"
                );
                return Ok(());
            }
            deps.db
                .write(move |tx| {
                    for (contact_inbox_id, contact_id, _) in &inboxes {
                        tx.execute(
                            r#"INSERT INTO "ContactToTag" ("contactId", "tagId") VALUES (?1, ?2) ON CONFLICT DO NOTHING"#,
                            params![contact_id, tag_id],
                        )?;
                        tag_channel::link_contact_inbox(tx, tag_id, tag_channel_id, *contact_inbox_id)
                            .map_err(crate::repo_err)?;
                    }
                    Ok(())
                })
                .await
                .map_err(db_error)
        }
        LabelEvent::Unassign { label_id, user_ids } => {
            if user_ids.is_empty() {
                return Ok(());
            }
            let c = ctx.clone();
            let done = deps
                .db
                .write(move |tx| unassign(tx, &c, &label_id, &user_ids).map_err(crate::repo_err))
                .await
                .map_err(db_error)?;
            if !done.is_empty() {
                tracing::info!(channel = CHANNEL, "inbox labels: {done}");
            }
            Ok(())
        }
        LabelEvent::DeleteLabel { label_id } => {
            // `removeLabel`: a channel-scoped delete of the mapped tag.
            let scope = ctx.clone();
            let label = label_id.clone();
            let mapping = deps
                .db
                .read(move |conn| find_tag_channel(conn, &scope, &label).map_err(crate::repo_err))
                .await
                .map_err(db_error)?;
            let Some(mapping) = mapping else {
                tracing::info!(
                    channel = CHANNEL,
                    label_id,
                    "inbox labels: deleted label not mapped to a tag, skipped"
                );
                return Ok(());
            };
            chatbotx_business::tag_sync::enqueue_delete(
                deps.jobs.as_ref(),
                ctx.workspace_id,
                mapping.tag_id,
                Some((CHANNEL, ctx.integration_id)),
            )
            .await;
            Ok(())
        }
    }
}

/// `unassignLabel`: the skip reason, or `""` when done.
fn unassign(
    conn: &Connection,
    ctx: &LabelContext,
    label_id: &str,
    user_ids: &[String],
) -> chatbotx_db::repo::Result<&'static str> {
    let Some(mapping) = find_tag_channel(conn, ctx, label_id)? else {
        return Ok("label not mapped to a tag, unassign skipped");
    };
    let inboxes = find_inboxes(conn, ctx.inbox_id, user_ids)?;
    if inboxes.is_empty() {
        return Ok("no contact for labelled user, unassign skipped");
    }
    for (contact_inbox_id, contact_id, _) in &inboxes {
        tag_channel::unlink_contact_inbox(conn, mapping.id, *contact_inbox_id)?;
        conn.execute(
            r#"DELETE FROM "ContactToTag" WHERE "contactId" = ?1 AND "tagId" = ?2"#,
            params![contact_id, mapping.tag_id],
        )?;
    }
    Ok("")
}

/// `ensureTagChannel` (get-or-create the tag by name and its mapping).
fn ensure_tag_channel(
    conn: &Connection,
    ctx: &LabelContext,
    label_id: &str,
    name: &str,
) -> chatbotx_db::repo::Result<Option<(i64, i64)>> {
    if let Some(existing) = find_tag_channel(conn, ctx, label_id)? {
        return Ok(Some((existing.tag_id, existing.id)));
    }
    if name.is_empty() {
        return Ok(None);
    }
    let found: Option<i64> = conn
        .query_row(
            r#"SELECT "id" FROM "Tag" WHERE "workspaceId" = ?1 AND "name" = ?2 AND "deletedAt" IS NULL"#,
            params![ctx.workspace_id, name],
            |r| r.get(0),
        )
        .optional()?;
    let tag_id = match found {
        Some(id) => id,
        None => conn.query_row(
            r#"INSERT INTO "Tag" ("id", "createdAt", "updatedAt", "name", "workspaceId") VALUES (?1, ?2, ?2, ?3, ?4)
               ON CONFLICT ("workspaceId", "name") WHERE "deletedAt" IS NULL DO UPDATE SET "name" = excluded."name"
               RETURNING "id""#,
            params![new_id(), now_ms(), name, ctx.workspace_id],
            |r| r.get(0),
        )?,
    };
    let key = TagChannelKey {
        workspace_id: ctx.workspace_id,
        tag_id,
        channel_type: CHANNEL.to_owned(),
        integration_id: ctx.integration_id,
        external_label_id: label_id.to_owned(),
    };
    Ok(tag_channel::upsert_by_tag_and_integration(conn, &key)?.map(|row| (tag_id, row.id)))
}

/// `findOrCreateInboxes`: users the inbox has never seen get a contact
/// first (`createLabelledContact`, source `imported`).
async fn find_or_create_inboxes(
    deps: &Deps,
    ctx: &LabelContext,
    page_id: &str,
    user_ids: Vec<String>,
) -> Result<Vec<(i64, i64, String)>, JobError> {
    let inbox_id = ctx.inbox_id;
    let ids = user_ids.clone();
    let found = deps
        .db
        .read(move |conn| Ok(find_inboxes(conn, inbox_id, &ids)?))
        .await
        .map_err(db_error)?;
    let missing: Vec<&String> = user_ids
        .iter()
        .filter(|id| !found.iter().any(|(_, _, source)| source == *id))
        .collect();
    if missing.is_empty() {
        return Ok(found);
    }
    let Some((_, _, inbox)) = deps
        .channels
        .resolve_by_identifier(&deps.db, deps.realtime.as_ref(), CHANNEL, page_id)
        .await
        .map_err(|e| JobError::unrecoverable(e.to_string()))?
    else {
        return Err(JobError::unrecoverable(format!(
            "Integration not found: {CHANNEL} {page_id}"
        )));
    };
    for source_id in missing {
        let contact = IncomingContact {
            source_id: source_id.clone(),
            ..IncomingContact::default()
        };
        crate::inbound::detect_contact_and_conversation(deps, &inbox, &contact, "imported").await?;
    }
    deps.db
        .read(move |conn| Ok(find_inboxes(conn, inbox_id, &user_ids)?))
        .await
        .map_err(db_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(value: Value) -> Value {
        json!({ "entry": [{ "changes": [{ "field": "inbox_labels", "value": value }] }] })
    }

    #[test]
    fn messenger_label_events() {
        let add = payload(
            json!({ "action": "add", "user": { "id": "u1" }, "label": { "id": "l1", "page_label_name": "VIP" } }),
        );
        assert_eq!(
            messenger_events(&add),
            Some(vec![LabelEvent::Assign {
                label_id: "l1".into(),
                label_name: "VIP".into(),
                user_ids: vec!["u1".into()]
            }])
        );
        let remove =
            payload(json!({ "action": "remove", "user": { "id": "u1" }, "label": { "id": "l1" } }));
        assert_eq!(
            messenger_events(&remove),
            Some(vec![LabelEvent::Unassign {
                label_id: "l1".into(),
                user_ids: vec!["u1".into()]
            }])
        );
        // No user: skipped; another field: nothing; no `entry`: invalid.
        assert_eq!(
            messenger_events(&payload(
                json!({ "action": "add", "label": { "id": "l1" } })
            )),
            Some(vec![])
        );
        assert_eq!(
            messenger_events(
                &json!({ "entry": [{ "changes": [{ "field": "feed", "value": {} }] }] })
            ),
            Some(vec![])
        );
        assert_eq!(messenger_events(&json!({})), None);
        assert_eq!(
            messenger_events(&payload(json!({ "action": "add", "user": { "id": "u1" } }))),
            None
        );
    }

    /// `ensureTagChannel` reuses the tag by name and the mapping by label;
    /// assign then unassign leaves no ContactToTag / ContactToTagChannel row.
    #[tokio::test]
    async fn tag_mapping_assign_unassign() {
        let dir = std::env::temp_dir().join(format!(
            "chatbotx-worker-labels-{}-{}",
            std::process::id(),
            new_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = chatbotx_db::Db::open(dir.join("t.db")).unwrap();
        db.migrate().await.unwrap();
        let counts = db
            .write(|tx| {
                for sql in [
                    r#"INSERT INTO "User" ("id", "email") VALUES (1, 'u@example.test')"#,
                    r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (2, 'WS', 1)"#,
                    r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (4, 2, 'Page', 'messenger', 'p1')"#,
                    r#"INSERT INTO "Tag" ("id", "name", "workspaceId") VALUES (7, 'VIP', 2)"#,
                    r#"INSERT INTO "Contact" ("id", "workspaceId") VALUES (8, 2)"#,
                    r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId") VALUES (9, 8, 8, 4, 'messenger', 'direct', 'u1')"#,
                ] {
                    tx.execute(sql, [])?;
                }
                let ctx = LabelContext { workspace_id: 2, integration_id: 3, inbox_id: 4 };
                let first = ensure_tag_channel(tx, &ctx, "l1", "VIP").map_err(crate::repo_err)?;
                let again = ensure_tag_channel(tx, &ctx, "l1", "").map_err(crate::repo_err)?;
                let unknown = ensure_tag_channel(tx, &ctx, "l2", "").map_err(crate::repo_err)?;
                let (tag_id, mapping_id) = first.unwrap();
                tx.execute(r#"INSERT INTO "ContactToTag" ("contactId", "tagId") VALUES (8, ?1)"#, [tag_id])?;
                tag_channel::link_contact_inbox(tx, tag_id, mapping_id, 9).map_err(crate::repo_err)?;
                let found = find_inboxes(tx, 4, &["u1".into(), "u2".into()])?;
                let done = unassign(tx, &ctx, "l1", &["u1".into()]).map_err(crate::repo_err)?;
                let skipped = unassign(tx, &ctx, "l2", &["u1".into()]).map_err(crate::repo_err)?;
                let left: i64 = tx.query_row(
                    r#"SELECT (SELECT count(*) FROM "ContactToTag") + (SELECT count(*) FROM "ContactToTagChannel")"#,
                    [],
                    |r| r.get(0),
                )?;
                Ok((tag_id, again == first, unknown, found, done, skipped, left))
            })
            .await
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(counts.0, 7);
        assert!(counts.1);
        assert_eq!(counts.2, None);
        assert_eq!(counts.3, vec![(9, 8, "u1".to_owned())]);
        assert_eq!(counts.4, "");
        assert_eq!(counts.5, "label not mapped to a tag, unassign skipped");
        assert_eq!(counts.6, 0);
    }
}
