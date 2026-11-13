//! The provider metadata of `CONNECTION_REGISTRY` (`kind`, `strategy`,
//! capabilities) and the enums of the connection requests.

use crate::BusinessError;

/// `integrationTypes`.
pub const INTEGRATION_TYPES: &[&str] = &[
    "activeCampaign",
    "api",
    "chatbotx",
    "claude",
    "deepseek",
    "drip",
    "facebookAds",
    "gemini",
    "getResponse",
    "klaviyo",
    "googleAds",
    "googleCalendar",
    "googleSheets",
    "instagram",
    "instagramFacebook",
    "threads",
    "mailchimp",
    "mailerLite",
    "metaCatalog",
    "messenger",
    "moosend",
    "muse",
    "openai",
    "openaiCompatible",
    "openrouter",
    "outlookCalendar",
    "sendGrid",
    "smtp",
    "telegram",
    "tiktok",
    "webchat",
    "whatsapp",
    "zalo",
];

/// `channelTypes`.
pub const CHANNEL_TYPES: &[&str] = &[
    "omnichannel",
    "webchat",
    "messenger",
    "whatsapp",
    "zalo",
    "smtp",
    "telegram",
    "instagram",
    "threads",
    "tiktok",
    "api",
];

/// `connectionStatuses`.
pub const CONNECTION_STATUSES: &[&str] = &[
    "connected",
    "degraded",
    "needs_reauth",
    "paused",
    "disconnected",
];

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
pub fn provider_meta(provider: &str) -> Option<ProviderMeta> {
    const fn meta(
        kind: &'static str,
        strategy: &'static str,
        multi_account: bool,
        refreshable: bool,
        verifiable: bool,
    ) -> Option<ProviderMeta> {
        Some(ProviderMeta {
            kind,
            strategy,
            multi_account,
            refreshable,
            verifiable,
        })
    }
    match provider {
        // `selfServeConnection` (kind channel; `verify` always ok).
        "api" | "chatbotx" | "smtp" | "webchat" => {
            meta("channel", "self_serve", false, false, true)
        }
        "whatsapp" => meta("channel", "self_serve", false, true, true),
        "telegram" => meta("channel", "token", false, false, true),
        "messenger" | "instagramFacebook" => meta("channel", "oauth_redirect", true, true, true),
        "instagram" | "threads" | "tiktok" | "zalo" => {
            meta("channel", "oauth_redirect", false, true, true)
        }
        "googleAds" => meta("integration", "oauth_redirect", true, true, true),
        "googleCalendar" => meta("integration", "oauth_redirect", false, true, true),
        "facebookAds" | "googleSheets" => meta("integration", "oauth_redirect", false, false, true),
        // The AI key providers (`credential-providers.ts`).
        "claude" | "deepseek" | "gemini" | "muse" | "openai" | "openaiCompatible"
        | "openrouter" => meta("integration", "api_key", false, false, true),
        // The email / marketing integrations (credential connects).
        "activeCampaign" | "drip" | "getResponse" | "klaviyo" | "mailchimp" | "mailerLite"
        | "moosend" | "sendGrid" => meta("integration", "api_key", false, false, false),
        _ => None,
    }
}

/// `toChannelType`.
pub fn to_channel_type(provider: &str) -> &'static str {
    match provider {
        "instagramFacebook" => "instagram",
        other => CHANNEL_TYPES
            .iter()
            .find(|channel| **channel == other)
            .copied()
            .unwrap_or("omnichannel"),
    }
}

fn required_scope_for_kind(kind: &str) -> &'static str {
    if kind == "channel" {
        "channels"
    } else {
        "integrations"
    }
}

fn forbidden(scope: &str) -> BusinessError {
    BusinessError::exception(
        format!("Token is not authorized for the '{scope}' scope"),
        "FORBIDDEN",
        403,
    )
}

/// `assertTokenScopeForProvider`: `scopes = None` = every scope.
pub fn assert_token_scope_for_provider(
    scopes: Option<&[String]>,
    provider: &str,
) -> Result<(), BusinessError> {
    let Some(scopes) = scopes else {
        return Ok(());
    };
    let kind = provider_meta(provider).map_or("integration", |m| m.kind);
    let required = required_scope_for_kind(kind);
    if scopes.iter().any(|scope| scope == required) {
        Ok(())
    } else {
        Err(forbidden(required))
    }
}

/// `resolveListKind`.
pub fn resolve_list_kind(
    scopes: Option<&[String]>,
    requested: Option<&str>,
) -> Result<Option<String>, BusinessError> {
    let Some(scopes) = scopes else {
        return Ok(requested.map(str::to_string));
    };
    let mut allowed = Vec::new();
    if scopes.iter().any(|scope| scope == "channels") {
        allowed.push("channel");
    }
    if scopes.iter().any(|scope| scope == "integrations") {
        allowed.push("integration");
    }
    if let Some(requested) = requested {
        if !allowed.contains(&requested) {
            return Err(forbidden(required_scope_for_kind(requested)));
        }
        return Ok(Some(requested.to_string()));
    }
    Ok(match allowed.as_slice() {
        [only] => Some((*only).to_string()),
        _ => None,
    })
}

/// `assertGenericProviderAllowed` (`googleAds` is dedicated-only).
pub fn assert_generic_provider_allowed(provider: &str) -> Result<(), BusinessError> {
    if provider == "googleAds" {
        return Err(BusinessError::exception(
            format!(
                "Connection provider \"{provider}\" can only be managed from its dedicated settings page."
            ),
            "connectionProviderDedicatedOnly",
            403,
        ));
    }
    Ok(())
}
