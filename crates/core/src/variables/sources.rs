//! [`VariableSources`]: every database, business-service and provider call
//! the variable resolvers make. The original imports these from
//! `@chatbotx.io/business`, `@chatbotx.io/database`, the channel registry and
//! the integrations; core cannot depend on `api`/`server`, so the server
//! implements this trait and tests use a fake.

use std::fmt;
use std::future::Future;

use super::models::{
    Appointment, Attachment, BotFieldRow, CallRecord, ChatMessage, ContactCustomFieldValue,
    ContactInboxModel, ContactModel, ContactProfileSnapshot, ConversationModel,
    InboxWithIntegrations, LeadAdsLead, MemberUser, MessageModel, MessageWithAttachments,
    PipelineVariableTarget, TenantSettings, WorkspaceModel,
};

/// A failed source call (a rejected promise in the original).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceError(pub String);

impl SourceError {
    pub fn new(message: impl Into<String>) -> SourceError {
        SourceError(message.into())
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SourceError {}

/// The result of one source call.
pub type SourceResult<T> = Result<T, SourceError>;

/// What `resolveContactAvatarUrl` (business `media`) resolves a contact's
/// picture to, before the storage key is made public.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvatarResolution {
    /// No picture.
    None,
    /// A storage key (or an absolute URL), finalized with
    /// `toPublicStorageUrl(key, workspaceId)`.
    StorageKey(String),
    /// A final URL (the media proxy URL of a picture not mirrored yet).
    Url(String),
}

/// The `{ kind: "attachment" }` input of `resolveMediaUrl`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentMediaRequest {
    pub attachment_id: String,
    pub workspace_id: String,
    pub origin_path: String,
    pub channel: String,
    pub message_created_at: Option<i64>,
}

/// The `me` system field row payload (`systemFieldService.create`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeSystemFieldPayload {
    pub workspace_id: String,
    pub channel: String,
    pub integration_id: String,
    pub source_id: String,
    pub contact_inbox_id: String,
    pub conversation_id: Option<String>,
    pub contact_id: String,
}

/// The data sources of the variable resolvers. Each method names the
/// original call it replaces.
pub trait VariableSources: Send + Sync {
    /// `db.query.contactModel.findFirst({ where: { id } })`.
    fn find_contact(
        &self,
        contact_id: &str,
    ) -> impl Future<Output = SourceResult<Option<ContactModel>>> + Send;

    /// `db.query.contactInboxModel.findFirst({ where: { id } })` /
    /// `contactInboxService.findBy({ where: { id } })`.
    fn find_contact_inbox(
        &self,
        id: &str,
    ) -> impl Future<Output = SourceResult<Option<ContactInboxModel>>> + Send;

    /// `workspaceService.find({ where: { id } })`.
    fn find_workspace(
        &self,
        workspace_id: &str,
    ) -> impl Future<Output = SourceResult<Option<WorkspaceModel>>> + Send;

    /// `db.query.contactCustomFieldModel.findMany({ where: { contactId }, with: { customField: true } })`
    /// mapped to `{ key: name, type, value, description }`.
    fn list_contact_custom_fields(
        &self,
        contact_id: &str,
    ) -> impl Future<Output = SourceResult<Vec<ContactCustomFieldValue>>> + Send;

    /// `db.query.botFieldModel.findMany({ where: { workspaceId } })`.
    fn list_bot_fields(
        &self,
        workspace_id: &str,
    ) -> impl Future<Output = SourceResult<Vec<BotFieldRow>>> + Send;

    /// `conversationService.findBy({ where: { contactId, workspaceId? } })`.
    fn find_conversation_by_contact(
        &self,
        contact_id: &str,
        workspace_id: Option<&str>,
    ) -> impl Future<Output = SourceResult<Option<ConversationModel>>> + Send;

