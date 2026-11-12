//! Port of `packages/utils/__tests__/meta-capi.test.ts`.

use chatbotx_core::utils::meta_capi::{
    META_CAPI_BUSINESS_MESSAGING_EVENT_NAMES, META_CAPI_CONTENT_TYPE_VALUES,
    META_PIXEL_STANDARD_EVENT_NAMES, MetaCapiActionSource, MetaCapiEventCatalog,
    default_event_name_by_catalog, meta_capi_action_source_policy, parse_capi_test_messaging_id,
    parse_meta_capi_currency, parse_meta_capi_event_name, parse_meta_capi_value,
};

/// Port of: packages/utils/__tests__/meta-capi.test.ts:16 "metaCapiActionSourcePolicy > has exactly one entry per offered action source"
#[test]
fn metacapiactionsourcepolicy_has_exactly_one_entry_per_offered_action_source() {
    // An exhaustive function: one policy per action source.
    for source in MetaCapiActionSource::ALL {
        let _ = meta_capi_action_source_policy(*source);
    }
    assert_eq!(MetaCapiActionSource::ALL.len(), 7);
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:22 "metaCapiActionSourcePolicy > only business_messaging uses the messaging identity and business-messaging catalog"
#[test]
fn metacapiactionsourcepolicy_only_business_messaging_uses_the_messaging_identity_and_business_messaging_catalog()
 {
    for source in MetaCapiActionSource::ALL {
        let policy = meta_capi_action_source_policy(*source);
        let is_business_messaging = *source == MetaCapiActionSource::BusinessMessaging;
        assert_eq!(policy.uses_messaging_identity, is_business_messaging);
        assert_eq!(
            policy.event_catalog,
            if is_business_messaging {
                MetaCapiEventCatalog::BusinessMessaging
            } else {
                MetaCapiEventCatalog::Pixel
            }
        );
        assert_eq!(policy.allows_custom_event_names, !is_business_messaging);
    }
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:37 "event name catalogs > business-messaging catalog has Meta's 14 documented events"
#[test]
fn event_name_catalogs_business_messaging_catalog_has_meta_s_14_documented_events() {
    assert_eq!(META_CAPI_BUSINESS_MESSAGING_EVENT_NAMES.len(), 14);
    assert!(META_CAPI_BUSINESS_MESSAGING_EVENT_NAMES.contains(&"LeadSubmitted"));
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:42 "event name catalogs > pixel catalog has Meta's 17 standard events"
#[test]
fn event_name_catalogs_pixel_catalog_has_meta_s_17_standard_events() {
    assert_eq!(META_PIXEL_STANDARD_EVENT_NAMES.len(), 17);
    assert!(META_PIXEL_STANDARD_EVENT_NAMES.contains(&"Lead"));
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:47 "event name catalogs > defaults each catalog to its documented default event"
#[test]
fn event_name_catalogs_defaults_each_catalog_to_its_documented_default_event() {
    assert_eq!(
        default_event_name_by_catalog(MetaCapiEventCatalog::BusinessMessaging),
        "LeadSubmitted"
    );
    assert_eq!(
        default_event_name_by_catalog(MetaCapiEventCatalog::Pixel),
        "Lead"
    );
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:54 "metaCapiEventNameSchema > accepts a standard or custom event name up to 50 characters"
#[test]
fn metacapieventnameschema_accepts_a_standard_or_custom_event_name_up_to_50_characters() {
    assert!(parse_meta_capi_event_name("Purchase").is_some());
    assert!(parse_meta_capi_event_name("my-custom-event").is_some());
    assert!(parse_meta_capi_event_name(&"a".repeat(50)).is_some());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:62 "metaCapiEventNameSchema > rejects an empty, whitespace-only, or over-length event name"
#[test]
fn metacapieventnameschema_rejects_an_empty_whitespace_only_or_over_length_event_name() {
    assert!(parse_meta_capi_event_name("").is_none());
    assert!(parse_meta_capi_event_name("   ").is_none());
    assert!(parse_meta_capi_event_name(&"a".repeat(51)).is_none());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:72 "metaCapiContentTypeValues > only offers Meta's two documented content types"
#[test]
fn metacapicontenttypevalues_only_offers_meta_s_two_documented_content_types() {
    assert_eq!(META_CAPI_CONTENT_TYPE_VALUES, &["product", "product_group"]);
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:78 "metaCapiValueSchema / metaCapiCurrencySchema > accepts plain decimal "19.99""
#[test]
fn metacapivalueschema_metacapicurrencyschema_accepts_plain_decimal_19_99() {
    assert!(parse_meta_capi_value("19.99").is_ok());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:78 "metaCapiValueSchema / metaCapiCurrencySchema > accepts plain decimal " 250 ""
#[test]
fn metacapivalueschema_metacapicurrencyschema_accepts_plain_decimal_250() {
    assert!(parse_meta_capi_value(" 250 ").is_ok());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:78 "metaCapiValueSchema / metaCapiCurrencySchema > accepts plain decimal "0""
#[test]
fn metacapivalueschema_metacapicurrencyschema_accepts_plain_decimal_0() {
    assert!(parse_meta_capi_value("0").is_ok());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:78 "metaCapiValueSchema / metaCapiCurrencySchema > accepts plain decimal "9007199254740991""
#[test]
fn metacapivalueschema_metacapicurrencyschema_accepts_plain_decimal_9007199254740991() {
    assert!(parse_meta_capi_value("9007199254740991").is_ok());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:87 "metaCapiValueSchema / metaCapiCurrencySchema > rejects non-canonical value "12,50""
#[test]
fn metacapivalueschema_metacapicurrencyschema_rejects_non_canonical_value_12_50() {
    assert!(parse_meta_capi_value("12,50").is_err());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:87 "metaCapiValueSchema / metaCapiCurrencySchema > rejects non-canonical value "1e5""
#[test]
fn metacapivalueschema_metacapicurrencyschema_rejects_non_canonical_value_1e5() {
    assert!(parse_meta_capi_value("1e5").is_err());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:87 "metaCapiValueSchema / metaCapiCurrencySchema > rejects non-canonical value "-5""
#[test]
fn metacapivalueschema_metacapicurrencyschema_rejects_non_canonical_value_5() {
    assert!(parse_meta_capi_value("-5").is_err());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:87 "metaCapiValueSchema / metaCapiCurrencySchema > rejects non-canonical value "abc""
#[test]
fn metacapivalueschema_metacapicurrencyschema_rejects_non_canonical_value_abc() {
    assert!(parse_meta_capi_value("abc").is_err());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:87 "metaCapiValueSchema / metaCapiCurrencySchema > rejects non-canonical value "{{amount}}""
#[test]
fn metacapivalueschema_metacapicurrencyschema_rejects_non_canonical_value_amount() {
    assert!(parse_meta_capi_value("{{amount}}").is_err());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:87 "metaCapiValueSchema / metaCapiCurrencySchema > rejects non-canonical value """
#[test]
fn metacapivalueschema_metacapicurrencyschema_rejects_non_canonical_value() {
    assert!(parse_meta_capi_value("").is_err());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:87 "metaCapiValueSchema / metaCapiCurrencySchema > rejects non-canonical value "1.""
#[test]
fn metacapivalueschema_metacapicurrencyschema_rejects_non_canonical_value_1() {
    assert!(parse_meta_capi_value("1.").is_err());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:87 "metaCapiValueSchema / metaCapiCurrencySchema > rejects non-canonical value ".5""
#[test]
fn metacapivalueschema_metacapicurrencyschema_rejects_non_canonical_value_5_2() {
    assert!(parse_meta_capi_value(".5").is_err());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:100 "metaCapiValueSchema / metaCapiCurrencySchema > rejects a value that would not survive Number() intact"
#[test]
fn metacapivalueschema_metacapicurrencyschema_rejects_a_value_that_would_not_survive_number_intact()
{
    assert!(parse_meta_capi_value("9007199254740992").is_err());
    assert!(parse_meta_capi_value(&"1".repeat(400)).is_err());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:107 "metaCapiValueSchema / metaCapiCurrencySchema > currency is upper-cased and must be a 3-letter code"
#[test]
fn metacapivalueschema_metacapicurrencyschema_currency_is_upper_cased_and_must_be_a_3_letter_code()
{
    assert_eq!(parse_meta_capi_currency(" usd "), Ok("USD".to_owned()));
    assert!(parse_meta_capi_currency("US").is_err());
    assert!(parse_meta_capi_currency("USDT").is_err());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:115 "capiTestMessagingIdSchema > accepts the alphabet Meta's Test events samples use and trims whitespace"
#[test]
fn capitestmessagingidschema_accepts_the_alphabet_meta_s_test_events_samples_use_and_trims_whitespace()
 {
    assert_eq!(
        parse_capi_test_messaging_id("  ARAkLkA8rml-FeiCkt_EJQ ").as_deref(),
        Some("ARAkLkA8rml-FeiCkt_EJQ")
    );
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:121 "capiTestMessagingIdSchema > rejects """
#[test]
fn capitestmessagingidschema_rejects() {
    assert!(parse_capi_test_messaging_id("").is_none());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:121 "capiTestMessagingIdSchema > rejects "   ""
#[test]
fn capitestmessagingidschema_rejects_2() {
    assert!(parse_capi_test_messaging_id("   ").is_none());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:121 "capiTestMessagingIdSchema > rejects "psid 1""
#[test]
fn capitestmessagingidschema_rejects_psid_1() {
    assert!(parse_capi_test_messaging_id("psid 1").is_none());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:121 "capiTestMessagingIdSchema > rejects "id;drop""
#[test]
fn capitestmessagingidschema_rejects_id_drop() {
    assert!(parse_capi_test_messaging_id("id;drop").is_none());
}

/// Port of: packages/utils/__tests__/meta-capi.test.ts:121 "capiTestMessagingIdSchema > rejects "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx""
#[test]
fn capitestmessagingidschema_rejects_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx()
 {
    assert!(parse_capi_test_messaging_id("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx").is_none());
}
