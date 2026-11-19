//! Port of `apps/builder/__tests__/magic-links-public-api.test.ts` (Q.D13a):
//! the TS test checks that each `/v1/magic-links` route calls
//! `magicLinkService` with the token workspace; here the routes run on a
//! migrated database. Not applicable: the router scope assert (the route
//! table test).

mod handlers_common;

use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::json;

#[tokio::test]
async fn magic_links_are_scoped_to_the_token_workspace() {
    let fx = fixture().await;
    let (status, created) = call(
        &fx,
        "magicLinks.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "promo", "url": "https://example.com" })),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    // The response carries the public `/r/{workspaceId}/{name}` link.
    assert_eq!(created["name"], "promo");
    assert_eq!(
        created["url"],
        format!("http://app/r/{WS}/promo"),
        "{created}"
    );
    let id = created["id"].as_str().expect("id").to_owned();

    // A client-supplied workspace id is ignored.
    let query = format!("page=1&perPage=50&keyword=promo&workspaceId={OTHER_WS}");
    let (status, list) = call(&fx, "magicLinks.list", WS, &[], Some(&query), None).await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(list["data"][0]["id"], id.as_str(), "{list}");
    let (_, other) = call(&fx, "magicLinks.list", OTHER_WS, &[], None, None).await;
    assert_eq!(other["data"], json!([]), "{other}");

    for route in ["magicLinks.get", "magicLinks.delete"] {
        let (status, _) = call(&fx, route, OTHER_WS, &[("id", &id)], None, None).await;
        assert_eq!(status, 404, "{route}");
    }
    let (status, _) = call(
        &fx,
        "magicLinks.update",
        OTHER_WS,
        &[("id", &id)],
        None,
        Some(json!({ "url": "https://example.org" })),
    )
    .await;
    assert_eq!(status, 404);

    let (status, found) = call(&fx, "magicLinks.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(
        (status, found["name"].clone()),
        (200, json!("promo")),
        "{found}"
    );
    let (status, updated) = call(
        &fx,
        "magicLinks.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "url": "https://example.org" })),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    let stored = handlers_common::scalar(
        &fx,
        r#"SELECT "url" FROM "MagicLink" WHERE "id" = ?1"#,
        id.parse().expect("id"),
    )
    .await;
    assert_eq!(stored, Some(json!("https://example.org")));
    let (status, _) = call(&fx, "magicLinks.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204);
    let (status, _) = call(&fx, "magicLinks.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
}