    /// `conversationService.findDMByContact({ workspaceId, contactId })`.
    fn find_dm_conversation_by_contact(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> impl Future<Output = SourceResult<Option<ConversationModel>>> + Send;

    /// `conversationService.findLatestByContact({ contactId })`.
    fn find_latest_conversation_by_contact(
        &self,
        contact_id: &str,
    ) -> impl Future<Output = SourceResult<Option<ConversationModel>>> + Send;

    /// `workspaceMemberService.findWithUserByWorkspaceIdAndUserId(...).user`.
    fn find_member_user(
        &self,
        workspace_id: &str,
        user_id: &str,
    ) -> impl Future<Output = SourceResult<Option<MemberUser>>> + Send;

    /// `db.query.inboxTeamModel.findFirst({ where: { id } }).name`.
    fn find_inbox_team_name(
        &self,
        team_id: &str,
    ) -> impl Future<Output = SourceResult<Option<String>>> + Send;

    /// `inboxTeamService.listTeamNamesByUserId({ workspaceId, userId })`.
    fn list_team_names_by_user_id(
        &self,
        workspace_id: &str,
        user_id: &str,
    ) -> impl Future<Output = SourceResult<Vec<String>>> + Send;

    /// `tagService.listByContactId({ contactId })`, the tag names.
    fn list_tag_names_by_contact_id(
        &self,
        contact_id: &str,
    ) -> impl Future<Output = SourceResult<Vec<String>>> + Send;

    /// `contactReferralEventService.listAdIds({ workspaceId, contactId, limit })`.
    fn list_ad_ids(
        &self,
        workspace_id: &str,
        contact_id: &str,
        limit: usize,
    ) -> impl Future<Output = SourceResult<Vec<String>>> + Send;

    /// `contactInboxService.findRecentByContactId({ workspaceId, contactId })`.
    fn find_recent_contact_inbox(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> impl Future<Output = SourceResult<Option<ContactInboxModel>>> + Send;

    /// `contactNoteService.listByContactId({ workspaceId, contactId })`, the
    /// note texts, newest first.
    fn list_contact_note_texts(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> impl Future<Output = SourceResult<Vec<String>>> + Send;

    /// `messageService.findById({ id, createdAt, workspaceId })`.
    fn find_message_by_id(
        &self,
        id: &str,
        created_at: i64,
        workspace_id: &str,
    ) -> impl Future<Output = SourceResult<Option<MessageModel>>> + Send;

    /// `messageService.listIncomingTextsByContactInbox({ contactInboxId, limit, sinceTime, workspaceId })`,
    /// newest first.
    fn list_incoming_texts_by_contact_inbox(
        &self,
        contact_inbox_id: &str,
        limit: usize,
        since_time: Option<i64>,
        workspace_id: &str,
    ) -> impl Future<Output = SourceResult<Vec<String>>> + Send;

    /// `messageService.listLastMessages({ conversationId, limit, sinceTime, workspaceId })`.
    fn list_last_messages(
        &self,
        conversation_id: &str,
        limit: usize,
        since_time: i64,
        workspace_id: &str,
    ) -> impl Future<Output = SourceResult<Vec<ChatMessage>>> + Send;

    /// `messageService.findLatestIncomingMessageWithAttachments({ conversationId, sinceTime, workspaceId })`.
    fn find_latest_incoming_message_with_attachments(
        &self,
        conversation_id: &str,
        since_time: i64,
        workspace_id: &str,
    ) -> impl Future<Output = SourceResult<Option<MessageWithAttachments>>> + Send;

    /// `ensureAttachmentStored({ attachmentId, workspaceId, messageCreatedAt, originPath })`:
    /// the stored object key, `None` when the attachment is gone.
    fn ensure_attachment_stored(
        &self,
        attachment: &Attachment,
        workspace_id: &str,
    ) -> impl Future<Output = SourceResult<Option<String>>> + Send;

    /// `resolveMediaUrl({ kind: "attachment", ... }, finalize)`: the media
    /// proxy URL of an attachment.
    fn resolve_attachment_media_url(
        &self,
        request: &AttachmentMediaRequest,
    ) -> impl Future<Output = SourceResult<Option<String>>> + Send;

    /// `whatsappCallRepository.findLatestRecordedByContactId`.
    fn find_latest_recorded_call(
        &self,
        contact_id: &str,
    ) -> impl Future<Output = SourceResult<Option<CallRecord>>> + Send;

    /// `whatsappCallRepository.findLatestTranscribedByContactId`.
    fn find_latest_transcribed_call(
        &self,
        contact_id: &str,
    ) -> impl Future<Output = SourceResult<Option<CallRecord>>> + Send;

    /// `callRecordingService.getRecordingSignedUrl({ recordingPath })`.
    fn recording_signed_url(
        &self,
        recording_path: &str,
    ) -> impl Future<Output = SourceResult<String>> + Send;

    /// `resolveTenantSettings({ workspaceId })`.
    fn tenant_settings(
        &self,
        workspace_id: &str,
    ) -> impl Future<Output = SourceResult<TenantSettings>> + Send;

    /// `resolveWorkspaceAppUrl({ workspaceId })`.
    fn workspace_app_url(
        &self,
        workspace_id: &str,
    ) -> impl Future<Output = SourceResult<String>> + Send;

    /// `inboxService.findWithIntegrationsById({ id })`.
    fn find_inbox_with_integrations(
        &self,
        inbox_id: &str,
    ) -> impl Future<Output = SourceResult<Option<InboxWithIntegrations>>> + Send;

    /// `resolveIntegrationContextFromContactInbox({ workspaceId, contactInbox })`:
    /// the channel handler context (opaque to the variables).
    fn resolve_integration_context(
        &self,
        workspace_id: &str,
        contact_inbox: &ContactInboxModel,
    ) -> impl Future<Output = SourceResult<serde_json::Value>> + Send;

    /// `integration.runChannelHandler("contact", "getProfileSnapshot", { ctx, data: { sourceId } })`.
    fn run_get_profile_snapshot(
        &self,
        ctx: &serde_json::Value,
        source_id: &str,
    ) -> impl Future<Output = SourceResult<ContactProfileSnapshot>> + Send;

    /// `contactInboxService.refreshProfileSnapshot({ contactInboxId, inboxId, snapshot })`.
    fn refresh_profile_snapshot(
        &self,
        contact_inbox_id: &str,
        inbox_id: &str,
        snapshot: &ContactProfileSnapshot,
    ) -> impl Future<Output = SourceResult<()>> + Send;

    /// Messenger `getUserInboxLink({ ctx: { auth }, input: { userId } })`.
    fn messenger_user_inbox_link(
        &self,
        auth: &serde_json::Value,
        user_id: &str,
    ) -> impl Future<Output = SourceResult<Option<String>>> + Send;

    /// Messenger `getPostDetails({ ctx: { auth }, input: { postId } }).message`.
    fn messenger_post_message(
        &self,
        auth: &serde_json::Value,
        post_id: &str,
    ) -> impl Future<Output = SourceResult<Option<String>>> + Send;

    /// Instagram `getPostDetails({ ctx: { auth }, input: { postId } }).caption`.
    fn instagram_post_caption(
        &self,
        auth: &serde_json::Value,
        post_id: &str,
    ) -> impl Future<Output = SourceResult<Option<String>>> + Send;

    /// `systemFieldService.create({ type: "me", payload })`: the row id.
    fn create_me_system_field(
        &self,
        payload: &MeSystemFieldPayload,
    ) -> impl Future<Output = SourceResult<String>> + Send;

    /// `appointmentService.findBy({ workspaceId, id })`.
    fn find_appointment(
        &self,
        workspace_id: &str,
        id: &str,
    ) -> impl Future<Output = SourceResult<Option<Appointment>>> + Send;

    /// `appointmentService.findLatestForContact({ workspaceId, contactId })`.
    fn find_latest_appointment_for_contact(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> impl Future<Output = SourceResult<Option<Appointment>>> + Send;

    /// `questionnaireSubmissionService.findLatestCompletedForContact(...)?.totalPoints`
    /// (`None` without a submission).
    fn latest_questionnaire_points(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> impl Future<Output = SourceResult<Option<f64>>> + Send;

    /// `facebookLeadAdsLeadService.findLatestByContact({ workspaceId, contactId })`.
    fn find_latest_lead(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> impl Future<Output = SourceResult<Option<LeadAdsLead>>> + Send;

    /// `workspaceApiTokenService.resolveDefaultTokenPlaintext({ workspaceId })`.
    fn resolve_default_api_token(
        &self,
        workspace_id: &str,
    ) -> impl Future<Output = SourceResult<String>> + Send;

    /// `resolveContactAvatarUrl(...)` with `ensureContactAvatarMirrored`,
    /// before the storage key is finalized.
    fn resolve_contact_avatar(
        &self,
        workspace_id: &str,
        contact: &ContactModel,
        contact_inbox: Option<&ContactInboxModel>,
    ) -> impl Future<Output = SourceResult<AvatarResolution>> + Send;

    /// `findLeaderboardReferrerName({ contactId, workspaceId })`.
    fn leaderboard_referrer_name(
        &self,
        contact_id: &str,
        workspace_id: &str,
    ) -> impl Future<Output = SourceResult<Option<String>>> + Send;

    /// `loadPipelineVariableTargets({ workspaceId, contactId })`.
    fn load_pipeline_variable_targets(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> impl Future<Output = SourceResult<Vec<PipelineVariableTarget>>> + Send;

    /// `loadLastPipelineItemId({ workspaceId, contactId })`.
    fn load_last_pipeline_item_id(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> impl Future<Output = SourceResult<Option<String>>> + Send;

    /// `couponService.resolveCouponVariable({ workspaceId, contactId, topicId })`.
    fn resolve_coupon_variable(
        &self,
        workspace_id: &str,
        contact_id: &str,
        topic_id: &str,
    ) -> impl Future<Output = SourceResult<String>> + Send;
}
