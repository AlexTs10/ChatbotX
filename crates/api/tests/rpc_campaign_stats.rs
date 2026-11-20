//! Q.P6-U5 (COVERAGE E21): `analyticsRoutes.getBroadcastStatsAnalyticsAPI`
//! and `getSequenceStepStatsAnalyticsAPI` take their ids as path
//! parameters. No original TS test covers the private twins.

mod rpc_common;

use axum::http::StatusCode;
use rpc_common::{OWNER_TOKEN, WS, call, fixture};
use serde_json::json;

#[tokio::test]
async fn the_broadcast_and_sequence_step_stats_answer_the_delivery_counters() {
    let fx = fixture().await;
    let ws = WS.to_string();
    let zero = json!({
        "message:sent": 0, "message:delivered": 0, "message:seen": 0,
        "flow:clicked": 0, "message:failed": 0,
    });
    let (status, body) = call(
        &fx,
        "GET",
        "/rpc/analyticsRoutes/getBroadcastStatsAnalyticsAPI",
        Some(OWNER_TOKEN),
        json!({ "workspaceId": ws, "broadcastId": "301" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["json"], zero);
    let (status, body) = call(
        &fx,
        "GET",
        "/rpc/analyticsRoutes/getSequenceStepStatsAnalyticsAPI",
        Some(OWNER_TOKEN),
        json!({ "workspaceId": ws, "sequenceId": "31", "stepId": "41" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["json"], zero);
    let (status, _) = call(
        &fx,
        "GET",
        "/rpc/analyticsRoutes/getBroadcastStatsAnalyticsAPI",
        Some(OWNER_TOKEN),
        json!({ "workspaceId": ws }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
