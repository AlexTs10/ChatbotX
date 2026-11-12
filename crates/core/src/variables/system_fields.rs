//! `getSystemFieldValue` (`utils.ts`): the value of one `{{system_field}}`.

use super::sources::{AvatarResolution, VariableSources};
use super::system_field::{
    ContactSource, SystemFieldType, is_workspace_scheduled_for_deletion, language_from_locale,
    offset_from_stored_timezone, resolve_gender_label,
};
use super::text::{
    capitalize_first_letter, format_date, format_date_time, get_contact_timezone, get_timezone,
};
use super::{
    Appointment, ContactInboxModel, MessageModel, VariableContext, VariableError, Variables,
};
use crate::encryption::UserHashParams;
use crate::encryption::{
    APPOINTMENT_TOKEN_TTL_MS, AppointmentClaims, ContactInboxClaims, LEADERBOARD_VIEW_TOKEN_TTL_MS,
    MINI_APP_TOKEN_TTL_MS, MINIGAME_PLAY_TOKEN_TTL_MS, MiniAppClaims, SHOP_TOKEN_TTL_MS,
    ShopCustomerClaims,
};
use crate::js;
use crate::utils::datetime::DATE_TIME_FORMAT;
use crate::utils::datetime::format_with_fallback;
use crate::utils::tz::DateArg;

/// `contactSourceLabels`.
fn contact_source_label(source: ContactSource) -> &'static str {
    match source {
        ContactSource::InboundMessage => "Inbound Message",
        ContactSource::Webchat => "Webchat",
        ContactSource::Ads => "Ads",
        ContactSource::BotLink => "Bot Link",
        ContactSource::ChatPlugin => "Chat Plugin",
        ContactSource::Comments => "Facebook/IG Comment",
        ContactSource::LiveComments => "Facebook/IG Livestream",
        ContactSource::Imported => "Imported",
        ContactSource::Api => "API",
        ContactSource::Direct => "Direct",
        ContactSource::InboundWebhook => "Inbound Webhook",
    }
}

/// `formatContactSource`: the human label; comments from TikTok / Threads
/// get their own name; an unknown source as it is; `Unknown` without one.
pub fn format_contact_source(source: Option<&str>, channel: Option<&str>) -> String {
    let Some(source) = source.filter(|s| !s.is_empty()) else {
        return "Unknown".to_owned();
    };
    let Some(parsed) = ContactSource::parse(source) else {
        return source.to_owned();
    };
    let comment_label = match channel {
        Some("tiktok") => Some("TikTok Comment"),
        Some("threads") => Some("Threads Comment"),
        _ => None,
    };
    match (parsed, comment_label) {
        (ContactSource::Comments, Some(label)) => label.to_owned(),
        _ => contact_source_label(parsed).to_owned(),
    }
}

/// `getReferralValue`: a non-empty referral string.
fn referral_value(
    contact_inbox: Option<&ContactInboxModel>,
    read: fn(&super::Referral) -> &Option<String>,
) -> Option<String> {
    contact_inbox
        .and_then(|ci| ci.referral.as_ref())
        .and_then(|r| read(r).clone())
        .filter(|v| !v.is_empty())
}

/// `getCommentMessagePostId`.
fn comment_message_post_id(message: Option<&MessageModel>) -> Option<String> {
    message
        .and_then(|m| m.content_attributes.as_ref())
        .and_then(|attrs| attrs.get("postId"))
        .and_then(|id| id.as_str())
        .map(str::to_owned)
}

type Res<T> = Result<T, VariableError>;

