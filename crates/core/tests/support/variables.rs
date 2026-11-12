//! A fake [`VariableSources`] for the ported `packages/variables` tests:
//! each `vi.fn()` mock becomes a configurable answer in [`FakeState`], and
//! every call is recorded as `name(arg, arg, ...)` so the
//! `toHaveBeenCalledWith` assertions can be checked.
#![allow(clippy::redundant_closure)] // P3.1 orchestrator: style lint in locked ported test
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chatbotx_core::encryption::{Encryption, Keys};
use chatbotx_core::variables::*;
use chatbotx_core::{Clock, TestClock};
use chatbotx_kv::cache::Cache;
use serde_json::Value;

use super::TEST_ENCRYPTION_KEY;

type Answer<T> = Result<T, String>;

/// What `ensureAttachmentStored` does.
#[derive(Debug, Clone, Default)]
pub enum EnsureStored {
    /// Returns the attachment's own `originPath` (the test's pass-through).
    #[default]
    PassThrough,
    /// Returns this answer.
    Answer(Answer<Option<String>>),
}

/// The configurable answers of [`Fake`]. `Default` mirrors a bare `vi.fn()`
/// (resolves `undefined`, i.e. `None` / empty).
#[derive(Debug, Clone)]
pub struct FakeState {
    pub contact: Option<ContactModel>,
    pub contact_inbox: Answer<Option<ContactInboxModel>>,
    pub workspace: Option<WorkspaceModel>,
    pub custom_fields: Vec<ContactCustomFieldValue>,
    pub bot_fields: Vec<BotFieldRow>,
    pub conversation_by_contact: Option<ConversationModel>,
    pub dm_conversation: Option<ConversationModel>,
    pub latest_conversation: Option<ConversationModel>,
    pub member: Option<MemberUser>,
    pub team_name: Option<String>,
    pub team_names: Vec<String>,
    pub tags: Vec<String>,
    pub ad_ids: Vec<String>,
    pub recent_contact_inbox: Option<ContactInboxModel>,
    pub notes: Vec<String>,
    pub message: Option<MessageModel>,
    pub incoming_texts: Vec<String>,
    pub last_messages: Vec<ChatMessage>,
    pub latest_input: Option<MessageWithAttachments>,
    pub ensure_stored: EnsureStored,
    pub attachment_media_url: Answer<Option<String>>,
    pub recorded_call: Option<CallRecord>,
    pub transcribed_call: Option<CallRecord>,
    pub signed_url: Answer<String>,
    pub tenant: TenantSettings,
    pub app_url: String,
    pub inbox: Answer<Option<InboxWithIntegrations>>,
    pub integration_ctx: Answer<Value>,
    pub snapshot: Answer<ContactProfileSnapshot>,
    pub refresh_snapshot: Answer<()>,
    pub user_inbox_link: Answer<Option<String>>,
    pub post_message: Answer<Option<String>>,
    pub post_caption: Answer<Option<String>>,
    pub me_row_id: String,
    pub appointment: Option<Appointment>,
    pub latest_appointment: Option<Appointment>,
    pub points: Option<f64>,
    pub lead: Option<LeadAdsLead>,
    pub api_token: Answer<String>,
    /// `ensureContactAvatarMirrored`'s `{ avatar }` (`None` = contact missing).
    pub mirrored_avatar: Option<String>,
    /// The `system-fields-exhaustive` avatar mock (no `ensureMirrored`):
    /// a messenger / instagram inbox resolves to its media proxy URL.
    pub avatar_proxy: bool,
    pub referrer: Option<String>,
    pub pipeline_targets: Answer<Vec<PipelineVariableTarget>>,
    pub last_item_id: Option<String>,
    pub coupon: String,
}

