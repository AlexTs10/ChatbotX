//! `contact/profile-refresh` (`rules.ts`, `cooldown.ts`, `service.ts`) and
//! the builder lib `features/contacts/lib/refresh-contact-profile.ts`.

use std::future::Future;

use chatbotx_core::variables::system_field::{normalize_locale, normalize_stored_timezone};
use chatbotx_db::Db;
use chatbotx_db::repo::models::Contact;
use chatbotx_db::rusqlite::{Connection, params, params_from_iter, types::Value as SqlValue};
use chatbotx_files::Uploader;
use chatbotx_kv::MemKv;

use super::create::{load, repo_err};
use super::service::{find_or_fail, now_ms};
use super::update_fields::normalize_language;
use crate::{BusinessError, Result, tx};

/// `channelTypes.options`.
pub const CHANNEL_TYPES: &[&str] = &[
    "messenger",
    "instagram",
    "zalo",
    "telegram",
    "whatsapp",
    "tiktok",
    "threads",
    "api",
    "webchat",
    "smtp",
    "omnichannel",
];

/// `PROFILE_NAME_BLANK_CHARACTERS`: ECMAScript WhiteSpace + LineTerminator.
pub const PROFILE_NAME_BLANK_CHARACTERS: &str = "\u{0009}\u{000A}\u{000B}\u{000C}\u{000D}\u{0020}\u{00A0}\u{1680}\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}\u{FEFF}";

/// `PROFILE_REFRESH_COOLDOWN_SECONDS`.
pub const PROFILE_REFRESH_COOLDOWN_SECONDS: u64 = 5 * 60;

/// `ContactProfileNameSource`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileNameSource {
    Payload,
    ChannelApi,
}

/// `ContactProfileNameCapability`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    pub inbound: Option<ProfileNameSource>,
    pub on_demand: bool,
}

/// The fields of `IncomingContact` the refresh reads (`None` = absent).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IncomingProfile {
    pub source_id: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub avatar: Option<String>,
    pub locale: Option<String>,
    pub language: Option<String>,
    pub timezone: Option<String>,
    pub gender: Option<String>,
}

/// `ContactProfileUpdate` (`None` = not written).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactProfileUpdate {
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub avatar: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub gender: Option<String>,
}

/// `skipped.reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    ProfileComplete,
    CoolingDown,
    ChannelNotCapable,
}

/// `ContactProfileRefreshResult`.
#[derive(Debug, Clone, PartialEq)]
pub enum RefreshOutcome {
    Updated(Box<Contact>),
    Skipped(SkipReason),
    Unavailable,
    Failed,
}

/// The contact inbox fields of `RefreshContactProfileInput`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshContactInbox {
    pub id: i64,
    pub channel: String,
    pub contact_id: i64,
    pub language: Option<String>,
    pub source_id: String,
}

/// `RefreshContactProfileInput` (without the fetcher).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshInput {
    pub workspace_id: i64,
    pub contact_id: i64,
    pub contact_inbox: RefreshContactInbox,
    pub source: ProfileNameSource,
}

/// The stores of the refresh: the cooldown keys (`distributedStore`) and the
/// avatar objects (`uploader`). `None` = the store is off.
#[derive(Clone, Copy, Default)]
pub struct RefreshServices<'a> {
    pub kv: Option<&'a MemKv>,
    pub uploader: Option<&'a Uploader>,
}

/// What a profile fetcher returns: a profile, nothing, or an error.
pub type FetchResult = std::result::Result<Option<IncomingProfile>, String>;

/// `String.prototype.trim` (the [`PROFILE_NAME_BLANK_CHARACTERS`] set).
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(|c| PROFILE_NAME_BLANK_CHARACTERS.contains(c))
}

fn blank(value: Option<&str>) -> bool {
    value.is_none_or(|text| js_trim(text).is_empty())
}

/// `hasEmptyProfileName`.
pub fn has_empty_profile_name(first_name: Option<&str>, last_name: Option<&str>) -> bool {
    blank(first_name) && blank(last_name)
}

/// `hasProfileName`.
pub fn has_profile_name(update: &ContactProfileUpdate) -> bool {
    !has_empty_profile_name(update.first_name.as_deref(), update.last_name.as_deref())
}

