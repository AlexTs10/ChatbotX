//! `contact/profile-refresh` (`rules.ts`, `cooldown.ts`, `service.ts`) and
//! the builder lib `features/contacts/lib/refresh-contact-profile.ts`.

use std::future::Future;

use chatbotx_db::Db;
use chatbotx_db::repo::models::Contact;
use chatbotx_files::Uploader;
use chatbotx_kv::MemKv;

use crate::Result;

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
    let _ = text;
    todo!("P3.6-D2 refresh profile stage 2")
}

/// `hasEmptyProfileName`.
pub fn has_empty_profile_name(first_name: Option<&str>, last_name: Option<&str>) -> bool {
    let _ = (first_name, last_name);
    todo!("P3.6-D2 refresh profile stage 2")
}

/// `hasProfileName`.
pub fn has_profile_name(update: &ContactProfileUpdate) -> bool {
    let _ = update;
    todo!("P3.6-D2 refresh profile stage 2")
}

/// `buildContactProfileUpdate`.
pub fn build_contact_profile_update(profile: &IncomingProfile) -> ContactProfileUpdate {
    let _ = profile;
    todo!("P3.6-D2 refresh profile stage 2")
}

/// `contactProfileNameCapabilities[channel]`.
pub fn capability(channel: &str) -> Option<Capability> {
    let _ = channel;
    todo!("P3.6-D2 refresh profile stage 2")
}

/// `resolveInboundProfileNameSource`.
pub fn resolve_inbound_profile_name_source(channel: &str) -> Option<ProfileNameSource> {
    let _ = channel;
    todo!("P3.6-D2 refresh profile stage 2")
}

/// `hasOnDemandProfileApi`.
pub fn has_on_demand_profile_api(channel: &str) -> bool {
    let _ = channel;
    todo!("P3.6-D2 refresh profile stage 2")
}

/// `COOLDOWN_BY_PROFILE_SOURCE`.
pub fn cooldown_by_profile_source(source: ProfileNameSource) -> bool {
    let _ = source;
    todo!("P3.6-D2 refresh profile stage 2")
}

/// `isContactProfileRefreshCoolingDown`.
pub fn is_cooling_down(kv: Option<&MemKv>, contact_inbox_id: i64) -> bool {
    let _ = (kv, contact_inbox_id);
    todo!("P3.6-D2 refresh profile stage 2")
}

/// `startContactProfileRefreshCooldown`.
pub fn start_cooldown(kv: Option<&MemKv>, contact_inbox_id: i64) {
    let _ = (kv, contact_inbox_id);
    todo!("P3.6-D2 refresh profile stage 2")
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
    let _ = (db, services, input, fetch_profile);
    todo!("P3.6-D2 refresh profile stage 2")
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
    let _ = (db, services, workspace_id, contact_id, contact_inbox_id);
    todo!("P3.6-D2 refresh profile stage 2")
}