impl Default for FakeState {
    fn default() -> Self {
        FakeState {
            contact: None,
            contact_inbox: Ok(None),
            workspace: None,
            custom_fields: Vec::new(),
            bot_fields: Vec::new(),
            conversation_by_contact: None,
            dm_conversation: None,
            latest_conversation: None,
            member: None,
            team_name: None,
            team_names: Vec::new(),
            tags: Vec::new(),
            ad_ids: Vec::new(),
            recent_contact_inbox: None,
            notes: Vec::new(),
            message: None,
            incoming_texts: Vec::new(),
            last_messages: Vec::new(),
            latest_input: None,
            ensure_stored: EnsureStored::PassThrough,
            attachment_media_url: Ok(None),
            recorded_call: None,
            transcribed_call: None,
            signed_url: Ok(String::new()),
            tenant: TenantSettings::default(),
            app_url: String::new(),
            inbox: Ok(None),
            integration_ctx: Ok(Value::Null),
            snapshot: Ok(ContactProfileSnapshot::default()),
            refresh_snapshot: Ok(()),
            user_inbox_link: Ok(None),
            post_message: Ok(None),
            post_caption: Ok(None),
            me_row_id: String::new(),
            appointment: None,
            latest_appointment: None,
            points: None,
            lead: None,
            api_token: Ok(String::new()),
            mirrored_avatar: None,
            avatar_proxy: false,
            referrer: None,
            pipeline_targets: Ok(Vec::new()),
            last_item_id: None,
            coupon: String::new(),
        }
    }
}

/// The fake sources: answers plus the call log.
#[derive(Debug, Default)]
pub struct Fake {
    state: Mutex<FakeState>,
    calls: Mutex<Vec<String>>,
}

fn err<T>(answer: &Answer<T>) -> SourceResult<T>
where
    T: Clone,
{
    answer.clone().map_err(SourceError::new)
}

