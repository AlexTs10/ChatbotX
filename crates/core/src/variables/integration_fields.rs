//! `helpers/integration-fields.ts`: system fields read from the inbox's
//! integration (page name, Instagram profile snapshot, chat links, the `me`
//! privacy link, WhatsApp user ids) and the commented post text.

use chatbotx_kv::cache::CacheOpts;

use super::sources::{MeSystemFieldPayload, SourceError, VariableSources};
use super::system_field::{SystemFieldType, normalize_stored_timezone, supports_profile_snapshot};
use super::{
    ContactInboxModel, ContactModel, ContactProfileSnapshot, InboxWithIntegrations, VariableError,
    Variables,
};
use crate::encryption::MeLinkParams;

/// `PROFILE_SNAPSHOT_CACHE_TTL` (seconds).
pub const PROFILE_SNAPSHOT_CACHE_TTL: u64 = 300;
/// `FB_CHAT_LINK_CACHE_TTL` (seconds).
pub const FB_CHAT_LINK_CACHE_TTL: u64 = 60 * 60;
/// `POST_TEXT_CACHE_TTL` (seconds): page owners edit captions in place.
pub const POST_TEXT_CACHE_TTL: u64 = 60;

/// `profileSnapshotCacheKey`.
pub fn profile_snapshot_cache_key(contact_inbox_id: &str) -> String {
    format!("profile-snapshot:{contact_inbox_id}")
}

/// The system fields read from the profile snapshot.
fn is_profile_snapshot_field(key: SystemFieldType) -> bool {
    matches!(
        key,
        SystemFieldType::IgUserName
            | SystemFieldType::IgFollowers
            | SystemFieldType::IgVerified
            | SystemFieldType::IgFollowBusiness
            | SystemFieldType::IgBusinessFollowUser
    )
}

/// `profileSnapshotFields[key].read`, as the rendered string.
fn read_snapshot_field(key: SystemFieldType, snapshot: &ContactProfileSnapshot) -> Option<String> {
    match key {
        SystemFieldType::IgUserName => snapshot.username.clone(),
        SystemFieldType::IgFollowers => snapshot.follower_count.map(|n| n.to_string()),
        SystemFieldType::IgVerified => snapshot.account_verified.map(|b| b.to_string()),
        SystemFieldType::IgFollowBusiness => snapshot.follows_business.map(|b| b.to_string()),
        SystemFieldType::IgBusinessFollowUser => {
            snapshot.business_follows_contact.map(|b| b.to_string())
        }
        _ => None,
    }
}

/// `getStoredProfileSnapshot`: the contact-inbox columns, `None` when all
/// are empty.
fn stored_profile_snapshot(contact_inbox: &ContactInboxModel) -> Option<ContactProfileSnapshot> {
    let snapshot = ContactProfileSnapshot {
        follows_business: contact_inbox.follows_business,
        business_follows_contact: contact_inbox.business_follows_contact,
        account_verified: contact_inbox.account_verified,
        follower_count: contact_inbox.follower_count,
        username: contact_inbox.source_username.clone(),
    };
    (snapshot != ContactProfileSnapshot::default()).then_some(snapshot)
}

/// `getChannelIntegrationId`.
pub fn channel_integration_id(inbox: &InboxWithIntegrations, channel: &str) -> Option<String> {
    inbox.integration(channel).map(|row| row.id.clone())
}

type Res<T> = Result<T, VariableError>;

