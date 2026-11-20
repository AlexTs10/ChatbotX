//! `magicLinks.update` with no field: drizzle's `.set({})` throws, so the
//! update is an unexpected (500) error and the link stays as it was.

mod common;

use chatbotx_business::BusinessError;
use chatbotx_business::magic_link as service;
use common::{WS, two_workspaces};

#[tokio::test]
async fn an_empty_update_fails_and_changes_nothing() {
    let db = two_workspaces().await;
    let link = service::create(&db, WS, "promo".into(), "https://parity.test/a".into())
        .await
        .unwrap();
    let error = service::update(&db, WS, link.id, None, None)
        .await
        .unwrap_err();
    assert!(matches!(error, BusinessError::Repo(_)), "{error:?}");
    let unknown = service::update(&db, WS, link.id + 1, None, None)
        .await
        .unwrap_err();
    assert_eq!(unknown.status(), Some(404));
    let stored = service::find_or_fail(&db, WS, link.id).await.unwrap();
    assert_eq!((stored.name, stored.url), (link.name, link.url));
}
