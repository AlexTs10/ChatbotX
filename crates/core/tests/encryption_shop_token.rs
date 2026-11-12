//! Port of `packages/encryption/__tests__/shop-token.test.ts`.

mod support;

use chatbotx_core::encryption::{
    ContactInboxClaims, MINIGAME_PLAY_TOKEN_TTL_MS, SHOP_TOKEN_TTL_MS, ShopCustomerClaims,
    ShopTokenPayload,
};
use regex::Regex;

fn customer() -> ShopCustomerClaims {
    ShopCustomerClaims {
        workspace_id: "1".to_owned(),
        contact_id: "2".to_owned(),
        contact_inbox_id: Some("3".to_owned()),
        conversation_id: None,
    }
}

/// Port of: packages/encryption/__tests__/shop-token.test.ts:20 "shop token > round-trips a customer token"
#[test]
fn shop_token_round_trips_a_customer_token() {
    let enc = support::encryption();
    let token = enc.sign_shop_token(&customer(), SHOP_TOKEN_TTL_MS).unwrap();
    assert!(Regex::new(r"^[A-Za-z0-9\-_]+$").unwrap().is_match(&token));
    match enc.verify_shop_token(&token).unwrap() {
        ShopTokenPayload::Customer {
            workspace_id,
            contact_id,
            contact_inbox_id,
            conversation_id,
            ..
        } => {
            assert_eq!(workspace_id, "1");
            assert_eq!(contact_id, "2");
            assert_eq!(contact_inbox_id.as_deref(), Some("3"));
            assert_eq!(conversation_id, None);
        }
        other => panic!("expected a customer token, got {other:?}"),
    }
}

/// Port of: packages/encryption/__tests__/shop-token.test.ts:29 "shop token > a preview token names no contact"
#[test]
fn shop_token_a_preview_token_names_no_contact() {
    let enc = support::encryption();
    let payload = enc
        .verify_shop_token(&enc.sign_shop_preview_token("1").unwrap())
        .unwrap();
    match &payload {
        ShopTokenPayload::Preview { workspace_id, .. } => assert_eq!(workspace_id, "1"),
        other => panic!("expected a preview token, got {other:?}"),
    }
    let serialized = serde_json::to_value(&payload).unwrap();
    assert_eq!(serialized["mode"], "preview");
    assert!(serialized.get("contactId").is_none());
}

/// Port of: packages/encryption/__tests__/shop-token.test.ts:35 "shop token > rejects an expired token"
#[test]
fn shop_token_rejects_an_expired_token() {
    let enc = support::encryption();
    let token = enc.sign_shop_token(&customer(), -1).unwrap();
    let err = enc.verify_shop_token(&token).unwrap_err();
    assert!(err.to_string().contains("expired"), "{err}");
}

/// Port of: packages/encryption/__tests__/shop-token.test.ts:40 "shop token > rejects a malformed token"
#[test]
fn shop_token_rejects_a_malformed_token() {
    let enc = support::encryption();
    assert!(enc.verify_shop_token("not-a-token").is_err());
}

/// Port of: packages/encryption/__tests__/shop-token.test.ts:44 "shop token > rejects another kind of signed token"
#[test]
fn shop_token_rejects_another_kind_of_signed_token() {
    let enc = support::encryption();
    let minigame_token = enc
        .sign_minigame_play_token(
            &ContactInboxClaims {
                workspace_id: "1".to_owned(),
                contact_id: "2".to_owned(),
                contact_inbox_id: "3".to_owned(),
            },
            MINIGAME_PLAY_TOKEN_TTL_MS,
        )
        .unwrap();
    assert!(enc.verify_shop_token(&minigame_token).is_err());
}