/// `buildContactProfileUpdate` (`gender` only when `genderTypes` accepts it).
pub fn build_contact_profile_update(profile: &IncomingProfile) -> ContactProfileUpdate {
    let gender = profile
        .gender
        .clone()
        .filter(|g| matches!(g.as_str(), "male" | "female" | "unknown"));
    ContactProfileUpdate {
        first_name: profile.first_name.clone(),
        last_name: profile.last_name.clone(),
        avatar: profile.avatar.clone(),
        locale: profile.locale.clone(),
        timezone: profile.timezone.clone(),
        gender,
    }
}

/// `contactProfileNameCapabilities[channel]`.
pub fn capability(channel: &str) -> Option<Capability> {
    use ProfileNameSource::{ChannelApi, Payload};
    let (inbound, on_demand) = match channel {
        "messenger" | "instagram" | "zalo" | "telegram" => (Some(ChannelApi), true),
        "whatsapp" | "api" => (Some(Payload), false),
        "tiktok" | "threads" | "webchat" | "smtp" | "omnichannel" => (None, false),
        _ => return None,
    };
    Some(Capability { inbound, on_demand })
}

/// `resolveInboundProfileNameSource`.
pub fn resolve_inbound_profile_name_source(channel: &str) -> Option<ProfileNameSource> {
    capability(channel).and_then(|c| c.inbound)
}

/// `hasOnDemandProfileApi`.
pub fn has_on_demand_profile_api(channel: &str) -> bool {
    capability(channel).is_some_and(|c| c.on_demand)
}

/// `COOLDOWN_BY_PROFILE_SOURCE`.
pub fn cooldown_by_profile_source(source: ProfileNameSource) -> bool {
    source == ProfileNameSource::ChannelApi
}

fn cooldown_key(contact_inbox_id: i64) -> String {
    format!("contact-profile-refresh:cooldown:{contact_inbox_id}")
}

/// `isContactProfileRefreshCoolingDown` (no store = not cooling down).
pub fn is_cooling_down(kv: Option<&MemKv>, contact_inbox_id: i64) -> bool {
    kv.is_some_and(|kv| kv.exists(&cooldown_key(contact_inbox_id)))
}

/// `startContactProfileRefreshCooldown` (`setNumber(key, now, 300)`).
pub fn start_cooldown(kv: Option<&MemKv>, contact_inbox_id: i64) {
    if let Some(kv) = kv {
        #[allow(clippy::cast_precision_loss)] // epoch ms fit f64 exactly.
        let now = now_ms() as f64;
        kv.set_number(
            &cooldown_key(contact_inbox_id),
            now,
            PROFILE_REFRESH_COOLDOWN_SECONDS,
        );
    }
}

/// `isManagedAvatarObject`.
fn is_managed_avatar_object(avatar: &str) -> bool {
    !(avatar.starts_with("http://") || avatar.starts_with("https://"))
        && avatar.contains("/avatars/")
}

/// `discardUploadedAvatar` (best effort).
async fn discard_uploaded_avatar(uploader: Option<&Uploader>, avatar: Option<&str>) {
    let (Some(uploader), Some(avatar)) = (uploader, avatar) else {
        return;
    };
    if !is_managed_avatar_object(avatar) {
        return;
    }
    if let Err(error) = uploader.delete_object(avatar).await {
        tracing::warn!(%error, avatar, "discardUploadedAvatar: failed to delete avatar object");
    }
}

fn start_cooldown_if_applicable(kv: Option<&MemKv>, source: ProfileNameSource, id: i64) {
    if cooldown_by_profile_source(source) {
        start_cooldown(kv, id);
    }
}

/// `recordProfileRefreshFailure`: an Error-Log row for the channel (logged
/// as a warning as well).
async fn record_profile_refresh_failure(
    channel: &str,
    workspace_id: i64,
    contact_id: i64,
    source_id: &str,
    error: &str,
) {
    tracing::warn!(
        channel,
        workspace_id,
        contact_id,
        error,
        "contact profile refresh failed"
    );
    let mut input = crate::error_log::ProviderErrorInput::new(
        channel,
        &workspace_id.to_string(),
        crate::error_log::Thrown::Text(error.to_owned()),
    );
    input.contact_id = Some(contact_id.to_string());
    input.source_id = Some(source_id.to_owned());
    crate::error_log::log_provider_error_for_channel(
        crate::events::global::buses(),
        channel,
        input,
    )
    .await;
}

