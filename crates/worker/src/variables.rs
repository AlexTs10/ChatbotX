//! [`VariableSources`] on the database: the `{{variable}}` data of the
//! flow step sends (`resolveContactVariablesDeep`).
//!
//! ponytail: sources of unported domains (calls, lead ads,
//! questionnaires, leaderboards, coupons, pipelines, channel profile and post
//! lookups, attachments, the default API token) answer "nothing" (`None`,
//! empty, `""`), so their variables resolve empty instead of failing the send.
//! Each domain unit fills its source in.

use std::sync::Arc;

use chatbotx_core::variables::{Appointment, Attachment};
use chatbotx_core::variables::{
    AttachmentMediaRequest, AvatarResolution, BotFieldRow, CallRecord, ChatMessage,
    ContactCustomFieldValue, ContactInboxModel, ContactLocation, ContactModel,
    ContactProfileSnapshot, ConversationModel, InboxWithIntegrations, LeadAdsLead,
    MeSystemFieldPayload, MemberUser, MessageModel, MessageWithAttachments, PipelineVariableTarget,
    Referral, SourceError, SourceResult, TenantSettings, VariableSources, WorkspaceModel,
};
use chatbotx_db::Db;
use chatbotx_db::repo::Model as _;
use chatbotx_db::repo::models::{Contact, ContactInbox, Conversation, Workspace};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::{Value, json};

pub struct DbSources {
    pub db: Arc<Db>,
    /// `NEXT_PUBLIC_BUILDER_URL` without a trailing `/`.
    pub app_url: String,
}

fn id(text: &str) -> SourceResult<i64> {
    text.parse()
        .map_err(|_| SourceError::new(format!("invalid id {text}")))
}