impl<S: VariableSources> Variables<'_, S> {
    /// `resolveVariableContactAvatarUrl`.
    async fn contact_avatar_url(&self, context: &VariableContext) -> Res<Option<String>> {
        let contact = &context.contact;
        let resolution = self
            .sources
            .resolve_contact_avatar(
                &contact.workspace_id,
                contact,
                context.contact_inbox.as_ref(),
            )
            .await?;
        Ok(match resolution {
            AvatarResolution::None => None,
            AvatarResolution::Url(url) => Some(url),
            AvatarResolution::StorageKey(key) => {
                self.to_public_storage_url(Some(&key), &contact.workspace_id)
                    .await?
            }
        })
    }

    /// `getLastUserComment`: the contact's last comment (by its stored
    /// message pointer), unless deleted.
    async fn last_user_comment(&self, context: &VariableContext) -> Res<Option<MessageModel>> {
        let Some(contact_inbox) = &context.contact_inbox else {
            return Ok(None);
        };
        let (Some(id), Some(created_at)) = (
            contact_inbox
                .last_comment_message_id
                .as_deref()
                .filter(|id| !id.is_empty()),
            contact_inbox.last_comment_message_at,
        ) else {
            return Ok(None);
        };
        let message = self
            .sources
            .find_message_by_id(id, created_at, &context.contact.workspace_id)
            .await?;
        Ok(message.filter(|m| m.deleted_at.is_none()))
    }

    /// `getAppointment`: the context appointment (when it is the contact's),
    /// else the contact's latest one.
    async fn appointment(&self, context: &VariableContext) -> Res<Option<Appointment>> {
        let contact = &context.contact;
        if let Some(id) = context
            .appointment_id
            .as_deref()
            .filter(|id| !id.is_empty())
        {
            let appointment = self
                .sources
                .find_appointment(&contact.workspace_id, id)
                .await?;
            return Ok(appointment.filter(|a| a.contact_id == contact.id));
        }
        Ok(self
            .sources
            .find_latest_appointment_for_contact(&contact.workspace_id, &contact.id)
            .await?)
    }

    /// `getFlowStepValue`.
    async fn flow_step_value(
        &self,
        context: &VariableContext,
        current: bool,
    ) -> Res<Option<String>> {
        let conversation = self
            .sources
            .find_dm_conversation_by_contact(&context.contact.workspace_id, &context.contact.id)
            .await?;
        Ok(conversation.and_then(|c| if current { c.current_step } else { c.last_step }))
    }

    async fn latest_lead_value(
        &self,
        context: &VariableContext,
        read: fn(super::LeadAdsLead) -> Option<String>,
    ) -> Res<Option<String>> {
        let lead = self
            .sources
            .find_latest_lead(&context.contact.workspace_id, &context.contact.id)
            .await?;
        Ok(lead.and_then(read))
    }

    async fn api_key(&self, context: &VariableContext) -> Option<String> {
        if context
            .workspace
            .as_ref()
            .is_some_and(is_workspace_scheduled_for_deletion)
        {
            return None;
        }
        let workspace_id = &context.contact.workspace_id;
        match self.sources.resolve_default_api_token(workspace_id).await {
            Ok(token) => Some(token),
            Err(err) => {
                tracing::error!(
                    err = %err,
                    workspace_id = %workspace_id,
                    "Failed to resolve {{{{api_key}}}} default token plaintext"
                );
                None
            }
        }
    }

    async fn booking_link(&self, context: &VariableContext) -> Res<Option<String>> {
        let Some(appointment) = self.appointment(context).await? else {
            return Ok(None);
        };
        let settings = self
            .sources
            .tenant_settings(&context.contact.workspace_id)
            .await?;
        let token = self.encryption.sign_appointment_schedule_token(
            &AppointmentClaims {
                appointment_id: appointment.id.clone(),
                workspace_id: appointment.workspace_id.clone(),
                contact_id: appointment.contact_id.clone(),
                conversation_id: appointment.conversation_id.clone(),
                ..AppointmentClaims::default()
            },
            APPOINTMENT_TOKEN_TTL_MS,
        )?;
        Ok(Some(build_appointment_url(
            &settings.app_url,
            "/booking/schedule",
            &token,
        )?))
    }

    /// `getSystemFieldValue`: `None` renders as an empty substitution. A
    /// failing source fails the render, except where the original degrades
    /// to `null` (`api_key`, `last_call_recorded`, `last_input`, profile
    /// snapshot fetches).
    pub async fn get_system_field_value(
        &self,
        context: &VariableContext,
        key: SystemFieldType,
    ) -> Res<Option<String>> {
        use SystemFieldType as F;
        let contact = &context.contact;
        let contact_inbox = context.contact_inbox.as_ref();
        let workspace = context.workspace.as_ref();
        let timezone = get_timezone(context);
        let timezone = timezone.as_deref();
        let contact_timezone = get_contact_timezone(context);
        let contact_timezone = Some(contact_timezone.as_str());
        let ws = contact.workspace_id.as_str();
        let now = self.clock.now_ms();

        Ok(match key {
            F::Email => contact.email.clone(),
            F::Phone => contact.phone_number.clone(),
            F::FirstName => contact.first_name.clone(),
            F::LastName => contact.last_name.clone(),
            F::FullName => Some(
                [contact.first_name.as_deref(), contact.last_name.as_deref()]
                    .into_iter()
                    .flatten()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            F::ProfilePic | F::Avatar => self.contact_avatar_url(context).await?,
            F::Gender => Some(resolve_gender_label(
                workspace.and_then(|w| w.language.as_deref()),
                contact.gender.as_deref(),
            )),
            F::UserCountry => contact.country.clone(),
            F::UserState => contact.state.clone(),
            F::UserCity => contact.city.clone(),
            F::Locale => contact.locale.clone(),
            F::Locale2 => contact
                .locale
                .as_deref()
                .map(|locale| locale.split(['-', '_']).next().unwrap_or("").to_owned()),
            F::Timezone => offset_from_stored_timezone(contact.timezone.as_deref(), now),
            F::Language => contact_inbox
                .and_then(|ci| ci.language.clone())
                .or_else(|| language_from_locale(contact.locale.as_deref())),
            F::UserId => Some(contact.id.clone()),
            F::SubscribedDate => {
                format_date(contact_inbox.and_then(|ci| ci.created_at), contact_timezone)
            }
            F::LastSeen => format_date_time(
                contact_inbox.and_then(|ci| ci.contact_last_read_at),
                contact_timezone,
            ),
            F::LastOpportunityId => match &context.pipeline_items {
                Some(loader) => loader.last_item_id(self.sources).await?,
                None => {
                    self.sources
                        .load_last_pipeline_item_id(ws, &contact.id)
                        .await?
                }
            },
            F::LastInput => self.get_contact_last_input(&contact.id).await?,
            F::LastInputType => self.get_contact_last_input_type(&contact.id).await?,
            F::LastTextInput => self.get_contact_last_text_input(&contact.id).await?,
            F::LastCallRecorded => self.get_contact_last_call_recording(&contact.id).await?,
            F::LastCallTranscript => self.get_contact_last_call_transcript(&contact.id).await?,
            F::UserChannel => {
                let channel = match contact_inbox {
                    Some(ci) => Some(ci.channel.clone()),
                    None => self.find_primary_contact_channel(&contact.id, ws).await?,
                };
                capitalize_first_letter(channel.as_deref())
            }
            F::UserTags => Some(self.list_contact_tags_string(&contact.id).await?),
            F::UserHash => contact_inbox.map(|ci| {
                self.encryption.sign_user_hash(&UserHashParams {
                    source_id: ci.source_id.clone(),
                    contact_inbox_id: ci.id.clone(),
                })
            }),
            F::MinigamePlayToken => match contact_inbox {
                Some(ci) => Some(self.encryption.sign_minigame_play_token(
                    &contact_inbox_claims(ws, &contact.id, &ci.id),
                    MINIGAME_PLAY_TOKEN_TTL_MS,
                )?),
                None => None,
            },
            F::LeaderboardViewToken => match contact_inbox {
                Some(ci) => Some(self.encryption.sign_leaderboard_view_token(
                    &contact_inbox_claims(ws, &contact.id, &ci.id),
                    LEADERBOARD_VIEW_TOKEN_TTL_MS,
                )?),
                None => None,
            },
            F::ReferralName => self.get_leaderboard_referrer_name(&contact.id, ws).await?,
            F::MiniAppToken => Some(self.encryption.sign_mini_app_token(
                &MiniAppClaims {
                    workspace_id: ws.to_owned(),
                    contact_id: contact.id.clone(),
                },
                MINI_APP_TOKEN_TTL_MS,
            )?),
            F::ShopToken => Some(self.encryption.sign_shop_token(
                &ShopCustomerClaims {
                    workspace_id: ws.to_owned(),
                    contact_id: contact.id.clone(),
                    contact_inbox_id: contact_inbox.map(|ci| ci.id.clone()),
                    conversation_id: None,
                },
                SHOP_TOKEN_TTL_MS,
            )?),
            F::WorkspaceId | F::AccountId => Some(ws.to_owned()),
            F::UserSource => Some(format_contact_source(
                contact_inbox.and_then(|ci| ci.source.as_deref()),
                contact_inbox.map(|ci| ci.channel.as_str()),
            )),
            F::AssignedAdminName | F::MemberName => {
                self.resolve_assignee_name(&contact.id, ws).await?
            }
            F::AssignedAdminEmail => self.resolve_assignee_email(&contact.id, ws).await?,
            F::AssignedAdminId => self.resolve_assignee_id(&contact.id, ws).await?,
            F::CurrentUserTime => Some(format_with_fallback(
                DateArg::Ms(now),
                contact_timezone,
                DATE_TIME_FORMAT,
            )),
            F::ChatHistory => self.get_chat_history(&contact.id, 50, false).await?,
            F::ChatHistoryLarge => self.get_chat_history(&contact.id, 200, false).await?,
            F::ChatHistoryDetails => self.get_chat_history(&contact.id, 50, true).await?,
            F::ChatHistoryDetailsLarge => self.get_chat_history(&contact.id, 200, true).await?,
            F::AiQueuedMessages => self.get_queued_messages(context).await?,
            F::UserNotes => Some(self.list_contact_notes_string(&contact.id, ws).await?),
            F::CurrentTime => Some(format_with_fallback(
                DateArg::Ms(now),
                timezone,
                DATE_TIME_FORMAT,
            )),
            F::WorkspaceName | F::AccountName => workspace.and_then(|w| w.name.clone()),
            F::AccountImage => {
                let logo = workspace
                    .and_then(|w| w.logo.as_deref())
                    .filter(|l| !l.is_empty());
                self.to_public_storage_url(logo, ws).await?
            }
            F::PageUserName
            | F::InboxLink
            | F::IgUserName
            | F::IgFollowers
            | F::IgVerified
            | F::IgFollowBusiness
            | F::IgBusinessFollowUser
            | F::TimezoneName
            | F::FbChatLink
            | F::UserCode
            | F::Webchat
            | F::WaUserId
            | F::WaUserName => {
                let conversation_id = context.conversation.as_ref().map(|c| c.id.as_str());
                self.get_integration_field(contact, key, contact_inbox, conversation_id)
                    .await?
            }
            F::Me => {
                if workspace.is_some_and(is_workspace_scheduled_for_deletion) {
                    None
                } else {
                    let conversation_id = context.conversation.as_ref().map(|c| c.id.as_str());
                    self.get_integration_field(contact, key, contact_inbox, conversation_id)
                        .await?
                }
            }
            F::LastRef => contact.ref_.clone().or_else(|| {
                contact_inbox
                    .and_then(|ci| ci.referral.as_ref())
                    .and_then(|r| r.ref_.clone())
            }),
            F::LastInteraction => format_date_time(
                contact_inbox.and_then(|ci| ci.last_incoming_message_at),
                timezone,
            ),
            F::LastUserNote => self.get_latest_contact_note_string(&contact.id, ws).await?,
            F::TeamName => self.get_assigned_team_name(&contact.id).await?,
            F::AdminName => match context.sent_by_user_id.as_deref().filter(|u| !u.is_empty()) {
                Some(user_id) => self.resolve_sender_name(ws, user_id).await?,
                None => None,
            },
            F::AdminTeamName => {
                match context.sent_by_user_id.as_deref().filter(|u| !u.is_empty()) {
                    Some(user_id) => self.list_sender_team_names(ws, user_id).await?,
                    None => None,
                }
            }
            // No upstream tracking yet.
            F::LastOrder => None,
            // Always blank, for a template slot that must stay empty.
            F::Empty => Some(String::new()),
            // `last_btt_title` is the legacy spelling.
            F::LastBtnTitle | F::LastBttTitle => {
                contact_inbox.and_then(|ci| ci.last_btn_title.clone())
            }
            F::ConsecutiveFailedReply => {
                contact_inbox.map(|ci| ci.consecutive_failed_reply.unwrap_or(0).to_string())
            }
            F::UserExternalId => contact_inbox.map(|ci| ci.source_id.clone()),
            F::WebchatParentUrl => contact_inbox.and_then(|ci| ci.webchat_parent_url.clone()),
            // `page_api_key` is the legacy spelling.
            F::ApiKey | F::PageApiKey => self.api_key(context).await,
            F::LastAd => referral_value(contact_inbox, |r| &r.ad_id),
            F::LastCtwa => referral_value(contact_inbox, |r| &r.ctwa_clid),
            F::AdIds => self.list_contact_ad_ids_string(&contact.id, ws).await?,
            F::AdId => self.get_first_contact_ad_id(&contact.id, ws).await?,
            F::LastAdSourceUrl => referral_value(contact_inbox, |r| &r.source_url),
            F::LastAdSourcePlatform => referral_value(contact_inbox, |r| &r.source_platform),
            F::LastAdHeadline => referral_value(contact_inbox, |r| &r.ad_title),
            F::LastAdBody => referral_value(contact_inbox, |r| &r.ad_body),
            F::LastFbComment => self.last_user_comment(context).await?.and_then(|m| m.text),
            F::LastPostId => {
                comment_message_post_id(self.last_user_comment(context).await?.as_ref())
            }
            F::LastCommentId => self
                .last_user_comment(context)
                .await?
                .and_then(|m| m.source_id),
            F::LastStoryId => contact_inbox.and_then(|ci| ci.last_story_id.clone()),
            F::TotalNewTagged => contact.total_new_tagged.map(|n| n.to_string()),
            F::TotalTagged => contact.total_tagged.map(|n| n.to_string()),
            F::LastLatitude => contact
                .location
                .as_ref()
                .and_then(|l| l.latitude)
                .map(js::number_to_string),
            F::LastLongitude => contact
                .location
                .as_ref()
                .and_then(|l| l.longitude)
                .map(js::number_to_string),
            F::LastErrorLog => contact_inbox.and_then(|ci| ci.last_error_log.clone()),
            F::LastOutboundMessageAt => format_date_time(
                contact_inbox.and_then(|ci| ci.last_outbound_message_at),
                timezone,
            ),
            F::LastCommentedPostText => {
                let message = self.last_user_comment(context).await?;
                let post_id = comment_message_post_id(message.as_ref());
                self.get_last_commented_post_text(contact_inbox, post_id.as_deref())
                    .await?
            }
            F::LastStep => self.flow_step_value(context, false).await?,
            F::CurrentStep => self.flow_step_value(context, true).await?,
            F::BookingId => self.appointment(context).await?.map(|a| a.id),
            F::BookingCalendar => self
                .appointment(context)
                .await?
                .and_then(|a| a.calendar_name),
            F::BookingDate => {
                let appointment = self.appointment(context).await?;
                let zone = appointment
                    .as_ref()
                    .and_then(|a| a.invitee_timezone.clone())
                    .or_else(|| contact_timezone.map(str::to_owned));
                format_date_time(appointment.and_then(|a| a.start_at), zone.as_deref())
            }
            F::LastInputFailure => contact_inbox.and_then(|ci| ci.last_input_failure.clone()),
            // A contact who never completed a scored questionnaire has 0 points.
            F::LastPoints => Some(js::number_to_string(
                self.sources
                    .latest_questionnaire_points(ws, &contact.id)
                    .await?
                    .unwrap_or(0.0),
            )),
            F::LastLeadId => self.latest_lead_value(context, |l| l.leadgen_id).await?,
            F::LastFormId => self.latest_lead_value(context, |l| l.form_id).await?,
            F::LastFormName => self.latest_lead_value(context, |l| l.form_name).await?,
            F::LastAdgroupId => self.latest_lead_value(context, |l| l.adgroup_id).await?,
            F::BookingLink => self.booking_link(context).await?,
        })
    }
}

fn contact_inbox_claims(
    workspace_id: &str,
    contact_id: &str,
    contact_inbox_id: &str,
) -> ContactInboxClaims {
    ContactInboxClaims {
        workspace_id: workspace_id.to_owned(),
        contact_id: contact_id.to_owned(),
        contact_inbox_id: contact_inbox_id.to_owned(),
    }
}

/// `buildAppointmentUrl(appUrl, pathname, token)`.
pub fn build_appointment_url(
    app_url: &str,
    pathname: &str,
    token: &str,
) -> Result<String, super::SourceError> {
    let mut url = url::Url::parse(app_url)
        .and_then(|base| base.join(pathname))
        .map_err(|err| super::SourceError::new(format!("Invalid URL: {err}")))?;
    url.query_pairs_mut().append_pair("token", token);
    Ok(url.to_string())
}
