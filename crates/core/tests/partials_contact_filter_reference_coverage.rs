//! Port of `packages/database/__tests__/contact-filter-reference-coverage.test.ts` (P3.2b;
//! lives in chatbotx-core because it compares the db partial with `flow_config`; ported:
//! `CONTACT_FILTER_FIELD_REFERENCE` is `contact_filter_field_reference()`).

use chatbotx_core::flow_config::import_export::contact_filter_field_reference;
use chatbotx_db::repo::partials::contact::CONTACT_FILTER_FIELDS;

/// The contact-filter fields whose value is an id.
const ID_VALUED_FIELDS: &[&str] = &[
    "inbox",
    "tags",
    "subscribedToDripCampaign",
    "conversationAssigned",
    "entryPointsLinks",
    "completedWhatsAppFlows",
    "keywordsReceived",
    "executedFlow",
    "executedStep",
    "commentedOnPost",
    "broadcastSent",
    "broadcastDelivered",
    "broadcastSeen",
    "broadcastClicked",
    "broadcastFailed",
    "emailSent",
    "emailDelivered",
    "emailOpened",
    "emailClicked",
    "clickedAd",
    "boughtItems",
    "shoppingCartContainsItems",
];

fn reference_keys() -> Vec<String> {
    contact_filter_field_reference()
        .as_object()
        .expect("reference object")
        .keys()
        .cloned()
        .collect()
}

#[test]
fn contact_filter_field_reference_covers_every_contact_filter_field_whose_value_is_an_id() {
    let mut keys = reference_keys();
    keys.sort_unstable();
    let mut expected: Vec<&str> = ID_VALUED_FIELDS.to_vec();
    expected.sort_unstable();
    assert_eq!(keys, expected);
}

#[test]
fn contact_filter_field_reference_only_names_real_contact_filter_fields() {
    for field in reference_keys() {
        assert!(CONTACT_FILTER_FIELDS.contains(&field.as_str()), "{field}");
    }
}