impl Fake {
    pub fn new(state: FakeState) -> Fake {
        Fake {
            state: Mutex::new(state),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Changes the answers (`mockResolvedValue`).
    pub fn set(&self, f: impl FnOnce(&mut FakeState)) {
        f(&mut self.state.lock().unwrap());
    }

    fn get<T>(&self, f: impl FnOnce(&FakeState) -> T) -> T {
        f(&self.state.lock().unwrap())
    }

    fn record(&self, name: &str, args: &[&str]) {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{name}({})", args.join(", ")));
    }

    /// Every recorded call.
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    /// The calls of one method.
    pub fn calls_to(&self, name: &str) -> Vec<String> {
        let prefix = format!("{name}(");
        self.calls()
            .into_iter()
            .filter(|call| call.starts_with(&prefix))
            .collect()
    }

    /// How often `name` was called.
    pub fn count(&self, name: &str) -> usize {
        self.calls_to(name).len()
    }

    /// `toHaveBeenCalledWith`: `call` is e.g. `"list_ad_ids(workspace-1, contact-1, 50)"`.
    pub fn called(&self, call: &str) -> bool {
        self.calls().iter().any(|c| c == call)
    }

    /// `toHaveBeenLastCalledWith`.
    pub fn last_call_to(&self, name: &str) -> Option<String> {
        self.calls_to(name).pop()
    }

    /// `vi.clearAllMocks()` (keeps the answers).
    pub fn clear_calls(&self) {
        self.calls.lock().unwrap().clear();
    }
}

fn opt(value: Option<i64>) -> String {
    value.map_or("null".to_owned(), |ms| super::iso(ms))
}

impl VariableSources for Fake {
    async fn find_contact(&self, contact_id: &str) -> SourceResult<Option<ContactModel>> {
        self.record("find_contact", &[contact_id]);
        Ok(self.get(|s| s.contact.clone()))
    }

    async fn find_contact_inbox(&self, id: &str) -> SourceResult<Option<ContactInboxModel>> {
        self.record("find_contact_inbox", &[id]);
        self.get(|s| err(&s.contact_inbox))
    }

    async fn find_workspace(&self, workspace_id: &str) -> SourceResult<Option<WorkspaceModel>> {
        self.record("find_workspace", &[workspace_id]);
        Ok(self.get(|s| s.workspace.clone()))
    }

    async fn list_contact_custom_fields(
        &self,
        contact_id: &str,
    ) -> SourceResult<Vec<ContactCustomFieldValue>> {
        self.record("list_contact_custom_fields", &[contact_id]);
        Ok(self.get(|s| s.custom_fields.clone()))
    }

    async fn list_bot_fields(&self, workspace_id: &str) -> SourceResult<Vec<BotFieldRow>> {
        self.record("list_bot_fields", &[workspace_id]);
        Ok(self.get(|s| s.bot_fields.clone()))
    }

    async fn find_conversation_by_contact(
        &self,
        contact_id: &str,
        workspace_id: Option<&str>,
    ) -> SourceResult<Option<ConversationModel>> {
        match workspace_id {
            Some(ws) => self.record("find_conversation_by_contact", &[contact_id, ws]),
            None => self.record("find_conversation_by_contact", &[contact_id]),
        }
        Ok(self.get(|s| s.conversation_by_contact.clone()))
    }

    async fn find_dm_conversation_by_contact(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> SourceResult<Option<ConversationModel>> {
        self.record(
            "find_dm_conversation_by_contact",
            &[workspace_id, contact_id],
        );
        Ok(self.get(|s| s.dm_conversation.clone()))
    }

    async fn find_latest_conversation_by_contact(
        &self,
        contact_id: &str,
    ) -> SourceResult<Option<ConversationModel>> {
        self.record("find_latest_conversation_by_contact", &[contact_id]);
        Ok(self.get(|s| s.latest_conversation.clone()))
    }

    async fn find_member_user(
        &self,
        workspace_id: &str,
        user_id: &str,
    ) -> SourceResult<Option<MemberUser>> {
        self.record("find_member_user", &[workspace_id, user_id]);
        Ok(self.get(|s| s.member.clone()))
    }

    async fn find_inbox_team_name(&self, team_id: &str) -> SourceResult<Option<String>> {
        self.record("find_inbox_team_name", &[team_id]);
        Ok(self.get(|s| s.team_name.clone()))
    }

    async fn list_team_names_by_user_id(
        &self,
        workspace_id: &str,
        user_id: &str,
    ) -> SourceResult<Vec<String>> {
        self.record("list_team_names_by_user_id", &[workspace_id, user_id]);
        Ok(self.get(|s| s.team_names.clone()))
    }

    async fn list_tag_names_by_contact_id(&self, contact_id: &str) -> SourceResult<Vec<String>> {
        self.record("list_tag_names_by_contact_id", &[contact_id]);
        Ok(self.get(|s| s.tags.clone()))
    }

    async fn list_ad_ids(
        &self,
        workspace_id: &str,
        contact_id: &str,
        limit: usize,
    ) -> SourceResult<Vec<String>> {
        self.record(
            "list_ad_ids",
            &[workspace_id, contact_id, &limit.to_string()],
        );
        Ok(self.get(|s| s.ad_ids.clone()))
    }

    async fn find_recent_contact_inbox(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> SourceResult<Option<ContactInboxModel>> {
        self.record("find_recent_contact_inbox", &[workspace_id, contact_id]);
        Ok(self.get(|s| s.recent_contact_inbox.clone()))
    }

    async fn list_contact_note_texts(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> SourceResult<Vec<String>> {
        self.record("list_contact_note_texts", &[workspace_id, contact_id]);
        Ok(self.get(|s| s.notes.clone()))
    }

    async fn find_message_by_id(
        &self,
        id: &str,
        created_at: i64,
        workspace_id: &str,
    ) -> SourceResult<Option<MessageModel>> {
        self.record(
            "find_message_by_id",
            &[id, &super::iso(created_at), workspace_id],
        );
        Ok(self.get(|s| s.message.clone()))
    }

    async fn list_incoming_texts_by_contact_inbox(
        &self,
        contact_inbox_id: &str,
        limit: usize,
        since_time: Option<i64>,
        workspace_id: &str,
    ) -> SourceResult<Vec<String>> {
        self.record(
            "list_incoming_texts_by_contact_inbox",
            &[
                contact_inbox_id,
                &limit.to_string(),
                &opt(since_time),
                workspace_id,
            ],
        );
        Ok(self.get(|s| s.incoming_texts.clone()))
    }

    async fn list_last_messages(
        &self,
        conversation_id: &str,
        limit: usize,
        since_time: i64,
        workspace_id: &str,
    ) -> SourceResult<Vec<ChatMessage>> {
        self.record(
            "list_last_messages",
            &[
                conversation_id,
                &limit.to_string(),
                &super::iso(since_time),
                workspace_id,
            ],
        );
        Ok(self.get(|s| s.last_messages.clone()))
    }

    async fn find_latest_incoming_message_with_attachments(
        &self,
        conversation_id: &str,
        since_time: i64,
        workspace_id: &str,
    ) -> SourceResult<Option<MessageWithAttachments>> {
        self.record(
            "find_latest_incoming_message_with_attachments",
            &[conversation_id, &super::iso(since_time), workspace_id],
        );
        Ok(self.get(|s| s.latest_input.clone()))
    }

    async fn ensure_attachment_stored(
        &self,
        attachment: &Attachment,
        workspace_id: &str,
    ) -> SourceResult<Option<String>> {
        self.record(
            "ensure_attachment_stored",
            &[
                &attachment.id,
                workspace_id,
                &opt(attachment.message_created_at),
                &attachment.origin_path,
            ],
        );
        match self.get(|s| s.ensure_stored.clone()) {
            EnsureStored::PassThrough => {
                Ok(Some(attachment.origin_path.clone()).filter(|p| !p.is_empty()))
            }
            EnsureStored::Answer(answer) => answer.map_err(SourceError::new),
        }
    }

    async fn resolve_attachment_media_url(
        &self,
        request: &AttachmentMediaRequest,
    ) -> SourceResult<Option<String>> {
        self.record(
            "resolve_attachment_media_url",
            &[
                &request.attachment_id,
                &request.workspace_id,
                &request.origin_path,
                &request.channel,
                &opt(request.message_created_at),
            ],
        );
        self.get(|s| err(&s.attachment_media_url))
    }

    async fn find_latest_recorded_call(
        &self,
        contact_id: &str,
    ) -> SourceResult<Option<CallRecord>> {
        self.record("find_latest_recorded_call", &[contact_id]);
        Ok(self.get(|s| s.recorded_call.clone()))
    }

    async fn find_latest_transcribed_call(
        &self,
        contact_id: &str,
    ) -> SourceResult<Option<CallRecord>> {
        self.record("find_latest_transcribed_call", &[contact_id]);
        Ok(self.get(|s| s.transcribed_call.clone()))
    }

    async fn recording_signed_url(&self, recording_path: &str) -> SourceResult<String> {
        self.record("recording_signed_url", &[recording_path]);
        self.get(|s| err(&s.signed_url))
    }

    async fn tenant_settings(&self, workspace_id: &str) -> SourceResult<TenantSettings> {
        self.record("tenant_settings", &[workspace_id]);
        Ok(self.get(|s| s.tenant.clone()))
    }

    async fn workspace_app_url(&self, workspace_id: &str) -> SourceResult<String> {
        self.record("workspace_app_url", &[workspace_id]);
        Ok(self.get(|s| s.app_url.clone()))
    }

    async fn find_inbox_with_integrations(
        &self,
        inbox_id: &str,
    ) -> SourceResult<Option<InboxWithIntegrations>> {
        self.record("find_inbox_with_integrations", &[inbox_id]);
        self.get(|s| err(&s.inbox))
    }

    async fn resolve_integration_context(
        &self,
        workspace_id: &str,
        contact_inbox: &ContactInboxModel,
    ) -> SourceResult<Value> {
        self.record(
            "resolve_integration_context",
            &[
                workspace_id,
                &contact_inbox.channel,
                &contact_inbox.source_id,
            ],
        );
        self.get(|s| err(&s.integration_ctx))
    }

    async fn run_get_profile_snapshot(
        &self,
        ctx: &Value,
        source_id: &str,
    ) -> SourceResult<ContactProfileSnapshot> {
        self.record("run_get_profile_snapshot", &[&ctx.to_string(), source_id]);
        self.get(|s| err(&s.snapshot))
    }

    async fn refresh_profile_snapshot(
        &self,
        contact_inbox_id: &str,
        inbox_id: &str,
        snapshot: &ContactProfileSnapshot,
    ) -> SourceResult<()> {
        self.record(
            "refresh_profile_snapshot",
            &[
                contact_inbox_id,
                inbox_id,
                &serde_json::to_string(snapshot).unwrap(),
            ],
        );
        self.get(|s| err(&s.refresh_snapshot))
    }

    async fn messenger_user_inbox_link(
        &self,
        auth: &Value,
        user_id: &str,
    ) -> SourceResult<Option<String>> {
        self.record("messenger_user_inbox_link", &[&auth.to_string(), user_id]);
        self.get(|s| err(&s.user_inbox_link))
    }

    async fn messenger_post_message(
        &self,
        auth: &Value,
        post_id: &str,
    ) -> SourceResult<Option<String>> {
        self.record("messenger_post_message", &[&auth.to_string(), post_id]);
        self.get(|s| err(&s.post_message))
    }

    async fn instagram_post_caption(
        &self,
        auth: &Value,
        post_id: &str,
    ) -> SourceResult<Option<String>> {
        self.record("instagram_post_caption", &[&auth.to_string(), post_id]);
        self.get(|s| err(&s.post_caption))
    }

    async fn create_me_system_field(&self, payload: &MeSystemFieldPayload) -> SourceResult<String> {
        self.record(
            "create_me_system_field",
            &[
                &payload.workspace_id,
                &payload.channel,
                &payload.integration_id,
                &payload.source_id,
                &payload.contact_inbox_id,
                payload.conversation_id.as_deref().unwrap_or("-"),
                &payload.contact_id,
            ],
        );
        Ok(self.get(|s| s.me_row_id.clone()))
    }

    async fn find_appointment(
        &self,
        workspace_id: &str,
        id: &str,
    ) -> SourceResult<Option<Appointment>> {
        self.record("find_appointment", &[workspace_id, id]);
        Ok(self.get(|s| s.appointment.clone()))
    }

    async fn find_latest_appointment_for_contact(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> SourceResult<Option<Appointment>> {
        self.record(
            "find_latest_appointment_for_contact",
            &[workspace_id, contact_id],
        );
        Ok(self.get(|s| s.latest_appointment.clone()))
    }

    async fn latest_questionnaire_points(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> SourceResult<Option<f64>> {
        self.record("latest_questionnaire_points", &[workspace_id, contact_id]);
        Ok(self.get(|s| s.points))
    }

    async fn find_latest_lead(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> SourceResult<Option<LeadAdsLead>> {
        self.record("find_latest_lead", &[workspace_id, contact_id]);
        Ok(self.get(|s| s.lead.clone()))
    }

    async fn resolve_default_api_token(&self, workspace_id: &str) -> SourceResult<String> {
        self.record("resolve_default_api_token", &[workspace_id]);
        self.get(|s| err(&s.api_token))
    }

    async fn resolve_contact_avatar(
        &self,
        workspace_id: &str,
        contact: &ContactModel,
        contact_inbox: Option<&ContactInboxModel>,
    ) -> SourceResult<AvatarResolution> {
        self.record("resolve_contact_avatar", &[workspace_id, &contact.id]);
        if let Some(avatar) = contact.avatar.clone().filter(|a| !a.is_empty()) {
            return Ok(AvatarResolution::StorageKey(avatar));
        }
        let Some(ci) =
            contact_inbox.filter(|ci| ci.channel == "messenger" || ci.channel == "instagram")
        else {
            return Ok(AvatarResolution::None);
        };
        if self.get(|s| s.avatar_proxy) {
            return Ok(AvatarResolution::Url(format!(
                "https://app.example.com/media/avatar/{}",
                ci.id
            )));
        }
        self.record("ensure_contact_avatar_mirrored", &[&ci.id, workspace_id]);
        Ok(match self.get(|s| s.mirrored_avatar.clone()) {
            Some(avatar) => AvatarResolution::StorageKey(avatar),
            None => AvatarResolution::None,
        })
    }

    async fn leaderboard_referrer_name(
        &self,
        contact_id: &str,
        workspace_id: &str,
    ) -> SourceResult<Option<String>> {
        self.record("leaderboard_referrer_name", &[contact_id, workspace_id]);
        Ok(self.get(|s| s.referrer.clone()))
    }

    async fn load_pipeline_variable_targets(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> SourceResult<Vec<PipelineVariableTarget>> {
        self.record(
            "load_pipeline_variable_targets",
            &[workspace_id, contact_id],
        );
        self.get(|s| err(&s.pipeline_targets))
    }

    async fn load_last_pipeline_item_id(
        &self,
        workspace_id: &str,
        contact_id: &str,
    ) -> SourceResult<Option<String>> {
        self.record("load_last_pipeline_item_id", &[workspace_id, contact_id]);
        Ok(self.get(|s| s.last_item_id.clone()))
    }

    async fn resolve_coupon_variable(
        &self,
        workspace_id: &str,
        contact_id: &str,
        topic_id: &str,
    ) -> SourceResult<String> {
        self.record(
            "resolve_coupon_variable",
            &[workspace_id, contact_id, topic_id],
        );
        Ok(self.get(|s| s.coupon.clone()))
    }
}

/// The fake plus the other module-level dependencies (`withCache`, the
/// encryption key, the clock).
pub struct Harness {
    pub fake: Fake,
    pub cache: Cache,
    pub encryption: Encryption,
    pub clock: Arc<TestClock>,
}

impl Harness {
    /// Now = the real clock.
    pub fn new(state: FakeState) -> Harness {
        Harness::at(super::now_ms(), state)
    }

    /// Now = `now_ms` (`vi.setSystemTime`).
    pub fn at(now_ms: i64, state: FakeState) -> Harness {
        let clock = Arc::new(TestClock::new(now_ms));
        Harness {
            fake: Fake::new(state),
            cache: Cache::new(clock.clone() as Arc<dyn Clock>),
            encryption: Encryption::with_clock(
                Keys::new(TEST_ENCRYPTION_KEY).expect("test key"),
                clock.clone() as Arc<dyn Clock>,
            ),
            clock,
        }
    }

    pub fn v(&self) -> Variables<'_, Fake> {
        Variables::new(
            &self.fake,
            &self.cache,
            &self.encryption,
            self.clock.as_ref(),
        )
    }
}

/// `new Map(fields.map(...))` of `ContactCustomFieldValue`s.
pub fn custom_fields(fields: &[(&str, &str, Option<&str>)]) -> CustomFieldsMap {
    fields
        .iter()
        .map(|(key, kind, value)| {
            (
                (*key).to_owned(),
                ContactCustomFieldValue {
                    key: (*key).to_owned(),
                    kind: (*kind).to_owned(),
                    value: value.map(str::to_owned),
                    description: String::new(),
                },
            )
        })
        .collect::<HashMap<_, _>>()
}

/// `new Map(fields.map(...))` of `BotFieldValue`s.
pub fn bot_fields(fields: &[(&str, &str, Option<&str>)]) -> BotFieldsMap {
    fields
        .iter()
        .map(|(id, kind, value)| {
            (
                (*id).to_owned(),
                BotFieldValue {
                    kind: (*kind).to_owned(),
                    value: value.map(str::to_owned),
                },
            )
        })
        .collect()
}

pub fn s(text: &str) -> Option<String> {
    Some(text.to_owned())
}
