//! `helpers/*.ts` except `integration-fields.ts`: assignee and sender
//! names, contact tags / notes / ad ids, the message window, chat history,
//! last input, last call, queued messages, public storage URLs and the
//! leaderboard referrer.

use super::sources::{AttachmentMediaRequest, SourceError, VariableSources};
use super::system_field::CONTACT_AD_IDS_LIMIT;
use super::{
    Attachment, ConversationModel, MemberUser, MessageWithAttachments, VariableContext,
    VariableError, Variables,
};
use crate::utils::storage::get_public_file_url;

/// `ONE_YEAR_MS`: the chat-history lookback.
pub const ONE_YEAR_MS: i64 = 365 * 24 * 60 * 60 * 1000;

const HOUR_MS: i64 = 60 * 60 * 1000;

/// `getSafeSinceTime(time, bufferMs)`: `time - bufferMs` floored to the
/// hour (the process zone is UTC); `None` without a time.
pub fn get_safe_since_time(time: Option<i64>, buffer_ms: i64) -> Option<i64> {
    let shifted = time? - buffer_ms;
    Some(shifted.div_euclid(HOUR_MS) * HOUR_MS)
}

/// `toPublicStorageUrl(path, baseUrl)` (business `utils`): an absolute
/// `http(s)` URL as it is, else the key under the public base URL.
pub fn public_storage_url(path: &str, base_url: &str) -> Result<String, SourceError> {
    let lower = path
        .get(..8)
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Ok(path.to_owned());
    }
    get_public_file_url(path, base_url)
        .map_err(|err| SourceError::new(format!("Invalid URL: {err}")))
}

/// `ContactMessageWindow`.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactMessageWindow {
    pub conversation: ConversationModel,
    pub since_time: i64,
}

type Res<T> = Result<T, VariableError>;

