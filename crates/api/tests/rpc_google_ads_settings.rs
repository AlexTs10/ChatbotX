//! The Google Ads settings page under `/rpc` (Q.F2-settings):
//! `googleAdsSettingsPage` (`loadGoogleAdsSettingsPage` +
//! `loadSettingsSession`), `googleAdsAPI.listEvents` and the super-admin
//! guard of the retry / consent / validate actions. Ports of
//! `apps/builder/__tests__/google-ads-load-settings-session.test.ts` (the
//! original mocks `connectSessionService`; here the rows are real).

mod rpc_common;

use axum::http::StatusCode;
use chatbotx_db::rusqlite::params;
use rpc_common::{AGENT_TOKEN, NOW_MS, OUTSIDER_TOKEN, OWNER_TOKEN, WS, call, fixture};
use serde_json::{Value, json};

async fn act(
    fx: &rpc_common::Fixture,
    token: &str,
    name: &str,
    args: Value,
) -> (StatusCode, Value) {
    let (status, body) = call(
        fx,
        "POST",
        &format!("/rpc/actions/{name}"),
        Some(token),
        json!({ "workspaceId": WS.to_string(), "args": args }),
    )
    .await;
    (status, body["json"].clone())
}

/// `ConnectSession` rows: (id, provider, status, createdAt offset).
async fn sessions(fx: &rpc_common::Fixture, rows: &'static [(i64, &str, &str, i64)]) {
    fx.db()
        .write(move |tx| {
            for (id, provider, status, created) in rows {
                tx.execute(
                    r#"INSERT INTO "ConnectSession" ("id", "createdAt", "workspaceId", "provider", "purpose",
                           "stateNonceHash", "status", "expiresAt", "consumedAt")
                       VALUES (?1, ?2, ?3, ?4, 'connect', ?5, ?6, ?7,
                               CASE WHEN ?6 IN ('completed', 'failed', 'expired', 'cancelled') THEN ?2 END)"#,
                    params![
                        id,
                        NOW_MS + created,
                        WS,
                        provider,
                        format!("nonce-{id}"),
                        status,
                        i64::MAX / 2
                    ],
                )?;
            }
            Ok(())
        })
        .await
        .expect("sessions");
}

async fn page_session(fx: &rpc_common::Fixture, param: Value) -> Value {
    let (status, page) = act(fx, OWNER_TOKEN, "googleAdsSettingsPage", json!([param])).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    page["session"].clone()
}

/// A failed Google Ads session `id` in workspace 202 (another tenant).
async fn other_workspace_session(fx: &rpc_common::Fixture, id: i64) {
    fx.db()
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (202, 'Other', 103)"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "ConnectSession" ("id", "createdAt", "workspaceId", "provider", "purpose",
                       "stateNonceHash", "status", "expiresAt", "consumedAt")
                   VALUES (?1, ?2, 202, 'googleAds', 'connect', 'nonce-other', 'failed', ?3, ?2)"#,
                params![id, NOW_MS, i64::MAX / 2],
            )?;
            Ok(())
        })
        .await
        .expect("other workspace");
}

/// Port of: apps/builder/__tests__/google-ads-load-settings-session.test.ts:35 "surfaces a %s session so the picker can render it"
/// and :183 "shows the waiting state while the callback lands" (`pending`).
#[tokio::test]
async fn load_settings_session_surfaces_a_session_of_any_status_so_the_picker_can_render_it() {
    const ALL: &[(i64, &str, &str, i64)] = &[
        (9001, "googleAds", "pending", 0),
        (9002, "googleAds", "authorized", 0),
        (9003, "googleAds", "awaiting_selection", 0),
        (9004, "googleAds", "completed", 0),
        (9005, "googleAds", "failed", 0),
        (9006, "googleAds", "expired", 0),
        (9007, "googleAds", "cancelled", 0),
    ];
    let fx = fixture().await;
    sessions(&fx, ALL).await;
    for (id, _, status, _) in ALL {
        assert_eq!(
            page_session(&fx, json!(id.to_string())).await,
            json!({ "id": id.to_string(), "status": status })
        );
    }
}

/// Port of: apps/builder/__tests__/google-ads-load-settings-session.test.ts:54 "looks the session up scoped to the workspace"
/// and :96 "a session of another workspace (not found) falls back to in-flight, then null".
#[tokio::test]
async fn load_settings_session_looks_the_session_up_scoped_to_the_workspace() {
    let fx = fixture().await;
    other_workspace_session(&fx, 9999).await;
    assert_eq!(page_session(&fx, json!("9999")).await, Value::Null);
    // With an in-flight session of its own, that one is shown instead.
    sessions(&fx, &[(9001, "googleAds", "awaiting_selection", 0)]).await;
    assert_eq!(
        page_session(&fx, json!("9999")).await,
        json!({ "id": "9001", "status": "awaiting_selection" })
    );
}

/// Port of: apps/builder/__tests__/google-ads-load-settings-session.test.ts:129 "resumes the newest in-flight Google Ads session"
/// and :172 "is null when nothing is in flight".
#[tokio::test]
async fn load_settings_session_resumes_the_newest_in_flight_google_ads_session() {
    let fx = fixture().await;
    assert_eq!(page_session(&fx, Value::Null).await, Value::Null);
    sessions(
        &fx,
        &[
            (9001, "googleAds", "awaiting_selection", -5000),
            (9002, "googleAds", "awaiting_selection", 0),
            (9003, "metaAds", "awaiting_selection", 1000),
        ],
    )
    .await;
    assert_eq!(
        page_session(&fx, Value::Null).await,
        json!({ "id": "9002", "status": "awaiting_selection" })
    );
}