impl<S: VariableSources> Variables<'_, S> {
    /// `resolveLiveProfileSnapshot`: the channel's `getProfileSnapshot`,
    /// cached for [`PROFILE_SNAPSHOT_CACHE_TTL`] and written through to the
    /// contact-inbox columns. A failed fetch is logged and `None` (and not
    /// cached, so the next render retries); a failed persist is logged.
    async fn resolve_live_profile_snapshot(
        &self,
        contact_inbox: &ContactInboxModel,
        integration_id: Option<&str>,
        workspace_id: &str,
    ) -> Option<ContactProfileSnapshot> {
        let tags = integration_id
            .map(|id| vec![format!("integration:{}:{id}", contact_inbox.channel)])
            .unwrap_or_default();
        let opts = CacheOpts {
            ttl_secs: PROFILE_SNAPSHOT_CACHE_TTL,
            tags,
            ..CacheOpts::default()
        };
        let loaded = self
            .cache
            .get_or_load::<ContactProfileSnapshot, std::convert::Infallible, _>(
                &profile_snapshot_cache_key(&contact_inbox.id),
                opts,
                || async {
                    let fetched = async {
                        let ctx = self
                            .sources
                            .resolve_integration_context(workspace_id, contact_inbox)
                            .await?;
                        self.sources
                            .run_get_profile_snapshot(&ctx, &contact_inbox.source_id)
                            .await
                    };
                    let snapshot = match fetched.await {
                        Ok(snapshot) => snapshot,
                        Err(err) => {
                            tracing::warn!(
                                err = %err,
                                contact_inbox_id = %contact_inbox.id,
                                source_id = %contact_inbox.source_id,
                                "Contact profile snapshot fetch failed"
                            );
                            return Ok(None);
                        }
                    };
                    if let Err(err) = self
                        .sources
                        .refresh_profile_snapshot(
                            &contact_inbox.id,
                            &contact_inbox.inbox_id,
                            &snapshot,
                        )
                        .await
                    {
                        tracing::warn!(
                            err = %err,
                            contact_inbox_id = %contact_inbox.id,
                            "Contact profile snapshot persist failed"
                        );
                    }
                    Ok(Some(snapshot))
                },
            )
            .await;
        match loaded {
            Ok(snapshot) => snapshot,
            Err(never) => match never {},
        }
    }

    /// `refreshContactProfileSnapshot`: drops the cached snapshot and
    /// fetches a fresh one (written through). Never fails.
    pub async fn refresh_contact_profile_snapshot(
        &self,
        contact_inbox: &ContactInboxModel,
        workspace_id: &str,
    ) {
        if !supports_profile_snapshot(&contact_inbox.channel) {
            return;
        }
        let inbox = match self
            .sources
            .find_inbox_with_integrations(&contact_inbox.inbox_id)
            .await
        {
            Ok(inbox) => inbox,
            Err(err) => {
                tracing::warn!(
                    err = %err,
                    contact_inbox_id = %contact_inbox.id,
                    "Contact profile snapshot refresh failed"
                );
                return;
            }
        };
        self.cache
            .invalidate_keys(&[profile_snapshot_cache_key(&contact_inbox.id).as_str()]);
        let integration_id = inbox
            .as_ref()
            .and_then(|inbox| channel_integration_id(inbox, &contact_inbox.channel));
        self.resolve_live_profile_snapshot(contact_inbox, integration_id.as_deref(), workspace_id)
            .await;
    }

    /// `resolveProfileSnapshotField`.
    async fn resolve_profile_snapshot_field(
        &self,
        contact_inbox: &ContactInboxModel,
        inbox: &InboxWithIntegrations,
        key: SystemFieldType,
    ) -> Option<String> {
        if !supports_profile_snapshot(&contact_inbox.channel) {
            return None;
        }
        let stored = stored_profile_snapshot(contact_inbox);
        // The stored handle wins (identity, not a metric).
        if key == SystemFieldType::IgUserName {
            if let Some(value) = stored.as_ref().and_then(|s| read_snapshot_field(key, s)) {
                return Some(value);
            }
        }
        let integration_id = channel_integration_id(inbox, &contact_inbox.channel);
        let live = self
            .resolve_live_profile_snapshot(
                contact_inbox,
                integration_id.as_deref(),
                &inbox.workspace_id,
            )
            .await;
        live.or(stored).and_then(|s| read_snapshot_field(key, &s))
    }

    async fn cached_post_text(
        &self,
        channel: &str,
        post_id: &str,
        integration_id: &str,
        auth: &serde_json::Value,
    ) -> Option<String> {
        let opts = CacheOpts {
            ttl_secs: POST_TEXT_CACHE_TTL,
            tags: vec![format!("integration:{channel}:{integration_id}")],
            ..CacheOpts::default()
        };
        let loaded = self
            .cache
            .get_or_load::<String, std::convert::Infallible, _>(
                &format!("post-text:{channel}:{post_id}"),
                opts,
                || async {
                    let text = if channel == "messenger" {
                        self.sources.messenger_post_message(auth, post_id).await
                    } else {
                        self.sources.instagram_post_caption(auth, post_id).await
                    };
                    Ok(text.ok().flatten())
                },
            )
            .await;
        match loaded {
            Ok(text) => text,
            Err(never) => match never {},
        }
    }

    /// `getLastCommentedPostText`: the message / caption of the post the
    /// contact last commented on (Messenger, Instagram), cached briefly.
    pub async fn get_last_commented_post_text(
        &self,
        contact_inbox: Option<&ContactInboxModel>,
        post_id: Option<&str>,
    ) -> Res<Option<String>> {
        let (Some(contact_inbox), Some(post_id)) =
            (contact_inbox, post_id.filter(|p| !p.is_empty()))
        else {
            return Ok(None);
        };
        let Some(inbox) = self
            .sources
            .find_inbox_with_integrations(&contact_inbox.inbox_id)
            .await?
        else {
            return Ok(None);
        };
        let row = match contact_inbox.channel.as_str() {
            "messenger" => inbox.integration_messenger.as_ref(),
            "instagram" => inbox.integration_instagram.as_ref(),
            _ => None,
        };
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(self
            .cached_post_text(&contact_inbox.channel, post_id, &row.id, &row.auth)
            .await)
    }

    /// `getIntegrationField`: the context inbox, else the contact's most
    /// recent one.
    pub async fn get_integration_field(
        &self,
        contact: &ContactModel,
        key: SystemFieldType,
        context_contact_inbox: Option<&ContactInboxModel>,
        conversation_id: Option<&str>,
    ) -> Res<Option<String>> {
        let recent;
        let contact_inbox = match context_contact_inbox {
            Some(contact_inbox) => contact_inbox,
            None => {
                recent = self
                    .sources
                    .find_recent_contact_inbox(&contact.workspace_id, &contact.id)
                    .await?;
                match &recent {
                    Some(contact_inbox) => contact_inbox,
                    None => return Ok(None),
                }
            }
        };
        let Some(inbox) = self
            .sources
            .find_inbox_with_integrations(&contact_inbox.inbox_id)
            .await?
        else {
            return Ok(None);
        };
        let channel = contact_inbox.channel.as_str();

        if is_profile_snapshot_field(key) {
            return Ok(self
                .resolve_profile_snapshot_field(contact_inbox, &inbox, key)
                .await);
        }

        match key {
            SystemFieldType::PageUserName => {
                Ok(inbox.integration(channel).and_then(|row| row.name.clone()))
            }
            SystemFieldType::FbChatLink => {
                let Some(row) = inbox
                    .integration_messenger
                    .as_ref()
                    .filter(|_| channel == "messenger")
                else {
                    return Ok(None);
                };
                let Some(page_id) = row.page_id.as_deref().filter(|p| !p.is_empty()) else {
                    return Ok(None);
                };
                let opts = CacheOpts {
                    ttl_secs: FB_CHAT_LINK_CACHE_TTL,
                    tags: vec![format!("integration:messenger:{}", row.id)],
                    ..CacheOpts::default()
                };
                Ok(self
                    .cache
                    .get_or_load::<String, SourceError, _>(
                        &format!("fb-chat-link:{page_id}:{}", contact_inbox.source_id),
                        opts,
                        || {
                            self.sources
                                .messenger_user_inbox_link(&row.auth, &contact_inbox.source_id)
                        },
                    )
                    .await?)
            }
            SystemFieldType::Webchat => {
                let Some(row) = inbox
                    .integration_webchat
                    .as_ref()
                    .filter(|_| channel == "webchat")
                else {
                    return Ok(None);
                };
                let settings = self.sources.tenant_settings(&contact.workspace_id).await?;
                Ok(Some(format!(
                    "{}/webchat?webchatId={}",
                    settings.app_url, row.id
                )))
            }
            SystemFieldType::InboxLink => {
                let settings = self.sources.tenant_settings(&contact.workspace_id).await?;
                let mut url = url::Url::parse(&settings.app_url)
                    .and_then(|base| base.join(&format!("/space/{}/inbox", contact.workspace_id)))
                    .map_err(|err| SourceError::new(format!("Invalid URL: {err}")))?;
                if let Some(conversation_id) = conversation_id.filter(|c| !c.is_empty()) {
                    url.query_pairs_mut()
                        .append_pair("conversationId", conversation_id);
                }
                Ok(Some(url.to_string()))
            }
            SystemFieldType::TimezoneName => {
                let Some(timezone) = contact.timezone.as_deref().filter(|t| !t.is_empty()) else {
                    return Ok(None);
                };
                Ok(Some(
                    normalize_stored_timezone(Some(timezone))
                        .unwrap_or_else(|| timezone.to_owned()),
                ))
            }
            SystemFieldType::UserCode => Ok(Some(contact_inbox.source_id.clone())),
            SystemFieldType::Me => {
                let Some(integration_id) = channel_integration_id(&inbox, channel) else {
                    return Ok(None);
                };
                let app_url = self
                    .sources
                    .workspace_app_url(&contact.workspace_id)
                    .await?;
                let form_id = self
                    .sources
                    .create_me_system_field(&MeSystemFieldPayload {
                        workspace_id: contact.workspace_id.clone(),
                        channel: channel.to_owned(),
                        integration_id: integration_id.clone(),
                        source_id: contact_inbox.source_id.clone(),
                        contact_inbox_id: contact_inbox.id.clone(),
                        conversation_id: conversation_id
                            .filter(|c| !c.is_empty())
                            .map(str::to_owned),
                        contact_id: contact.id.clone(),
                    })
                    .await?;
                let hash = self.encryption.sign_me_link(&MeLinkParams {
                    workspace_id: contact.workspace_id.clone(),
                    source_id: contact_inbox.source_id.clone(),
                    integration_id: integration_id.clone(),
                    form_id: form_id.clone(),
                });
                let query = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("w", &contact.workspace_id)
                    .append_pair("u", &contact_inbox.source_id)
                    .append_pair("ib", &integration_id)
                    .append_pair("id", &form_id)
                    .append_pair("hash", &hash)
                    .finish();
                Ok(Some(format!("{app_url}/extensions/me/?{query}")))
            }
            SystemFieldType::WaUserId | SystemFieldType::WaUserName => {
                if inbox.integration_whatsapp.is_none() {
                    return Ok(None);
                }
                Ok(if key == SystemFieldType::WaUserId {
                    contact_inbox.source_user_id.clone()
                } else {
                    contact_inbox.source_username.clone()
                })
            }
            _ => Ok(None),
        }
    }
}
