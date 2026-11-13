//! The provider metadata of `CONNECTION_REGISTRY` (`kind`, `strategy`,
//! capabilities) and the enums of the connection requests.

use crate::BusinessError;

/// `integrationTypes`.
pub const INTEGRATION_TYPES: &[&str] = &[
    "activeCampaign", "api", "chatbotx", "claude", "deepseek", "drip", "facebookAds", "gemini",
    "getResponse", "klaviyo", "googleAds", "googleCalendar", "googleSheets", "instagram",
    "instagramFacebook", "threads", "mailchimp", "mailerLite", "metaCatalog", "messenger",
    "moosend", "muse", "openai", "openaiCompatible", "openrouter", "outlookCalendar", "sendGrid",
    "smtp", "telegram", "tiktok", "webchat", "whatsapp", "zalo",
];

/// `channelTypes`.
pub const CHANNEL_TYPES: &[&str] = &[
    "omnichannel", "webchat", "messenger", "whatsapp", "zalo", "smtp", "telegram", "instagram",
    "threads", "tiktok", "api",
];

/// `connectionStatuses`.
pub const CONNECTION_STATUSES: &[&str] =
    &["connected", "degraded", "needs_reauth", "paused", "disconnected"];

/// `connectionKinds`.
pub const CONNECTION_KINDS: &[&str] = &["channel", "integration"];

/// One registry entry's provider metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderMeta {
    pub kind: &'static str,
    pub strategy: &'static str,
    pub multi_account: bool,
    pub refreshable: bool,
    pub verifiable: bool,
}

/// `CONNECTION_REGISTRY[provider]?.provider`; `None` = no adapter
/// (`metaCatalog`, `outlookCalendar`, or an unknown provider).
pub fn provider_meta(_provider: &str) -> Option<ProviderMeta> {
    todo!("P3.6-D2")
}

/// `toChannelType`.
pub fn to_channel_type(_provider: &str) -> &'static str {
    todo!("P3.6-D2")
}

/// `assertTokenScopeForProvider`: `scopes = None` = every scope.
pub fn assert_token_scope_for_provider(
    _scopes: Option<&[String]>,
    _provider: &str,
) -> Result<(), BusinessError> {
    todo!("P3.6-D2")
}

/// `resolveListKind`.
pub fn resolve_list_kind(
    _scopes: Option<&[String]>,
    _requested: Option<&str>,
) -> Result<Option<String>, BusinessError> {
    todo!("P3.6-D2")
}

/// `assertGenericProviderAllowed` (`googleAds` is dedicated-only).
pub fn assert_generic_provider_allowed(_provider: &str) -> Result<(), BusinessError> {
    todo!("P3.6-D2")
}
