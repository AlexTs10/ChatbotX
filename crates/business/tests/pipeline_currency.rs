//! Port of `packages/business/__tests__/pipeline-currency.test.ts`.
//! Adapted: the mocked `workspaceService.findById` is a real `Workspace` row
//! (`targetCountry`); the mock call counts ("never reads", "reads once") have
//! no Rust counterpart, the results are asserted.

mod common;

use chatbotx_business::pipeline::currency::{
    EffectiveCurrency, WorkspaceCurrencyDefault, apply_effective_currency,
    resolve_workspace_currency, resolve_workspace_default, with_effective_currencies,
    with_effective_currency,
};
use common::{TestDb, WS, setup, target_country, two_workspaces};

async fn workspace_in(country: Option<&'static str>) -> TestDb {
    let db = two_workspaces().await;
    if let Some(country) = country {
        setup(&db, move |tx| target_country(tx, WS, country)).await;
    }
    db
}

fn effective(currency: &str, currency_override: Option<&str>) -> EffectiveCurrency {
    EffectiveCurrency {
        currency: currency.to_string(),
        currency_override: currency_override.map(str::to_string),
    }
}

#[tokio::test]
async fn resolve_workspace_currency_follows_the_target_country() {
    let db = workspace_in(Some("VN")).await;
    assert_eq!(resolve_workspace_currency(&db, WS).await.unwrap(), "VND");
}

#[tokio::test]
async fn resolve_workspace_currency_is_usd_for_no_unknown_or_currencyless_country() {
    for country in [None, Some("unknown"), Some("AQ")] {
        let db = workspace_in(country).await;
        assert_eq!(
            resolve_workspace_currency(&db, WS).await.unwrap(),
            "USD",
            "{country:?}"
        );
    }
}

#[tokio::test]
async fn resolve_workspace_default_names_the_country_the_currency_comes_from_upper_cased() {
    let db = workspace_in(Some(" vn ")).await;
    assert_eq!(
        resolve_workspace_default(&db, WS).await.unwrap(),
        WorkspaceCurrencyDefault {
            currency: "VND".to_string(),
            country: Some("VN".to_string()),
        }
    );
}

#[tokio::test]
async fn resolve_workspace_default_names_no_country_and_falls_back_to_usd() {
    for country in [None, Some("unknown"), Some("AQ")] {
        let db = workspace_in(country).await;
        assert_eq!(
            resolve_workspace_default(&db, WS).await.unwrap(),
            WorkspaceCurrencyDefault {
                currency: "USD".to_string(),
                country: None,
            },
            "{country:?}"
        );
    }
}

#[test]
fn apply_effective_currency_an_override_wins_over_the_workspace_currency() {
    assert_eq!(
        apply_effective_currency(Some("JPY"), "VND"),
        effective("JPY", Some("JPY"))
    );
}

#[test]
fn apply_effective_currency_no_override_takes_the_workspace_currency() {
    assert_eq!(
        apply_effective_currency(None, "VND"),
        effective("VND", None)
    );
}

#[tokio::test]
async fn with_effective_currency_an_override_never_reads_the_workspace() {
    // No workspace row at all: an override needs none.
    let db = common::fresh_db().await;
    assert_eq!(
        with_effective_currency(&db, WS, Some("JPY")).await.unwrap(),
        effective("JPY", Some("JPY"))
    );
}

#[tokio::test]
async fn with_effective_currency_exposes_the_effective_currency_and_keeps_the_stored_one_as_the_override()
 {
    let db = workspace_in(Some("VN")).await;
    assert_eq!(
        with_effective_currency(&db, WS, None).await.unwrap(),
        effective("VND", None)
    );
    assert_eq!(
        with_effective_currency(&db, WS, Some("USD")).await.unwrap(),
        effective("USD", Some("USD"))
    );
}

#[tokio::test]
async fn with_effective_currencies_reads_the_workspace_once_for_many_following_rows() {
    let db = workspace_in(Some("VN")).await;
    let rows = with_effective_currencies(&db, WS, &[None, Some("USD".to_string()), None])
        .await
        .unwrap();
    assert_eq!(
        rows.iter().map(|r| r.currency.as_str()).collect::<Vec<_>>(),
        ["VND", "USD", "VND"]
    );
    assert_eq!(
        rows.iter()
            .map(|r| r.currency_override.as_deref())
            .collect::<Vec<_>>(),
        [None, Some("USD"), None]
    );
}

#[tokio::test]
async fn with_effective_currencies_never_reads_the_workspace_when_every_row_has_an_override() {
    let db = common::fresh_db().await;
    let rows =
        with_effective_currencies(&db, WS, &[Some("USD".to_string()), Some("JPY".to_string())])
            .await
            .unwrap();
    assert_eq!(
        rows.iter().map(|r| r.currency.as_str()).collect::<Vec<_>>(),
        ["USD", "JPY"]
    );
}

#[tokio::test]
async fn with_effective_currencies_an_empty_list_reads_nothing() {
    let db = common::fresh_db().await;
    assert!(
        with_effective_currencies(&db, WS, &[])
            .await
            .unwrap()
            .is_empty()
    );
}
