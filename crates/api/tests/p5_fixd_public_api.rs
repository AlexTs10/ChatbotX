//! Q.P5-fixD parity fixes on public routes: lead-ads field-mapping issue
//! paths and name sort, marketing-message audience bodies, channel ad
//! accounts without `integrationId`.

mod handlers_common;

use handlers_common::{WS, call, fixture};
use serde_json::{Value, json};

fn issue_paths(body: &Value) -> Vec<String> {
    body["data"]["issues"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|i| {
            i["path"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| s.as_str().map_or_else(|| s.to_string(), str::to_owned))
                .collect::<Vec<_>>()
                .join(".")
        })
        .collect()
}

#[tokio::test]
async fn lead_ads_field_mapping_issues_name_the_item_field() {
    let fx = fixture().await;
    let body =
        json!({ "name": "n", "pageId": "1", "formId": "1", "fieldMapping": [{ "key": "email" }] });
    let (status, out) = call(&fx, "facebookLeadAds.create", WS, &[], None, Some(body)).await;
    assert_eq!(status, 422, "{out}");
    let paths = issue_paths(&out);
    assert!(
        paths.contains(&"fieldMapping.0.label".to_owned()),
        "{paths:?}"
    );
    assert!(
        paths.contains(&"fieldMapping.0.target".to_owned()),
        "{paths:?}"
    );
}

#[tokio::test]
async fn lead_ads_list_sorts_by_name_descending() {
    let fx = fixture().await;
    fx.state
        .db
        .write(|tx| {
            tx.execute_batch(&format!(
                r#"INSERT INTO "FacebookLeadAdsAutomation" ("id", "name", "workspaceId", "pageId", "formId") VALUES
                     (1, 'alpha', {WS}, 'p', '1'), (2, 'gamma', {WS}, 'p', '2'), (3, 'beta', {WS}, 'p', '3');"#
            ))?;
            Ok(())
        })
        .await
        .unwrap();
    let query = "sort%5B0%5D%5Bid%5D=name&sort%5B0%5D%5Bdesc%5D=true";
    let (status, out) = call(&fx, "facebookLeadAds.list", WS, &[], Some(query), None).await;
    assert_eq!(status, 200, "{out}");
    let names: Vec<&str> = out["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["gamma", "beta", "alpha"]);
}

#[tokio::test]
async fn audience_routes_read_the_json_body() {
    let fx = fixture().await;
    let body = json!({
        "adAccountId": "act_1", "pageId": "1", "name": "X", "description": "x".repeat(501),
        "headers": ["email"], "rows": [["a@b.co"]],
    });
    let (status, out) = call(
        &fx,
        "facebookMarketingMessages.audiences.create",
        WS,
        &[],
        None,
        Some(body),
    )
    .await;
    assert_eq!(status, 422, "{out}");
    assert_eq!(issue_paths(&out), ["description"]);
    let (status, out) = call(
        &fx,
        "facebookMarketingMessages.audiences.uploadMore",
        WS,
        &[("id", "1")],
        None,
        Some(json!({ "headers": ["email"] })),
    )
    .await;
    assert_eq!(status, 422, "{out}");
    assert_eq!(issue_paths(&out), ["rows"]);
}

#[tokio::test]
async fn channel_ad_accounts_need_no_integration_id() {
    let fx = fixture().await;
    let (status, out) = call(
        &fx,
        "ads.listChannelAdAccounts",
        WS,
        &[("channel", "messenger")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out, json!({ "data": [] }));
    let (status, out) = call(
        &fx,
        "ads.listChannelAdAccounts",
        WS,
        &[("channel", "messenger")],
        Some("integrationId=x"),
        None,
    )
    .await;
    assert_eq!(status, 422, "{out}");
    assert_eq!(issue_paths(&out), ["integrationId"]);
}