/// `normalizeChannelApiProfile`: the locale written is the normalized one;
/// the timezone only when the channel's value normalizes; the language for
/// the contact inbox.
fn normalize_channel_api_profile(
    profile: &IncomingProfile,
    mut update: ContactProfileUpdate,
) -> (ContactProfileUpdate, Option<String>) {
    let locale = normalize_locale(update.locale.as_deref());
    let timezone = update
        .timezone
        .as_deref()
        .and_then(|raw| normalize_stored_timezone(Some(raw)));
    let language = normalize_language(profile.language.as_deref())
        .or_else(|| normalize_language(locale.as_deref()));
    if update.locale.is_some() {
        update.locale = locale;
    }
    update.timezone = timezone;
    (update, language)
}

/// `contactService.updateIfProfileNameEmpty` (`only_if_name_empty`: the
/// write only lands while both names are still blank; `None` = a name was set
/// first) or `contactService.update` (always written).
fn update_if_profile_name_empty(
    tx: &Connection,
    workspace_id: i64,
    id: i64,
    update: &ContactProfileUpdate,
    only_if_name_empty: bool,
) -> Result<Option<Contact>> {
    find_or_fail(tx, workspace_id, id)?;
    let columns: Vec<(&str, &Option<String>)> = vec![
        ("firstName", &update.first_name),
        ("lastName", &update.last_name),
        ("avatar", &update.avatar),
        ("locale", &update.locale),
        ("timezone", &update.timezone),
        ("gender", &update.gender),
    ];
    let set: Vec<(&str, String)> = columns
        .into_iter()
        .filter_map(|(column, value)| value.clone().map(|v| (column, v)))
        .collect();
    let mut sql = String::from(r#"UPDATE "Contact" SET "updatedAt" = ?1"#);
    let mut values: Vec<SqlValue> = vec![now_ms().into(), id.into(), workspace_id.into()];
    values.push(PROFILE_NAME_BLANK_CHARACTERS.to_string().into());
    for (i, (column, value)) in set.iter().enumerate() {
        sql.push_str(&format!(r#", "{column}" = ?{}"#, i + 5));
        values.push(value.clone().into());
    }
    sql.push_str(r#" WHERE "id" = ?2 AND "workspaceId" = ?3"#);
    // `?4` is always bound, so the unconditional write reads it too.
    sql.push_str(if only_if_name_empty {
        r#" AND trim(coalesce("firstName", ''), ?4) = '' AND trim(coalesce("lastName", ''), ?4) = ''"#
    } else {
        " AND ?4 IS NOT NULL"
    });
    let changed = tx
        .execute(&sql, params_from_iter(values))
        .map_err(repo_err)?;
    if changed == 0 {
        return Ok(None);
    }
    load::<Contact>(tx, id).map(Some)
}

/// `contactInboxService.updateLanguageIfEmpty`.
fn update_language_if_empty(
    tx: &Connection,
    contact_id: i64,
    contact_inbox_id: i64,
    language: &str,
) -> Result<()> {
    tx.execute(
        r#"UPDATE "ContactInbox" SET "language" = ?3, "updatedAt" = ?4
           WHERE "id" = ?1 AND "contactId" = ?2 AND trim(coalesce("language", ''), ?5) = ''"#,
        params![
            contact_inbox_id,
            contact_id,
            language,
            now_ms(),
            PROFILE_NAME_BLANK_CHARACTERS
        ],
    )
    .map_err(repo_err)?;
    Ok(())
}

/// `applyContactProfileIfNameEmpty`: the conditional write, then the
/// superseded managed avatar is deleted (best effort).
async fn apply_contact_profile_if_name_empty(
    db: &Db,
    uploader: Option<&Uploader>,
    workspace_id: i64,
    contact_id: i64,
    update: ContactProfileUpdate,
) -> Result<Option<Contact>> {
    write_profile(db, uploader, workspace_id, contact_id, update, true).await
}

/// `applyContactProfile`: the write (current values overwritten), then the
/// superseded managed avatar is deleted (best effort).
pub async fn apply_contact_profile(
    db: &Db,
    uploader: Option<&Uploader>,
    workspace_id: i64,
    contact_id: i64,
    update: ContactProfileUpdate,
) -> Result<Option<Contact>> {
    write_profile(db, uploader, workspace_id, contact_id, update, false).await
}

async fn write_profile(
    db: &Db,
    uploader: Option<&Uploader>,
    workspace_id: i64,
    contact_id: i64,
    update: ContactProfileUpdate,
    only_if_name_empty: bool,
) -> Result<Option<Contact>> {
    let new_avatar = update.avatar.clone();
    let (previous_avatar, updated) = tx::write(db, move |tx| {
        let previous = if update.avatar.is_some() {
            find_or_fail(tx, workspace_id, contact_id)?.avatar
        } else {
            None
        };
        let updated = update_if_profile_name_empty(
            tx,
            workspace_id,
            contact_id,
            &update,
            only_if_name_empty,
        )?;
        Ok((previous, updated))
    })
    .await?;
    if updated.is_some()
        && let (Some(previous), Some(uploader)) = (previous_avatar, uploader)
        && Some(&previous) != new_avatar.as_ref()
        && is_managed_avatar_object(&previous)
        && let Err(error) = uploader.delete_object(&previous).await
    {
        tracing::warn!(%error, path = previous, "applyProfile: failed to delete superseded avatar");
    }
    Ok(updated)
}

/// The `updateMessengerContactData` step: the fetched profile overwrites the
/// contact's current values (even a set name). Best effort: no profile, an
/// empty one, or a fetch or write error is a no-op (errors are logged).
pub async fn overwrite_contact_profile<F, Fut>(
    db: &Db,
    uploader: Option<&Uploader>,
    workspace_id: i64,
    contact_id: i64,
    fetch_profile: F,
) where
    F: FnOnce() -> Fut,
    Fut: Future<Output = FetchResult>,
{
    let profile = match fetch_profile().await {
        Ok(Some(profile)) => profile,
        Ok(None) => return,
        Err(error) => {
            tracing::error!(error, "updateMessengerContactData failed");
            return;
        }
    };
    let update = build_contact_profile_update(&profile);
    if update == ContactProfileUpdate::default() {
        return;
    }
    if let Err(error) = apply_contact_profile(db, uploader, workspace_id, contact_id, update).await
    {
        tracing::error!(%error, "updateMessengerContactData failed");
    }
}

/// `contactProfileRefreshService.refresh`.
pub async fn refresh<F, Fut>(
    db: &Db,
    services: RefreshServices<'_>,
    input: RefreshInput,
    fetch_profile: F,
) -> Result<RefreshOutcome>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = FetchResult>,
{
    let RefreshInput {
        workspace_id,
        contact_id,
        contact_inbox,
        source,
    } = input;
    if source == ProfileNameSource::ChannelApi && !has_on_demand_profile_api(&contact_inbox.channel)
    {
        return Ok(RefreshOutcome::Skipped(SkipReason::ChannelNotCapable));
    }
    let contact = tx::read(db, move |conn| find_or_fail(conn, workspace_id, contact_id)).await?;
    if !has_empty_profile_name(contact.first_name.as_deref(), contact.last_name.as_deref()) {
        return Ok(RefreshOutcome::Skipped(SkipReason::ProfileComplete));
    }
    if cooldown_by_profile_source(source) && is_cooling_down(services.kv, contact_inbox.id) {
        return Ok(RefreshOutcome::Skipped(SkipReason::CoolingDown));
    }
    let profile = match fetch_profile().await {
        Ok(profile) => profile,
        Err(error) => {
            record_profile_refresh_failure(
                &contact_inbox.channel,
                workspace_id,
                contact_id,
                &contact_inbox.source_id,
                &error,
            )
            .await;
            start_cooldown_if_applicable(services.kv, source, contact_inbox.id);
            return Ok(RefreshOutcome::Failed);
        }
    };
    let Some(profile) = profile else {
        start_cooldown_if_applicable(services.kv, source, contact_inbox.id);
        return Ok(RefreshOutcome::Unavailable);
    };
    let update = build_contact_profile_update(&profile);
    if !has_profile_name(&update) {
        discard_uploaded_avatar(services.uploader, update.avatar.as_deref()).await;
        start_cooldown_if_applicable(services.kv, source, contact_inbox.id);
        return Ok(RefreshOutcome::Unavailable);
    }
    let (update, language) = if source == ProfileNameSource::ChannelApi {
        normalize_channel_api_profile(&profile, update)
    } else {
        (update, None)
    };
    let avatar = update.avatar.clone();
    match apply_contact_profile_if_name_empty(
        db,
        services.uploader,
        workspace_id,
        contact_id,
        update,
    )
    .await
    {
        Ok(None) => {
            discard_uploaded_avatar(services.uploader, avatar.as_deref()).await;
            Ok(RefreshOutcome::Skipped(SkipReason::ProfileComplete))
        }
        Ok(Some(updated)) => {
            if let Some(language) = language.filter(|_| blank(contact_inbox.language.as_deref())) {
                let (inbox_contact, inbox_id) = (contact_inbox.contact_id, contact_inbox.id);
                let written = tx::write(db, move |tx| {
                    update_language_if_empty(tx, inbox_contact, inbox_id, &language)
                })
                .await;
                if let Err(error) = written {
                    tracing::warn!(%error, contact_inbox_id = inbox_id, "writeContactInboxLanguageIfEmpty failed");
                }
            }
            Ok(RefreshOutcome::Updated(Box::new(updated)))
        }
        Err(error) => {
            discard_uploaded_avatar(services.uploader, avatar.as_deref()).await;
            record_profile_refresh_failure(
                &contact_inbox.channel,
                workspace_id,
                contact_id,
                &contact_inbox.source_id,
                &error.to_string(),
            )
            .await;
            start_cooldown_if_applicable(services.kv, source, contact_inbox.id);
            Ok(RefreshOutcome::Failed)
        }
    }
}

/// `refreshContactProfile` (builder lib): the contact (`Contact not found`),
/// its contact inbox (`Contact inbox not found`), then [`refresh`] with the
/// channel fetcher (`channelApi`).
pub async fn refresh_contact_profile(
    db: &Db,
    services: RefreshServices<'_>,
    workspace_id: i64,
    contact_id: i64,
    contact_inbox_id: i64,
) -> Result<RefreshOutcome> {
    let http = crate::channel_api::LiveChannelHttp;
    refresh_contact_profile_with(
        db,
        services,
        &http,
        workspace_id,
        contact_id,
        contact_inbox_id,
    )
    .await
}

/// [`refresh_contact_profile`] over `http` (the channel provider calls).
pub async fn refresh_contact_profile_with(
    db: &Db,
    services: RefreshServices<'_>,
    http: &dyn crate::channel_api::ChannelHttp,
    workspace_id: i64,
    contact_id: i64,
    contact_inbox_id: i64,
) -> Result<RefreshOutcome> {
    let (contact_inbox, inbox_id) = tx::read(db, move |conn| {
        find_or_fail(conn, workspace_id, contact_id)?;
        conn.query_row(
            r#"SELECT "channel", "language", "sourceId", "inboxId" FROM "ContactInbox" WHERE "id" = ?1 AND "contactId" = ?2"#,
            params![contact_inbox_id, contact_id],
            |row| {
                Ok((
                    RefreshContactInbox {
                        id: contact_inbox_id,
                        channel: row.get(0)?,
                        contact_id,
                        language: row.get(1)?,
                        source_id: row.get(2)?,
                    },
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .map_err(|error| match error {
            chatbotx_db::rusqlite::Error::QueryReturnedNoRows => {
                BusinessError::not_found("Contact inbox not found")
            }
            other => repo_err(other),
        })
    })
    .await?;
    let (channel, source_id) = (
        contact_inbox.channel.clone(),
        contact_inbox.source_id.clone(),
    );
    let deps = super::channel_profile::ProfileDeps {
        db,
        http,
        uploader: services.uploader,
    };
    let input = RefreshInput {
        workspace_id,
        contact_id,
        contact_inbox,
        source: ProfileNameSource::ChannelApi,
    };
    // `profileFetcherFactories[channel]`: the channel's `contact.getProfile`.
    let fetch = || async move {
        super::channel_profile::fetch_profile(
            deps,
            workspace_id,
            &channel,
            inbox_id,
            &source_id,
            true,
        )
        .await
        .map(Some)
    };
    refresh(db, services, input, fetch).await
}
