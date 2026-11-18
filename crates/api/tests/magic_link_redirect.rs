//! `apps/builder/__tests__/magic-link-route.test.ts` (Q.D13), the parts that
//! need no flow: the redirect, the 404 / 410 answers and the bad codes. The
//! click attribution runs on the events bus (no listener in Rust yet).

mod handlers_common;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use chatbotx_api::magic_link_redirect::{RedirectState, router};
use handlers_common::{OTHER_WS, WS, fixture};
use tower::ServiceExt as _;

async fn get(fx: &handlers_common::Fixture, path: &str) -> (StatusCode, Option<String>) {
    let app = router(RedirectState {
        api: Arc::new(fx.state.clone()),
        buses: None,
    });
    let response = app
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let location = response
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().unwrap().to_owned());
    (response.status(), location)
}

#[tokio::test]
async fn magic_link_redirects_with_interpolated_params() {
    let fx = fixture().await;
    chatbotx_business::magic_link::create(
        &fx.state.db,
        WS,
        "promo".into(),
        "https://example.com/?ref={{src}}".into(),
    )
    .await
    .unwrap();
    let (status, location) = get(&fx, &format!("/r/{WS}/promo?src=ig")).await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(location.as_deref(), Some("https://example.com/?ref=ig"));

    // Another workspace's name is a 404.
    let (status, _) = get(&fx, &format!("/r/{OTHER_WS}/promo")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // A code that does not decode is a 400.
    let (status, _) = get(&fx, &format!("/r/{WS}/promo?code=garbage")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // A payload without a contact inbox id is a 400 too.
    let (status, _) = get(&fx, &format!("/r/{WS}/promo?code=1000000000001:")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // An unknown contact inbox: redirect without recording.
    let (status, _) = get(&fx, &format!("/r/{WS}/promo?code=1000000000001:::::99")).await;
    assert_eq!(status, StatusCode::FOUND);

    // A workspace with a scheduled deletion answers 410.
    fx.state
        .db
        .write(|tx| {
            tx.execute(
                r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = ?1"#,
                [WS],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let (status, _) = get(&fx, &format!("/r/{WS}/promo")).await;
    assert_eq!(status, StatusCode::GONE);
}