fn opt_str(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn contact_model(row: Contact) -> ContactModel {
    let location = row.location.map(|l| l.0);
    ContactModel {
        id: row.id.to_string(),
        workspace_id: row.workspace_id.to_string(),
        first_name: row.first_name,
        last_name: row.last_name,
        email: row.email,
        phone_number: row.phone_number,
        avatar: row.avatar,
        gender: row.gender,
        locale: row.locale,
        timezone: row.timezone,
        country: row.country,
        state: row.state,
        city: row.city,
        ref_: row.r#ref,
        location: location.filter(|l| l.is_object()).map(|l| ContactLocation {
            latitude: l.get("latitude").and_then(Value::as_f64),
            longitude: l.get("longitude").and_then(Value::as_f64),
        }),
        total_tagged: Some(row.total_tagged),
        total_new_tagged: Some(row.total_new_tagged),
    }
}

fn contact_inbox_model(row: ContactInbox) -> ContactInboxModel {
    let referral = row.referral.map(|r| r.0).filter(Value::is_object);
    ContactInboxModel {
        id: row.id.to_string(),
        inbox_id: row.inbox_id.to_string(),
        source_id: row.source_id,
        channel: row.channel,
        source: Some(row.source),
        created_at: Some(row.created_at),
        contact_last_read_at: row.contact_last_read_at,
        last_incoming_message_at: row.last_incoming_message_at,
        last_outbound_message_at: row.last_outbound_message_at,
        first_interaction_at: row.first_interaction_at,
        language: row.language,
        last_btn_title: row.last_btn_title,
        last_story_id: row.last_story_id,
        last_input_failure: row.last_input_failure,
        last_error_log: row.last_error_log,
        webchat_parent_url: row.webchat_parent_url,
        consecutive_failed_reply: Some(row.consecutive_failed_reply),
        last_comment_message_id: row.last_comment_message_id,
        last_comment_message_at: row.last_comment_message_at,
        referral: referral.map(|r| Referral {
            ref_: opt_str(&r, "ref"),
            ad_id: opt_str(&r, "adId"),
            ad_title: opt_str(&r, "adTitle"),
            ad_body: opt_str(&r, "adBody"),
            ctwa_clid: opt_str(&r, "ctwaClid"),
            source_url: opt_str(&r, "sourceUrl"),
            source_platform: opt_str(&r, "sourcePlatform"),
        }),
        source_username: row.source_username,
        source_user_id: row.source_user_id,
        follows_business: row.follows_business,
        business_follows_contact: row.business_follows_contact,
        account_verified: row.account_verified,
        follower_count: row.follower_count,
    }
}

pub(crate) fn conversation_model(row: &Conversation) -> ConversationModel {
    ConversationModel {
        id: row.id.to_string(),
        workspace_id: row.workspace_id.to_string(),
        contact_id: row.contact_id.to_string(),
        assigned_user_id: row.assigned_user_id.map(|v| v.to_string()),
        assigned_inbox_team_id: row.assigned_inbox_team_id.map(|v| v.to_string()),
        last_step: row.last_step.clone(),
        current_step: row.current_step.clone(),
        last_activity_at: row.last_activity_at,
        created_at: Some(row.created_at),
    }
}

impl DbSources {
    /// An appointment row with its calendar name (`include: { calendar }`).
    async fn appointment(
        &self,
        workspace_id: i64,
        find: impl FnOnce(
            &Connection,
        )
            -> chatbotx_db::repo::Result<Option<chatbotx_db::repo::models::Appointment>>
        + Send
        + 'static,
    ) -> SourceResult<Option<Appointment>> {
        chatbotx_business::tx::read(&self.db, move |conn| {
            let Some(row) = find(conn)? else {
                return Ok(None);
            };
            let calendar = chatbotx_db::repo::appointment_calendar::find_by(
                conn,
                workspace_id,
                row.calendar_id,
                true,
            )?;
            Ok(Some(Appointment {
                id: row.id.to_string(),
                workspace_id: row.workspace_id.to_string(),
                contact_id: row.contact_id.to_string(),
                conversation_id: row.conversation_id.map(|id| id.to_string()),
                start_at: Some(row.start_at),
                invitee_timezone: Some(row.invitee_timezone),
                calendar_name: calendar.map(|c| c.name),
            }))
        })
        .await
        .map_err(|e| SourceError::new(e.to_string()))
    }

    async fn read<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Connection) -> chatbotx_db::rusqlite::Result<T> + Send + 'static,
    ) -> SourceResult<T> {
        self.db
            .read(move |conn| Ok(f(conn)?))
            .await
            .map_err(|e| SourceError::new(e.to_string()))
    }

    async fn conversation_where(
        &self,
        filter: &'static str,
        args: Vec<i64>,
    ) -> SourceResult<Option<ConversationModel>> {
        self.read(move |conn| {
            conn.query_row(
                &format!("{} WHERE {filter} LIMIT 1", Conversation::select_sql()),
                chatbotx_db::rusqlite::params_from_iter(args),
                Conversation::from_row,
            )
            .optional()
        })
        .await
        .map(|row| row.as_ref().map(conversation_model))
    }
}

impl VariableSources for DbSources {
    async fn find_contact(&self, contact_id: &str) -> SourceResult<Option<ContactModel>> {
        let contact_id = id(contact_id)?;
        let row = self
            .read(move |conn| {
                conn.query_row(
                    &format!(r#"{} WHERE "id" = ?1"#, Contact::select_sql()),
                    params![contact_id],
                    Contact::from_row,
                )
                .optional()
            })
            .await?;
        Ok(row.map(contact_model))
    }

    async fn find_contact_inbox(
        &self,
        contact_inbox_id: &str,
    ) -> SourceResult<Option<ContactInboxModel>> {
        let contact_inbox_id = id(contact_inbox_id)?;
        let row = self
            .read(move |conn| {
                conn.query_row(
                    &format!(r#"{} WHERE "id" = ?1"#, ContactInbox::select_sql()),
                    params![contact_inbox_id],
                    ContactInbox::from_row,
                )
                .optional()
            })
            .await?;
        Ok(row.map(contact_inbox_model))
    }

    async fn find_workspace(&self, workspace_id: &str) -> SourceResult<Option<WorkspaceModel>> {
        let workspace_id = id(workspace_id)?;
        let row = self
            .read(move |conn| {
                conn.query_row(
                    &format!(r#"{} WHERE "id" = ?1"#, Workspace::select_sql()),
                    params![workspace_id],
                    Workspace::from_row,
                )
                .optional()
            })
            .await?;
        Ok(row.map(|w| WorkspaceModel {
            id: w.id.to_string(),
            name: Some(w.name),
            logo: w.logo,
            timezone: Some(w.timezone),
            language: Some(w.language),
            scheduled_deletion_at: w.scheduled_deletion_at,
        }))
    }

    async fn list_contact_custom_fields(
        &self,
        contact_id: &str,
    ) -> SourceResult<Vec<ContactCustomFieldValue>> {
        let contact_id = id(contact_id)?;
        self.read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT f."name", f."type", ccf."value", coalesce(f."description", '')
                   FROM "ContactCustomField" ccf JOIN "CustomField" f ON f."id" = ccf."customFieldId"
                   WHERE ccf."contactId" = ?1 ORDER BY ccf."id""#,
            )?;
            stmt.query_map(params![contact_id], |row| {
                Ok(ContactCustomFieldValue {
                    key: row.get(0)?,
                    kind: row.get(1)?,
                    value: row.get(2)?,
                    description: row.get(3)?,
                })
            })?
            .collect()
        })
        .await
    }

