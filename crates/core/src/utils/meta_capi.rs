//! `packages/utils/src/meta-capi.ts`: Meta Conversions API catalogs and
//! validators.

use std::sync::OnceLock;

use regex::Regex;

use crate::js;

/// Meta's 14 Business Messaging events.
pub const META_CAPI_BUSINESS_MESSAGING_EVENT_NAMES: &[&str] = &[
    "Purchase",
    "LeadSubmitted",
    "InitiateCheckout",
    "AddToCart",
    "ViewContent",
    "OrderCreated",
    "OrderShipped",
    "OrderDelivered",
    "OrderCanceled",
    "OrderReturned",
    "CartAbandoned",
    "QualifiedLead",
    "RatingProvided",
    "ReviewProvided",
];

/// Meta Pixel's 17 standard events.
pub const META_PIXEL_STANDARD_EVENT_NAMES: &[&str] = &[
    "AddPaymentInfo",
    "AddToCart",
    "AddToWishlist",
    "CompleteRegistration",
    "Contact",
    "CustomizeProduct",
    "Donate",
    "FindLocation",
    "InitiateCheckout",
    "Lead",
    "Purchase",
    "Schedule",
    "Search",
    "StartTrial",
    "SubmitApplication",
    "Subscribe",
    "ViewContent",
];

/// `metaCapiEventNameSchema`: the trimmed name, 1–50 characters.
pub fn parse_meta_capi_event_name(value: &str) -> Option<String> {
    let trimmed = js::trim(value);
    (1..=50)
        .contains(&js::utf16_len(trimmed))
        .then(|| trimmed.to_owned())
}

string_enum! {
    /// `MetaCapiEventCatalog`.
    pub enum MetaCapiEventCatalog {
        BusinessMessaging = "businessMessaging",
        Pixel = "pixel",
    }
}

/// `defaultEventNameByCatalog`.
pub fn default_event_name_by_catalog(catalog: MetaCapiEventCatalog) -> &'static str {
    match catalog {
        MetaCapiEventCatalog::BusinessMessaging => "LeadSubmitted",
        MetaCapiEventCatalog::Pixel => "Lead",
    }
}

/// `eventNamesByCatalog`.
pub fn event_names_by_catalog(catalog: MetaCapiEventCatalog) -> &'static [&'static str] {
    match catalog {
        MetaCapiEventCatalog::BusinessMessaging => META_CAPI_BUSINESS_MESSAGING_EVENT_NAMES,
        MetaCapiEventCatalog::Pixel => META_PIXEL_STANDARD_EVENT_NAMES,
    }
}

string_enum! {
    /// `metaCapiActionSourceValues`.
    pub enum MetaCapiActionSource {
        BusinessMessaging = "business_messaging",
        Email = "email",
        PhoneCall = "phone_call",
        Chat = "chat",
        PhysicalStore = "physical_store",
        SystemGenerated = "system_generated",
        Other = "other",
    }
}

/// `defaultMetaCapiActionSource`.
pub const DEFAULT_META_CAPI_ACTION_SOURCE: MetaCapiActionSource =
    MetaCapiActionSource::BusinessMessaging;

/// One `metaCapiActionSourcePolicy` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetaCapiActionSourcePolicy {
    pub uses_messaging_identity: bool,
    pub event_catalog: MetaCapiEventCatalog,
    pub allows_custom_event_names: bool,
}

/// `metaCapiActionSourcePolicy[actionSource]`.
pub fn meta_capi_action_source_policy(source: MetaCapiActionSource) -> MetaCapiActionSourcePolicy {
    if source == MetaCapiActionSource::BusinessMessaging {
        MetaCapiActionSourcePolicy {
            uses_messaging_identity: true,
            event_catalog: MetaCapiEventCatalog::BusinessMessaging,
            allows_custom_event_names: false,
        }
    } else {
        MetaCapiActionSourcePolicy {
            uses_messaging_identity: false,
            event_catalog: MetaCapiEventCatalog::Pixel,
            allows_custom_event_names: true,
        }
    }
}

/// `metaCapiValueSchema`: a trimmed plain decimal ≤ `MAX_SAFE_INTEGER`.
pub fn parse_meta_capi_value(value: &str) -> Result<String, &'static str> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let trimmed = js::trim(value);
    if !RE
        .get_or_init(|| Regex::new(r"^[0-9]+(\.[0-9]+)?$").expect("re"))
        .is_match(trimmed)
    {
        return Err("Value must be a plain number such as 19.99");
    }
    if js::to_number(trimmed) <= 9_007_199_254_740_991.0 {
        Ok(trimmed.to_owned())
    } else {
        Err("Value is too large")
    }
}

/// `metaCapiCurrencySchema`: trimmed, upper-cased, 3 letters.
pub fn parse_meta_capi_currency(value: &str) -> Result<String, &'static str> {
    let upper = js::trim(value).to_uppercase();
    if upper.len() == 3 && upper.bytes().all(|b| b.is_ascii_uppercase()) {
        Ok(upper)
    } else {
        Err("Currency must be a 3-letter ISO 4217 code such as USD")
    }
}

/// `CAPI_TEST_MESSAGING_ID_MAX_LENGTH`.
pub const CAPI_TEST_MESSAGING_ID_MAX_LENGTH: usize = 512;

/// `capiTestMessagingIdSchema`: trimmed, 1–512 of `[A-Za-z0-9_-]`.
pub fn parse_capi_test_messaging_id(value: &str) -> Option<String> {
    let trimmed = js::trim(value);
    let ok = !trimmed.is_empty()
        && trimmed.len() <= CAPI_TEST_MESSAGING_ID_MAX_LENGTH
        && trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    ok.then(|| trimmed.to_owned())
}

/// `metaCapiContentTypeValues`.
pub const META_CAPI_CONTENT_TYPE_VALUES: &[&str] = &["product", "product_group"];
