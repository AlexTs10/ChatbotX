//! `packages/utils/src/error-log.ts`: the closed set of `ErrorLog.action`
//! providers, their labels, and stack-frame capture.

use std::sync::OnceLock;

use regex::Regex;

use super::channel::ChannelType;

/// Non-channel providers, in declaration order, with their labels.
const OTHER_PROVIDERS: &[(&str, &str)] = &[
    ("meta-catalog", "Meta catalog"),
    ("meta-conversions", "Meta conversions"),
    ("facebook-ads", "Facebook ads"),
    ("mailchimp", "Mailchimp"),
    ("sendgrid", "SendGrid"),
    ("active-campaign", "ActiveCampaign"),
    ("klaviyo", "Klaviyo"),
    ("get-response", "GetResponse"),
    ("drip", "Drip"),
    ("mailer-lite", "MailerLite"),
    ("moosend", "Moosend"),
    ("google-sheets", "Google sheets"),
    ("external-request", "External request"),
    ("google-calendar", "Google calendar"),
    ("google-ads", "Google Ads"),
    ("openai", "OpenAI"),
    ("gemini", "Gemini"),
    ("claude", "Claude"),
    ("deepseek", "DeepSeek"),
    ("openrouter", "OpenRouter"),
    ("muse", "Muse"),
    ("openai-compatible", "OpenAI Compatible"),
];

/// Channel labels, in `errorLogProviderLabels` declaration order.
const CHANNEL_LABELS: &[(&str, &str)] = &[
    ("webchat", "Webchat"),
    ("messenger", "Messenger"),
    ("whatsapp", "WhatsApp"),
    ("zalo", "Zalo"),
    ("smtp", "Email"),
    ("telegram", "Telegram"),
    ("instagram", "Instagram"),
    ("threads", "Threads"),
    ("tiktok", "TikTok"),
    ("api", "API"),
];

/// `errorLogProviders.options`: every channel but `omnichannel`, then the
/// third parties.
pub fn error_log_providers() -> &'static [&'static str] {
    static ALL: OnceLock<Vec<&'static str>> = OnceLock::new();
    ALL.get_or_init(|| {
        ChannelType::ALL
            .iter()
            .filter(|c| **c != ChannelType::Omnichannel)
            .map(|c| c.as_str())
            .chain(OTHER_PROVIDERS.iter().map(|(p, _)| *p))
            .collect()
    })
}

/// `errorLogProviders.safeParse(value).success`.
pub fn is_error_log_provider(value: &str) -> bool {
    error_log_providers().contains(&value)
}

/// `errorLogProviderLabels`, in declaration order.
pub fn error_log_provider_labels() -> impl Iterator<Item = (&'static str, &'static str)> {
    CHANNEL_LABELS.iter().chain(OTHER_PROVIDERS).copied()
}

/// `errorLogProviderLabel`: the display name, or the raw value.
pub fn error_log_provider_label(action: &str) -> String {
    error_log_provider_labels()
        .find(|(provider, _)| *provider == action)
        .map_or_else(|| action.to_owned(), |(_, label)| label.to_owned())
}

/// `errorLogProvidersMatchingLabel`: providers whose label contains the
/// keyword (trimmed, case-insensitive).
pub fn error_log_providers_matching_label(keyword: &str) -> Vec<&'static str> {
    let needle = crate::js::trim(keyword).to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    error_log_provider_labels()
        .filter(|(_, label)| label.to_lowercase().contains(&needle))
        .map(|(provider, _)| provider)
        .collect()
}

/// `MAX_STACK_LENGTH`.
pub const MAX_STACK_LENGTH: usize = 2048;

/// `resolveStackFrames`: the frame lines of a stack (after the message),
/// capped at [`MAX_STACK_LENGTH`] UTF-16 units; `None` without frames.
pub fn resolve_stack_frames(message: &str, stack: Option<&str>) -> Option<String> {
    static FRAME: OnceLock<Regex> = OnceLock::new();
    let frame = FRAME.get_or_init(|| Regex::new(r"(?m)^[ \t]+at ").expect("valid regex"));
    let stack = stack?;
    let message_end = if message.is_empty() {
        None
    } else {
        stack.find(message)
    };
    let search_from = message_end.map_or(0, |i| i + message.len());
    let found = frame.find(&stack[search_from..])?;
    let start = search_from + found.start();
    Some(crate::js::slice_utf16(&stack[start..], 0, MAX_STACK_LENGTH).to_owned())
}