    async fn list_bot_fields(&self, workspace_id: &str) -> SourceResult<Vec<BotFieldRow>> {
        let workspace_id = id(workspace_id)?;
        self.read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "id", "type", "value" FROM "BotField" WHERE "workspaceId" = ?1 ORDER BY "id""#,
            )?;
            stmt.query_map(params![workspace_id], |row| {
                Ok(BotFieldRow {
                    id: row.get::<_, i64>(0)?.to_string(),
                    kind: row.get(1)?,
                    value: row.get(2)?,
                })
            })?
            .collect()
        })
        .await
    }

    async fn find_conversation_by_contact(
        &self,
        contact_id: &str,
        workspace_id: Option<&str>,
    ) -> SourceResult<Option<ConversationModel>> {
        let contact_id = id(contact_id)?;
        match workspace_id {
            Some(ws) => {
                self.conversation_where(
                    r#""contactId" = ?1 AND "workspaceId" = ?2"#,
                    vec![contact_id, id(ws)?],
                )
                .await
            }
            None => {
                self.conversation_where(r#""contactId" = ?1"#, vec![contact_id])
                    .await
            }
        }
    }

    async fn find_dm_conversation_by_contact(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> SourceResult<Option<ConversationModel>> {
        self.conversation_where(
            r#""workspaceId" = ?1 AND "contactId" = ?2 AND "sourceId" IS NULL"#,
            vec![id(workspace_id)?, id(contact_id)?],
        )
        .await
    }

    async fn find_latest_conversation_by_contact(
        &self,
        contact_id: &str,
    ) -> SourceResult<Option<ConversationModel>> {
        self.conversation_where(
            r#""contactId" = ?1 ORDER BY "lastActivityAt" IS NULL, "lastActivityAt" DESC, "id" DESC"#,
            vec![id(contact_id)?],
        )
        .await
    }

    async fn find_member_user(
        &self,
        workspace_id: &str,
        user_id: &str,
    ) -> SourceResult<Option<MemberUser>> {
        let (workspace_id, user_id) = (id(workspace_id)?, id(user_id)?);
        self.read(move |conn| {
            conn.query_row(
                r#"SELECT u."id", u."name", u."email" FROM "WorkspaceMember" m
                   JOIN "User" u ON u."id" = m."userId"
                   WHERE m."workspaceId" = ?1 AND m."userId" = ?2 LIMIT 1"#,
                params![workspace_id, user_id],
                |row| {
                    Ok(MemberUser {
                        id: row.get::<_, i64>(0)?.to_string(),
                        name: row.get(1)?,
                        email: row.get(2)?,
                    })
                },
            )
            .optional()
        })
        .await
    }

    async fn find_inbox_team_name(&self, team_id: &str) -> SourceResult<Option<String>> {
        let team_id = id(team_id)?;
        self.read(move |conn| {
            conn.query_row(
                r#"SELECT "name" FROM "InboxTeam" WHERE "id" = ?1"#,
                params![team_id],
                |row| row.get(0),
            )
            .optional()
        })
        .await
    }

    async fn list_team_names_by_user_id(
        &self,
        workspace_id: &str,
        user_id: &str,
    ) -> SourceResult<Vec<String>> {
        let (workspace_id, user_id) = (id(workspace_id)?, id(user_id)?);
        self.db
            .read(move |conn| {
                chatbotx_db::repo::inbox_team_member::list_team_names_by_user_id(
                    conn,
                    workspace_id,
                    user_id,
                )
                .map_err(crate::repo_err)
            })
            .await
            .map_err(|e| SourceError::new(e.to_string()))
    }

    async fn list_tag_names_by_contact_id(&self, contact_id: &str) -> SourceResult<Vec<String>> {
        let contact_id = id(contact_id)?;
        self.read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT t."name" FROM "ContactToTag" ct JOIN "Tag" t ON t."id" = ct."tagId"
                   WHERE ct."contactId" = ?1 AND t."deletedAt" IS NULL ORDER BY t."name""#,
            )?;
            stmt.query_map(params![contact_id], |row| row.get(0))?
                .collect()
        })
        .await
    }

    async fn list_ad_ids(
        &self,
        _workspace_id: &str,
        _contact_id: &str,
        _limit: usize,
    ) -> SourceResult<Vec<String>> {
        Ok(Vec::new())
    }

    async fn find_recent_contact_inbox(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> SourceResult<Option<ContactInboxModel>> {
        let row = chatbotx_business::conversation::find_recent_contact_inbox(
            &self.db,
            id(workspace_id)?,
            id(contact_id)?,
        )
        .await
        .map_err(|e| SourceError::new(e.to_string()))?;
        Ok(row.map(contact_inbox_model))
    }

    async fn list_contact_note_texts(
        &self,
        _workspace_id: &str,
        contact_id: &str,
    ) -> SourceResult<Vec<String>> {
        let contact_id = id(contact_id)?;
        self.read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "text" FROM "ContactNote" WHERE "contactId" = ?1
                   ORDER BY "createdAt" DESC, "id" DESC"#,
            )?;
            stmt.query_map(params![contact_id], |row| row.get(0))?
                .collect()
        })
        .await
    }

    async fn find_message_by_id(
        &self,
        message_id: &str,
        _created_at: i64,
        workspace_id: &str,
    ) -> SourceResult<Option<MessageModel>> {
        let (message_id, workspace_id) = (id(message_id)?, id(workspace_id)?);
        self.read(move |conn| {
            conn.query_row(
                r#"SELECT "id", "sourceId", "text", "contentAttributes", "deletedAt", "createdAt"
                   FROM "Message" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![message_id, workspace_id],
                |row| {
                    let attributes: Option<String> = row.get(3)?;
                    Ok(MessageModel {
                        id: row.get::<_, i64>(0)?.to_string(),
                        source_id: row.get(1)?,
                        text: row.get(2)?,
                        content_attributes: attributes.and_then(|a| serde_json::from_str(&a).ok()),
                        deleted_at: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .optional()
        })
        .await
    }

    async fn list_incoming_texts_by_contact_inbox(
        &self,
        contact_inbox_id: &str,
        limit: usize,
        since_time: Option<i64>,
        workspace_id: &str,
    ) -> SourceResult<Vec<String>> {
        let (contact_inbox_id, workspace_id) = (id(contact_inbox_id)?, id(workspace_id)?);
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "text" FROM "Message" WHERE "contactInboxId" = ?1 AND "workspaceId" = ?2
                   AND "messageType" = 'incoming' AND "text" IS NOT NULL AND "text" != ''
                   AND "deletedAt" IS NULL AND (?3 IS NULL OR "createdAt" >= ?3)
                   ORDER BY "createdAt" DESC, "id" DESC LIMIT ?4"#,
            )?;
            stmt.query_map(
                params![contact_inbox_id, workspace_id, since_time, limit],
                |row| row.get(0),
            )?
            .collect()
        })
        .await
    }

    async fn list_last_messages(
        &self,
        conversation_id: &str,
        limit: usize,
        since_time: i64,
        workspace_id: &str,
    ) -> SourceResult<Vec<ChatMessage>> {
        let (conversation_id, workspace_id) = (id(conversation_id)?, id(workspace_id)?);
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "text", "senderType" FROM "Message" WHERE "conversationId" = ?1
                   AND "workspaceId" = ?2 AND "createdAt" >= ?3 AND "deletedAt" IS NULL
                   ORDER BY "createdAt" DESC, "id" DESC LIMIT ?4"#,
            )?;
            stmt.query_map(
                params![conversation_id, workspace_id, since_time, limit],
                |row| {
                    Ok(ChatMessage {
                        text: row.get(0)?,
                        sender_type: row.get(1)?,
                    })
                },
            )?
            .collect()
        })
        .await
    }

    async fn find_latest_incoming_message_with_attachments(
        &self,
        _conversation_id: &str,
        _since_time: i64,
        _workspace_id: &str,
    ) -> SourceResult<Option<MessageWithAttachments>> {
        Ok(None)
    }

    async fn ensure_attachment_stored(
        &self,
        _attachment: &Attachment,
        _workspace_id: &str,
    ) -> SourceResult<Option<String>> {
        Ok(None)
    }

    async fn resolve_attachment_media_url(
        &self,
        _request: &AttachmentMediaRequest,
    ) -> SourceResult<Option<String>> {
        Ok(None)
    }

    async fn find_latest_recorded_call(
        &self,
        _contact_id: &str,
    ) -> SourceResult<Option<CallRecord>> {
        Ok(None)
    }

    async fn find_latest_transcribed_call(
        &self,
        _contact_id: &str,
    ) -> SourceResult<Option<CallRecord>> {
        Ok(None)
    }

    async fn recording_signed_url(&self, _recording_path: &str) -> SourceResult<String> {
        Ok(String::new())
    }

    async fn tenant_settings(&self, _workspace_id: &str) -> SourceResult<TenantSettings> {
        let storage = std::env::var("NEXT_PUBLIC_STORAGE_URL").ok();
        let urls = chatbotx_business::contact::avatar::TenantUrls::from_app_url(
            &self.app_url,
            storage.as_deref(),
        );
        Ok(TenantSettings {
            app_url: urls.app_url,
            storage_url: urls.storage_url,
        })
    }

    async fn workspace_app_url(&self, _workspace_id: &str) -> SourceResult<String> {
        Ok(self.app_url.clone())
    }

    async fn find_inbox_with_integrations(
        &self,
        _inbox_id: &str,
    ) -> SourceResult<Option<InboxWithIntegrations>> {
        Ok(None)
    }

    async fn resolve_integration_context(
        &self,
        _workspace_id: &str,
        _contact_inbox: &ContactInboxModel,
    ) -> SourceResult<Value> {
        Ok(Value::Null)
    }

    async fn run_get_profile_snapshot(
        &self,
        _ctx: &Value,
        _source_id: &str,
    ) -> SourceResult<ContactProfileSnapshot> {
        Err(SourceError::new("getProfileSnapshot is not ported"))
    }

    async fn refresh_profile_snapshot(
        &self,
        _contact_inbox_id: &str,
        _inbox_id: &str,
        _snapshot: &ContactProfileSnapshot,
    ) -> SourceResult<()> {
        Ok(())
    }

    async fn messenger_user_inbox_link(
        &self,
        _auth: &Value,
        _user_id: &str,
    ) -> SourceResult<Option<String>> {
        Ok(None)
    }

    async fn messenger_post_message(
        &self,
        _auth: &Value,
        _post_id: &str,
    ) -> SourceResult<Option<String>> {
        Ok(None)
    }

    async fn instagram_post_caption(
        &self,
        _auth: &Value,
        _post_id: &str,
    ) -> SourceResult<Option<String>> {
        Ok(None)
    }

    /// `systemFieldService.create({ type: "me", payload })`.
    async fn create_me_system_field(&self, payload: &MeSystemFieldPayload) -> SourceResult<String> {
        let row_id = chatbotx_db::repo::new_id();
        let now = chatbotx_db::repo::now_ms();
        let payload = json!({
            "workspaceId": payload.workspace_id,
            "channel": payload.channel,
            "integrationId": payload.integration_id,
            "sourceId": payload.source_id,
            "contactInboxId": payload.contact_inbox_id,
            "conversationId": payload.conversation_id,
            "contactId": payload.contact_id,
        })
        .to_string();
        self.db
            .write(move |conn| {
                conn.execute(
                    r#"INSERT INTO "SystemField" ("id", "createdAt", "updatedAt", "type", "payload")
                       VALUES (?1, ?2, ?2, 'me', ?3)"#,
                    params![row_id, now, payload],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| SourceError::new(e.to_string()))?;
        Ok(row_id.to_string())
    }

    async fn find_appointment(
        &self,
        workspace_id: &str,
        appointment_id: &str,
    ) -> SourceResult<Option<Appointment>> {
        let (Ok(ws), Ok(id)) = (workspace_id.parse::<i64>(), appointment_id.parse::<i64>()) else {
            return Ok(None);
        };
        self.appointment(ws, move |conn| {
            chatbotx_db::repo::appointment::find_by(conn, ws, id, false)
        })
        .await
    }

    async fn find_latest_appointment_for_contact(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> SourceResult<Option<Appointment>> {
        let (Ok(ws), Ok(contact)) = (workspace_id.parse::<i64>(), contact_id.parse::<i64>()) else {
            return Ok(None);
        };
        self.appointment(ws, move |conn| {
            chatbotx_db::repo::appointment::find_latest_for_contact(conn, ws, contact)
        })
        .await
    }

    async fn latest_questionnaire_points(
        &self,
        _workspace_id: &str,
        _contact_id: &str,
    ) -> SourceResult<Option<f64>> {
        Ok(None)
    }

    async fn find_latest_lead(
        &self,
        _workspace_id: &str,
        _contact_id: &str,
    ) -> SourceResult<Option<LeadAdsLead>> {
        Ok(None)
    }

    async fn resolve_default_api_token(&self, _workspace_id: &str) -> SourceResult<String> {
        Ok(String::new())
    }

    async fn resolve_contact_avatar(
        &self,
        _workspace_id: &str,
        contact: &ContactModel,
        _contact_inbox: Option<&ContactInboxModel>,
    ) -> SourceResult<AvatarResolution> {
        Ok(match contact.avatar.as_deref().filter(|a| !a.is_empty()) {
            Some(avatar) => AvatarResolution::StorageKey(avatar.to_owned()),
            None => AvatarResolution::None,
        })
    }

    async fn leaderboard_referrer_name(
        &self,
        _contact_id: &str,
        _workspace_id: &str,
    ) -> SourceResult<Option<String>> {
        Ok(None)
    }

    async fn load_pipeline_variable_targets(
        &self,
        _workspace_id: &str,
        _contact_id: &str,
    ) -> SourceResult<Vec<PipelineVariableTarget>> {
        Ok(Vec::new())
    }

    async fn load_last_pipeline_item_id(
        &self,
        _workspace_id: &str,
        _contact_id: &str,
    ) -> SourceResult<Option<String>> {
        Ok(None)
    }

    async fn resolve_coupon_variable(
        &self,
        _workspace_id: &str,
        _contact_id: &str,
        _topic_id: &str,
    ) -> SourceResult<String> {
        Ok(String::new())
    }
}

/// The `resolveContactVariablesDeep` dependencies of the worker.
pub struct VariableDeps {
    pub sources: DbSources,
    /// `withCache`.
    pub cache: chatbotx_kv::cache::Cache,
    /// Token signing (`ENCRYPTION_KEY`).
    pub encryption: chatbotx_core::encryption::Encryption,
}

impl VariableDeps {
    /// From the process env; `None` without a valid `ENCRYPTION_KEY`.
    pub fn from_env(db: Arc<Db>, app_url: String) -> Option<VariableDeps> {
        let keys = chatbotx_core::encryption::Keys::from_env(&chatbotx_core::env::ProcessEnv)
            .inspect_err(|err| tracing::warn!(err = %err, "Flow variables resolve as spintax only: no ENCRYPTION_KEY"))
            .ok()?;
        Some(VariableDeps {
            sources: DbSources { db, app_url },
            cache: chatbotx_kv::cache::Cache::new(Arc::new(chatbotx_kv::SystemClock)),
            encryption: chatbotx_core::encryption::Encryption::new(keys),
        })
    }

    /// `resolveContactVariablesDeep(contactId, step, { contactInbox,
    /// conversation, appointmentId, sentByUserId }, { spintax: true })`.
    pub async fn resolve_step(
        &self,
        conversation: &Conversation,
        contact_inbox: &ContactInbox,
        step: &Value,
        appointment_id: Option<String>,
        sent_by_user_id: Option<String>,
    ) -> Result<Value, String> {
        self.resolve(
            conversation,
            contact_inbox,
            step,
            appointment_id,
            sent_by_user_id,
            true,
        )
        .await
    }

    /// `resolveContactVariablesDeep` with the spintax option.
    pub async fn resolve(
        &self,
        conversation: &Conversation,
        contact_inbox: &ContactInbox,
        step: &Value,
        appointment_id: Option<String>,
        sent_by_user_id: Option<String>,
        spintax: bool,
    ) -> Result<Value, String> {
        use chatbotx_core::variables::Variables;
        use chatbotx_core::variables::contact_variable::ContactInboxArg;
        use chatbotx_core::variables::deep_replace::{DeepOptions, DeepSource};
        let clock = chatbotx_kv::SystemClock;
        Variables::new(&self.sources, &self.cache, &self.encryption, &clock)
            .resolve_contact_variables_deep(
                &conversation.contact_id.to_string(),
                step,
                DeepSource {
                    contact_inbox: ContactInboxArg::Model(contact_inbox_model(
                        contact_inbox.clone(),
                    )),
                    conversation: Some(conversation_model(conversation)),
                    workspace: None,
                    appointment_id,
                    sent_by_user_id,
                },
                DeepOptions {
                    spintax,
                    pick: None,
                },
            )
            .await
            .map_err(|err| err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolves_contact_custom_and_bot_fields_from_the_database() {
        let dir = std::env::temp_dir().join(format!(
            "chatbotx-worker-vars-{}-{}",
            std::process::id(),
            chatbotx_db::repo::new_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Arc::new(Db::open(dir.join("t.db")).unwrap());
        db.migrate().await.unwrap();
        db.write(|tx| {
            for sql in [
                r#"INSERT INTO "User" ("id", "email") VALUES (1, 'u@example.test')"#,
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (2, 'WS', 1)"#,
                r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (3, 'Ada', 2)"#,
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (4, 2, 'Web', 'webchat', 'w')"#,
                r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId") VALUES (5, 3, 3, 4, 'webchat', 'webchat', 's')"#,
                r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (6, 2, 3)"#,
                r#"INSERT INTO "CustomField" ("id", "name", "type", "workspaceId") VALUES (7, 'plan', 'shortText', 2)"#,
                r#"INSERT INTO "ContactCustomField" ("id", "value", "contactId", "customFieldId") VALUES (8, 'gold', 3, 7)"#,
            ] {
                tx.execute(sql, [])?;
            }
            Ok(())
        })
        .await
        .unwrap();
        let (conversation, contact_inbox) = db
            .read(|conn| {
                Ok((
                    conn.query_row(
                        &format!(r#"{} WHERE "id" = 6"#, Conversation::select_sql()),
                        [],
                        Conversation::from_row,
                    )?,
                    conn.query_row(
                        &format!(r#"{} WHERE "id" = 5"#, ContactInbox::select_sql()),
                        [],
                        ContactInbox::from_row,
                    )?,
                ))
            })
            .await
            .unwrap();
        let deps = VariableDeps {
            sources: DbSources {
                db,
                app_url: "http://app.test".into(),
            },
            cache: chatbotx_kv::cache::Cache::new(Arc::new(chatbotx_kv::SystemClock)),
            encryption: chatbotx_core::encryption::Encryption::new(
                chatbotx_core::encryption::Keys::new(&"ab".repeat(32)).unwrap(),
            ),
        };
        let step =
            json!({ "stepType": "sendText", "text": "Hi {{first_name}}, plan {{plan}} {a|a}" });
        let resolved = deps
            .resolve_step(&conversation, &contact_inbox, &step, None, None)
            .await
            .unwrap();
        assert_eq!(resolved["text"], "Hi Ada, plan gold a");
        let _ = std::fs::remove_dir_all(dir);
    }
}
