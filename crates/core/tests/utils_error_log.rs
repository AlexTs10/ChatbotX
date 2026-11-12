//! Port of `packages/utils/__tests__/error-log.test.ts`.

use chatbotx_core::utils::channel::ChannelType;
use chatbotx_core::utils::error_log::{
    error_log_provider_label, error_log_provider_labels, error_log_providers,
    error_log_providers_matching_label, is_error_log_provider,
};

/// Port of: packages/utils/__tests__/error-log.test.ts:11 "errorLogProviders > accepts every connectable channel type"
#[test]
fn errorlogproviders_accepts_every_connectable_channel_type() {
    for channel in ChannelType::ALL {
        if *channel == ChannelType::Omnichannel {
            continue;
        }
        assert!(is_error_log_provider(channel.as_str()), "{channel}");
    }
}

/// Port of: packages/utils/__tests__/error-log.test.ts:20 "errorLogProviders > rejects omnichannel, which is the unknown-channel fallback label"
#[test]
fn errorlogproviders_rejects_omnichannel_which_is_the_unknown_channel_fallback_label() {
    assert!(!is_error_log_provider("omnichannel"));
}

/// Port of: packages/utils/__tests__/error-log.test.ts:24 "errorLogProviders > rejects a value that is not a known provider"
#[test]
fn errorlogproviders_rejects_a_value_that_is_not_a_known_provider() {
    for value in ["Messenger", "fb", "system", "facebook-lead-ads"] {
        assert!(!is_error_log_provider(value), "{value}");
    }
}

/// Port of: packages/utils/__tests__/error-log.test.ts:34 "errorLogProviders > rejects instagram-facebook, which collapses into instagram"
#[test]
fn errorlogproviders_rejects_instagram_facebook_which_collapses_into_instagram() {
    assert!(!is_error_log_provider("instagram-facebook"));
    assert!(is_error_log_provider("instagram"));
}

/// Port of: packages/utils/__tests__/error-log.test.ts:43 "errorLogProviders > accepts every AI vendor an AI step can run against"
#[test]
fn errorlogproviders_accepts_every_ai_vendor_an_ai_step_can_run_against() {
    for provider in [
        "openai",
        "gemini",
        "claude",
        "deepseek",
        "openrouter",
        "muse",
        "openai-compatible",
    ] {
        assert!(is_error_log_provider(provider), "{provider}");
    }
}

/// Port of: packages/utils/__tests__/error-log.test.ts:57 "errorLogProviders > accepts the non-channel third parties"
#[test]
fn errorlogproviders_accepts_the_non_channel_third_parties() {
    for provider in [
        "mailchimp",
        "openai",
        "google-sheets",
        "google-calendar",
        "google-ads",
        "meta-catalog",
    ] {
        assert!(is_error_log_provider(provider), "{provider}");
    }
}

/// Port of: packages/utils/__tests__/error-log.test.ts:72 "errorLogProviderLabel > labels every provider with a non-slug display name"
#[test]
fn errorlogproviderlabel_labels_every_provider_with_a_non_slug_display_name() {
    for provider in error_log_providers() {
        let label = error_log_provider_labels()
            .find(|(p, _)| p == provider)
            .map(|(_, label)| label)
            .unwrap_or_default();
        assert!(!label.is_empty(), "{provider}");
        assert!(!label.contains('-'), "{provider}: {label}");
    }
}

/// Port of: packages/utils/__tests__/error-log.test.ts:80 "errorLogProviderLabel > humanises the stored slug"
#[test]
fn errorlogproviderlabel_humanises_the_stored_slug() {
    assert_eq!(error_log_provider_label("webchat"), "Webchat");
    assert_eq!(error_log_provider_label("facebook-ads"), "Facebook ads");
    assert_eq!(
        error_log_provider_label("google-calendar"),
        "Google calendar"
    );
    assert_eq!(error_log_provider_label("google-ads"), "Google Ads");
    assert_eq!(error_log_provider_label("smtp"), "Email");
}

/// Port of: packages/utils/__tests__/error-log.test.ts:89 "errorLogProviderLabel > falls back to the raw value for an unknown action"
#[test]
fn errorlogproviderlabel_falls_back_to_the_raw_value_for_an_unknown_action() {
    assert_eq!(
        error_log_provider_label("some-removed-provider"),
        "some-removed-provider"
    );
}

/// Port of: packages/utils/__tests__/error-log.test.ts:99 "errorLogProvidersMatchingLabel > finds the provider behind a label that shares no text with its slug"
#[test]
fn errorlogprovidersmatchinglabel_finds_the_provider_behind_a_label_that_shares_no_text_with_its_slug()
 {
    assert_eq!(error_log_providers_matching_label("Email"), vec!["smtp"]);
}

/// Port of: packages/utils/__tests__/error-log.test.ts:103 "errorLogProvidersMatchingLabel > is case-insensitive and matches on a fragment"
#[test]
fn errorlogprovidersmatchinglabel_is_case_insensitive_and_matches_on_a_fragment() {
    assert_eq!(
        error_log_providers_matching_label("meta"),
        vec!["meta-catalog", "meta-conversions"]
    );
}

/// Port of: packages/utils/__tests__/error-log.test.ts:110 "errorLogProvidersMatchingLabel > returns nothing for a blank or unmatched keyword"
#[test]
fn errorlogprovidersmatchinglabel_returns_nothing_for_a_blank_or_unmatched_keyword() {
    assert!(error_log_providers_matching_label("   ").is_empty());
    assert!(error_log_providers_matching_label("nothing here").is_empty());
}