/// Port of: apps/builder/__tests__/google-ads-load-settings-session.test.ts:68 "a failed reconnect session is returned for display"
/// and apps/builder/__tests__/google-ads-load-settings-page.test.ts:67 "forwards the session param and the configured flag".
#[tokio::test]
async fn load_settings_session_a_failed_reconnect_session_is_returned_for_display() {
    let fx = fixture().await;
    sessions(&fx, &[(9001, "googleAds", "failed", 0)]).await;
    assert_eq!(
        page_session(&fx, json!("9001")).await,
        json!({ "id": "9001", "status": "failed" })
    );
    // load-settings-page :67: the session param is forwarded and the
    // configured flag is false without the platform credential.
    let (_, page) = act(&fx, OWNER_TOKEN, "googleAdsSettingsPage", json!(["9001"])).await;
    assert_eq!(page["isConfigured"], json!(false));
}

/// Port of: apps/builder/__tests__/google-ads-load-settings-session.test.ts:80 "ignores a session of another provider and falls back to the in-flight one"
#[tokio::test]
async fn load_settings_session_ignores_a_session_of_another_provider_and_falls_back_to_the_in_flight_one()
 {
    let fx = fixture().await;
    sessions(
        &fx,
        &[
            (9001, "metaAds", "failed", 0),
            (9002, "googleAds", "awaiting_selection", 0),
        ],
    )
    .await;
    assert_eq!(
        page_session(&fx, json!("9001")).await,
        json!({ "id": "9002", "status": "awaiting_selection" })
    );
}

/// Port of: apps/builder/__tests__/google-ads-load-settings-session.test.ts:158 "does not resume an abandoned `pending` authorization (no endless 'waiting for Google')"
#[tokio::test]
async fn load_settings_session_does_not_resume_an_abandoned_pending_authorization() {
    let fx = fixture().await;
    sessions(&fx, &[(9001, "googleAds", "pending", 0)]).await;
    assert_eq!(page_session(&fx, Value::Null).await, Value::Null);
    // A malformed param is ignored, not looked up.
    assert_eq!(page_session(&fx, json!("9001; drop")).await, Value::Null);
}

/// Port of: apps/builder/__tests__/google-ads-load-settings-session.test.ts:144 "resumes an authorized session (the user is back from Google)"
#[tokio::test]
async fn load_settings_session_resumes_an_authorized_session() {
    let fx = fixture().await;
    sessions(
        &fx,
        &[
            (9001, "googleAds", "authorized", 0),
            (9002, "googleAds", "pending", -1000),
        ],
    )
    .await;
    assert_eq!(
        page_session(&fx, Value::Null).await,
        json!({ "id": "9001", "status": "authorized" })
    );
}

/// Own: the page loads for any member (no secret on it) and shows nothing
/// connected; an outsider is refused; the managing actions and the events
/// list need a super admin.
#[tokio::test]
async fn google_ads_settings_page_and_the_super_admin_guard() {
    let fx = fixture().await;
    let (status, page) = act(&fx, AGENT_TOKEN, "googleAdsSettingsPage", json!([null])).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["setup"], Value::Null);
    assert_eq!(page["isConfigured"], json!(false));
    assert_eq!(page["consent"]["status"], json!("absent"));
    let (status, _) = act(&fx, OUTSIDER_TOKEN, "googleAdsSettingsPage", json!([null])).await;
    assert!(status.is_client_error(), "{status}");

    let consent =
        json!({ "adUserData": { "type": "granted" }, "adPersonalization": { "type": "denied" } });
    let (status, _) = act(
        &fx,
        AGENT_TOKEN,
        "updateGoogleAdsConsentAction",
        json!([WS.to_string(), consent]),
    )
    .await;
    assert!(status.is_client_error(), "{status}");
    let (status, saved) = act(
        &fx,
        OWNER_TOKEN,
        "updateGoogleAdsConsentAction",
        json!([WS.to_string(), consent]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (_, page) = act(&fx, OWNER_TOKEN, "googleAdsSettingsPage", json!([null])).await;
    assert_eq!(page["consent"]["status"], json!("ok"));

    let (status, _) = act(
        &fx,
        AGENT_TOKEN,
        "retryGoogleAdsEventAction",
        json!([WS.to_string(), { "eventId": "1" }]),
    )
    .await;
    assert!(status.is_client_error(), "{status}");
    // The request schema: a malformed click id never reaches Google.
    let (status, _) = act(
        &fx,
        OWNER_TOKEN,
        "validateGoogleAdsRequestAction",
        json!([WS.to_string(), { "conversionActionId": "55", "clickIdType": "gclid", "clickId": "bad id!" }]),
    )
    .await;
    assert!(status.is_client_error(), "{status}");
    let (status, out) = act(
        &fx,
        OWNER_TOKEN,
        "validateGoogleAdsRequestAction",
        json!([WS.to_string(), { "conversionActionId": "55", "clickIdType": "gclid", "clickId": "gclid-1234567890" }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out, json!({ "ok": false, "code": "accountNotReady" }));

    let input = json!({ "workspaceId": WS.to_string(), "perPage": 5 });
    let (status, _) = call(
        &fx,
        "GET",
        "/rpc/googleAdsAPI/listEvents",
        Some(AGENT_TOKEN),
        input.clone(),
    )
    .await;
    assert!(status.is_client_error(), "{status}");
    let (status, list) = call(
        &fx,
        "GET",
        "/rpc/googleAdsAPI/listEvents",
        Some(OWNER_TOKEN),
        input,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(
        list["json"],
        json!({ "data": [], "total": 0, "page": 1, "perPage": 5 })
    );
}