impl<S: VariableSources> Variables<'_, S> {
    async fn resolve_assignee_member(
        &self,
        contact_id: &str,
        workspace_id: &str,
    ) -> Res<Option<MemberUser>> {
        let conversation = self
            .sources
            .find_conversation_by_contact(contact_id, Some(workspace_id))
            .await?;
        let Some(user_id) = conversation.and_then(|c| c.assigned_user_id) else {
            return Ok(None);
        };
        Ok(self
            .sources
            .find_member_user(workspace_id, &user_id)
            .await?)
    }

    /// `resolveAssigneeName`: the conversation assignee's name (never the
    /// email).
    pub async fn resolve_assignee_name(
        &self,
        contact_id: &str,
        workspace_id: &str,
    ) -> Res<Option<String>> {
        Ok(self
            .resolve_assignee_member(contact_id, workspace_id)
            .await?
            .and_then(|m| m.name))
    }

    /// `resolveAssigneeEmail`.
    pub async fn resolve_assignee_email(
        &self,
        contact_id: &str,
        workspace_id: &str,
    ) -> Res<Option<String>> {
        Ok(self
            .resolve_assignee_member(contact_id, workspace_id)
            .await?
            .and_then(|m| m.email))
    }

    /// `resolveAssigneeId`.
    pub async fn resolve_assignee_id(
        &self,
        contact_id: &str,
        workspace_id: &str,
    ) -> Res<Option<String>> {
        Ok(self
            .resolve_assignee_member(contact_id, workspace_id)
            .await?
            .map(|m| m.id))
    }

    /// `getAssignedTeamName`.
    pub async fn get_assigned_team_name(&self, contact_id: &str) -> Res<Option<String>> {
        let conversation = self
            .sources
            .find_conversation_by_contact(contact_id, None)
            .await?;
        let Some(team_id) = conversation.and_then(|c| c.assigned_inbox_team_id) else {
            return Ok(None);
        };
        Ok(self.sources.find_inbox_team_name(&team_id).await?)
    }

    /// `resolveSenderName`: the inbox user who sent the running flow.
    pub async fn resolve_sender_name(
        &self,
        workspace_id: &str,
        user_id: &str,
    ) -> Res<Option<String>> {
        Ok(self
            .sources
            .find_member_user(workspace_id, user_id)
            .await?
            .and_then(|m| m.name))
    }

    /// `listSenderTeamNames`: `"Sales, Support"`, `None` for no team.
    pub async fn list_sender_team_names(
        &self,
        workspace_id: &str,
        user_id: &str,
    ) -> Res<Option<String>> {
        let names = self
            .sources
            .list_team_names_by_user_id(workspace_id, user_id)
            .await?;
        Ok((!names.is_empty()).then(|| names.join(", ")))
    }

    /// `listContactTagsString`.
    pub async fn list_contact_tags_string(&self, contact_id: &str) -> Res<String> {
        Ok(self
            .sources
            .list_tag_names_by_contact_id(contact_id)
            .await?
            .join(", "))
    }

    /// `listContactAdIdsString`: the ad ids, oldest first, capped at
    /// [`CONTACT_AD_IDS_LIMIT`]; `None` when there is none.
    pub async fn list_contact_ad_ids_string(
        &self,
        contact_id: &str,
        workspace_id: &str,
    ) -> Res<Option<String>> {
        let ids = self
            .sources
            .list_ad_ids(workspace_id, contact_id, CONTACT_AD_IDS_LIMIT)
            .await?;
        Ok((!ids.is_empty()).then(|| ids.join(", ")))
    }

    /// `getFirstContactAdId`.
    pub async fn get_first_contact_ad_id(
        &self,
        contact_id: &str,
        workspace_id: &str,
    ) -> Res<Option<String>> {
        Ok(self
            .sources
            .list_ad_ids(workspace_id, contact_id, 1)
            .await?
            .into_iter()
            .next())
    }

    /// `findPrimaryContactChannel`.
    pub async fn find_primary_contact_channel(
        &self,
        contact_id: &str,
        workspace_id: &str,
    ) -> Res<Option<String>> {
        Ok(self
            .sources
            .find_recent_contact_inbox(workspace_id, contact_id)
            .await?
            .map(|ci| ci.channel))
    }

    /// `listContactNotesString`.
    pub async fn list_contact_notes_string(
        &self,
        contact_id: &str,
        workspace_id: &str,
    ) -> Res<String> {
        Ok(self
            .sources
            .list_contact_note_texts(workspace_id, contact_id)
            .await?
            .join("\n"))
    }

    /// `getLatestContactNoteString`.
    pub async fn get_latest_contact_note_string(
        &self,
        contact_id: &str,
        workspace_id: &str,
    ) -> Res<Option<String>> {
        Ok(self
            .sources
            .list_contact_note_texts(workspace_id, contact_id)
            .await?
            .into_iter()
            .next())
    }

    /// `getContactLastCallRecording`: a presigned URL (15 minutes), never
    /// the public storage URL; a presign failure is logged and `None`.
    pub async fn get_contact_last_call_recording(&self, contact_id: &str) -> Res<Option<String>> {
        let Some(call) = self.sources.find_latest_recorded_call(contact_id).await? else {
            return Ok(None);
        };
        let Some(path) = call.recording_path.as_deref().filter(|p| !p.is_empty()) else {
            return Ok(None);
        };
        match self.sources.recording_signed_url(path).await {
            Ok(url) => Ok(Some(url)),
            Err(err) => {
                tracing::error!(
                    err = %err,
                    contact_id,
                    whatsapp_call_id = %call.id,
                    "Failed to presign the contact's last call recording"
                );
                Ok(None)
            }
        }
    }

    /// `getContactLastCallTranscript`.
    pub async fn get_contact_last_call_transcript(&self, contact_id: &str) -> Res<Option<String>> {
        Ok(self
            .sources
            .find_latest_transcribed_call(contact_id)
            .await?
            .and_then(|c| c.transcript))
    }

    /// `getQueuedMessages`: the incoming texts since the last outbound
    /// message (else first interaction, else inbox creation), oldest first.
    pub async fn get_queued_messages(&self, context: &VariableContext) -> Res<Option<String>> {
        let Some(contact_inbox) = &context.contact_inbox else {
            return Ok(None);
        };
        let since_time = contact_inbox
            .last_outbound_message_at
            .or(contact_inbox.first_interaction_at)
            .or(contact_inbox.created_at);
        let mut messages = self
            .sources
            .list_incoming_texts_by_contact_inbox(
                &contact_inbox.id,
                50,
                since_time,
                &context.contact.workspace_id,
            )
            .await?;
        if messages.is_empty() {
            return Ok(None);
        }
        messages.reverse();
        Ok(Some(messages.join("\n")))
    }

    /// `resolveContactMessageWindow`: the conversation the contact is
    /// latest active in, and one year before its last activity.
    pub async fn resolve_contact_message_window(
        &self,
        contact_id: &str,
    ) -> Res<Option<ContactMessageWindow>> {
        let Some(conversation) = self
            .sources
            .find_latest_conversation_by_contact(contact_id)
            .await?
        else {
            return Ok(None);
        };
        let anchor = conversation.last_activity_at.or(conversation.created_at);
        Ok(
            get_safe_since_time(anchor, ONE_YEAR_MS).map(|since_time| ContactMessageWindow {
                conversation,
                since_time,
            }),
        )
    }

    /// `listLastMessages`: one line per message (`Attached File` without
    /// text), `User:` / `Admin:` prefixed with `include_detail`.
    pub async fn list_last_messages(
        &self,
        conversation_id: &str,
        workspace_id: &str,
        limit: usize,
        include_detail: bool,
        since_time: i64,
    ) -> Res<String> {
        let messages = self
            .sources
            .list_last_messages(conversation_id, limit, since_time, workspace_id)
            .await?;
        Ok(messages
            .iter()
            .map(|m| {
                let text = m.text.as_deref().unwrap_or("Attached File");
                if include_detail {
                    let sender = if m.sender_type == "user" {
                        "User"
                    } else {
                        "Admin"
                    };
                    format!("{sender}: {text}")
                } else {
                    text.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// `getChatHistory`.
    pub async fn get_chat_history(
        &self,
        contact_id: &str,
        limit: usize,
        include_detail: bool,
    ) -> Res<Option<String>> {
        let Some(window) = self.resolve_contact_message_window(contact_id).await? else {
            return Ok(None);
        };
        Ok(Some(
            self.list_last_messages(
                &window.conversation.id,
                &window.conversation.workspace_id,
                limit,
                include_detail,
                window.since_time,
            )
            .await?,
        ))
    }

    async fn find_latest_input_message(
        &self,
        contact_id: &str,
    ) -> Res<Option<(ConversationModel, MessageWithAttachments)>> {
        let Some(window) = self.resolve_contact_message_window(contact_id).await? else {
            return Ok(None);
        };
        let message = self
            .sources
            .find_latest_incoming_message_with_attachments(
                &window.conversation.id,
                window.since_time,
                &window.conversation.workspace_id,
            )
            .await?;
        Ok(message.map(|m| (window.conversation, m)))
    }

    async fn to_proxy_attachment_url(
        &self,
        attachment: &Attachment,
        message: &MessageWithAttachments,
        workspace_id: &str,
    ) -> Result<Option<String>, SourceError> {
        let Some(contact_inbox) = self
            .sources
            .find_contact_inbox(&message.contact_inbox_id)
            .await?
        else {
            return Ok(None);
        };
        self.sources
            .resolve_attachment_media_url(&AttachmentMediaRequest {
                attachment_id: attachment.id.clone(),
                workspace_id: workspace_id.to_owned(),
                origin_path: attachment.origin_path.clone(),
                channel: contact_inbox.channel,
                message_created_at: attachment.message_created_at,
            })
            .await
    }

    /// `toStoredAttachmentUrl`: store a lazily-ingested attachment first;
    /// one that is gone is `None`, one not stored in time its media proxy
    /// URL. Never fails.
    async fn to_stored_attachment_url(
        &self,
        attachment: &Attachment,
        message: &MessageWithAttachments,
        workspace_id: &str,
    ) -> Option<String> {
        let stored = async {
            match self
                .sources
                .ensure_attachment_stored(attachment, workspace_id)
                .await?
            {
                Some(key) => self.to_public_storage_url(Some(&key), workspace_id).await,
                None => Ok(None),
            }
        };
        match stored.await {
            Ok(url) => return url,
            Err(err) => tracing::warn!(
                err = %err,
                attachment_id = %attachment.id,
                workspace_id,
                "last_input: attachment not stored in time; using its media proxy URL"
            ),
        }
        match self
            .to_proxy_attachment_url(attachment, message, workspace_id)
            .await
        {
            Ok(url) => url,
            Err(err) => {
                tracing::warn!(
                    err = %err,
                    attachment_id = %attachment.id,
                    workspace_id,
                    "last_input: attachment media proxy URL could not be built"
                );
                None
            }
        }
    }

    /// `getContactLastInput`: the latest incoming message's attachment URL,
    /// else its text when it is a text message.
    pub async fn get_contact_last_input(&self, contact_id: &str) -> Res<Option<String>> {
        let Some((conversation, message)) = self.find_latest_input_message(contact_id).await?
        else {
            return Ok(None);
        };
        if let Some(attachment) = message.attachments.first() {
            return Ok(self
                .to_stored_attachment_url(attachment, &message, &conversation.workspace_id)
                .await);
        }
        if message.content_type == "text" {
            return Ok(message.text);
        }
        Ok(None)
    }

    /// `getContactLastTextInput`: the latest incoming message's text when it
    /// is plain text, never a media URL.
    pub async fn get_contact_last_text_input(&self, contact_id: &str) -> Res<Option<String>> {
        let Some((_, message)) = self.find_latest_input_message(contact_id).await? else {
            return Ok(None);
        };
        if message.content_type == "text" && message.attachments.is_empty() {
            return Ok(message.text);
        }
        Ok(None)
    }

    /// `getContactLastInputType`: the first attachment's file type, else the
    /// content type.
    pub async fn get_contact_last_input_type(&self, contact_id: &str) -> Res<Option<String>> {
        let Some((_, message)) = self.find_latest_input_message(contact_id).await? else {
            return Ok(None);
        };
        Ok(Some(match message.attachments.first() {
            Some(attachment) => attachment.file_type.clone(),
            None => message.content_type,
        }))
    }

    /// `toPublicStorageUrl(path, workspaceId)` (`helpers/storage-url.ts`):
    /// `None` without a path, else the key under the tenant's storage URL.
    pub async fn to_public_storage_url(
        &self,
        path: Option<&str>,
        workspace_id: &str,
    ) -> Result<Option<String>, SourceError> {
        let Some(path) = path.filter(|p| !p.is_empty()) else {
            return Ok(None);
        };
        let settings = self.sources.tenant_settings(workspace_id).await?;
        public_storage_url(path, &settings.storage_url).map(Some)
    }

    /// `getLeaderboardReferrerName`.
    pub async fn get_leaderboard_referrer_name(
        &self,
        contact_id: &str,
        workspace_id: &str,
    ) -> Res<Option<String>> {
        Ok(self
            .sources
            .leaderboard_referrer_name(contact_id, workspace_id)
            .await?)
    }
}
