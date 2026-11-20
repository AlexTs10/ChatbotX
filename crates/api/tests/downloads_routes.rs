//! Ports of `apps/builder/__tests__/flow-export-route.test.ts`,
//! `ads-analytics-export-route.test.ts`, `me-extension-download-route.test.ts`
//! and `public-spec-json-route.test.ts` (Q.P6-U5). Adapted: the mocked
//! services become real rows behind `downloads::router` /
//! `public_spec::router`; "called with X" becomes the answer. The ads CSV is
//! built in memory (no pull-driven page test); `getTranslations` gives the
//! real English labels, not the keys.

mod rpc_common;

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chatbotx_api::downloads::{DownloadState, router};
use chatbotx_api::session::sign_cookie_value;
use chatbotx_core::encryption::MeLinkParams;
use chatbotx_db::rusqlite::params;
use rpc_common::{AGENT_TOKEN, Fixture, OWNER_TOKEN, SECRET, WS, encryption, fixture};
use serde_json::{Value, json};
use tower::ServiceExt as _;

struct Answer {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    bytes: Vec<u8>,
}

impl Answer {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.bytes).expect("JSON body")
    }
    fn header(&self, name: header::HeaderName) -> &str {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
    }
}

async fn get(fx: &Fixture, uri: &str, token: Option<&str>, cookie: Option<&str>) -> Answer {
    let app = router(DownloadState {
        auth: Arc::clone(&fx.auth),
        api: Arc::clone(&fx.api),
    })
    .merge(chatbotx_api::public_spec::router(Arc::clone(&fx.api)));
    let mut cookies = Vec::new();
    if let Some(token) = token {
        cookies.push(format!(
            "better-auth.session_token={}",
            sign_cookie_value(token, SECRET)
        ));
    }
    if let Some(cookie) = cookie {
        cookies.push(cookie.to_string());
    }
    let mut request = Request::builder().uri(uri).header(header::HOST, "app.test");
    if !cookies.is_empty() {
        request = request.header(header::COOKIE, cookies.join("; "));
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Answer {
        status,
        headers,
        bytes,
    }
}

// ---- flow-export-route.test.ts ----------------------------------------------

/// Flow 61 "Onboarding" (published), flow 62 (draft only).
async fn flows(fx: &Fixture) {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (61, ?1, 'Onboarding'), (62, ?1, 'Draft')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "isLatest", "startNodeId")
                   VALUES (611, ?1, 61, '[]', '[]', 0, 1, 1), (621, ?1, 62, '[]', '[]', 1, 1, 1)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed flows");
}

#[tokio::test]
async fn flow_export_denies_access_with_a_bare_404_when_the_user_lacks_permission() {
    let fx = fixture().await;
    flows(&fx).await;
    let uri = format!("/space/{WS}/flows/61/export");
    let answer = get(&fx, &uri, Some(AGENT_TOKEN), None).await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert!(answer.bytes.is_empty());
    assert_eq!(
        get(&fx, &uri, None, None).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn flow_export_returns_404_when_the_flow_does_not_exist() {
    let fx = fixture().await;
    let answer = get(
        &fx,
        &format!("/space/{WS}/flows/99/export"),
        Some(OWNER_TOKEN),
        None,
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn flow_export_returns_a_distinct_not_published_error_instead_of_a_blank_404() {
    let fx = fixture().await;
    flows(&fx).await;
    let answer = get(
        &fx,
        &format!("/space/{WS}/flows/62/export"),
        Some(OWNER_TOKEN),
        None,
    )
    .await;
    assert_eq!(answer.status, StatusCode::CONFLICT);
    assert_eq!(answer.json(), json!({ "code": "notPublished" }));
}

#[tokio::test]
async fn flow_export_downloads_the_export_as_a_json_attachment() {
    let fx = fixture().await;
    flows(&fx).await;
    let answer = get(
        &fx,
        &format!("/space/{WS}/flows/61/export"),
        Some(OWNER_TOKEN),
        None,
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    assert_eq!(
        answer.header(header::CONTENT_DISPOSITION),
        "attachment; filename=\"onboarding.chatbotx-flow.json\""
    );
    assert_eq!(answer.header(header::CACHE_CONTROL), "no-store");
    let body = answer.json();
    assert_eq!(body["formatVersion"], 2);
    assert_eq!(body["flows"][0]["name"], "Onboarding");
    // `JSON.stringify(envelope, null, 2)`.
    assert!(answer.text().starts_with("{\n  \""));
}

// ---- ads-analytics-export-route.test.ts --------------------------------------

/// WhatsApp integration 101 (inbox 6); contacts 1 "Ada Lovelace"
/// (+12025550101) and 2 "Nguyễn Văn Bình"; lead 501 (ad-1, 2026-08-05) and
/// purchase 502.
async fn conversions(fx: &Fixture) {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (6, ?1, 'Wa', 'whatsapp', 'wa-6')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId")
                   VALUES (101, '{}', 'phone-1', 'waba-1', 'b', 'Wa', ?1, 6)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "lastName", "phoneNumber", "workspaceId")
                   VALUES (1, 'Ada', 'Lovelace', '+12025550101', ?1), (2, 'Nguyễn', 'Văn Bình', NULL, ?1)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
                   VALUES (11, 1, 1, 6, 'whatsapp', 'direct', 's1'), (12, 2, 2, 6, 'whatsapp', 'direct', 's2')"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "AdsConversionEvent" ("id", "workspaceId", "channel", "integrationWhatsappId", "wabaId", "source", "eventType", "ctwaClid", "adId", "contactInboxId", "occurredAt", "sourceEventId")
                   VALUES (501, ?1, 'whatsapp', 101, 'waba-1', 'rule', 'lead', 'clid', 'ad-1', 11, 1786008600000, 'e1'),
                          (502, ?1, 'whatsapp', 101, 'waba-1', 'rule', 'lead', 'clid', 'ad-2', 12, 1786095000000, 'e2')"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed conversions");
}

fn export_uri(query: &str) -> String {
    format!("/space/{WS}/dashboard/ads/export?{query}")
}

#[tokio::test]
async fn ads_export_propagates_the_workspace_guard_without_exporting_rows() {
    let fx = fixture().await;
    let answer = get(
        &fx,
        &export_uri("segment=leads&from=2026-08-01&to=2026-08-10"),
        Some(AGENT_TOKEN),
        None,
    )
    .await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn ads_export_returns_invalid_request_for_an_invalid_query() {
    let fx = fixture().await;
    for query in [
        "segment=&from=2026-08-01&to=2026-08-10",
        "segment=leads&channel=tiktok&from=2026-08-01&to=2026-08-10",
    ] {
        let answer = get(&fx, &export_uri(query), Some(OWNER_TOKEN), None).await;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(answer.json(), json!({ "code": "invalidRequest" }));
    }
}

#[tokio::test]
async fn ads_export_prefixes_a_bom_and_keeps_the_legacy_shape_without_a_channel() {
    let fx = fixture().await;
    conversions(&fx).await;
    let answer = get(
        &fx,
        &export_uri("segment=leads&integrationWhatsappId=101&from=2026-08-01&to=2026-08-10"),
        Some(OWNER_TOKEN),
        None,
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    assert_eq!(
        answer.header(header::CONTENT_TYPE),
        "text/csv; charset=utf-8"
    );
    assert_eq!(
        answer.header(header::CONTENT_DISPOSITION),
        "attachment; filename=\"ctwa-leads-2026-08-01-2026-08-10.csv\""
    );
    assert_eq!(answer.header(header::CACHE_CONTROL), "private, max-age=60");
    assert_eq!(answer.bytes[..3], [0xef, 0xbb, 0xbf]);
    assert_eq!(
        String::from_utf8_lossy(&answer.bytes[3..]),
        "Contact name,Phone,Ad ID,Occurred at\n\
         Ada Lovelace,'+12025550101,ad-1,2026-08-06T09:30:00.000Z\n\
         Nguyễn Văn Bình,,ad-2,2026-08-07T09:30:00.000Z\n"
    );
}

#[tokio::test]
async fn ads_export_with_a_channel_gets_the_five_column_format() {
    let fx = fixture().await;
    conversions(&fx).await;
    let answer = get(
        &fx,
        &export_uri("segment=leads&channel=whatsapp&adId=ad-1&from=2026-08-01&to=2026-08-10"),
        Some(OWNER_TOKEN),
        Some("NEXT_LOCALE=de"),
    )
    .await;
    assert_eq!(
        answer.header(header::CONTENT_DISPOSITION),
        "attachment; filename=\"ads-whatsapp-leads-2026-08-01-2026-08-10.csv\""
    );
    assert_eq!(
        String::from_utf8_lossy(&answer.bytes[3..]),
        "Kontaktname,Telefon,Anzeigen-ID,Aufgetreten am,Kanal\n\
         Ada Lovelace,'+12025550101,ad-1,2026-08-06T09:30:00.000Z,WhatsApp\n"
    );
}

#[tokio::test]
async fn ads_export_all_channels_streams_each_rows_channel_with_a_neutral_file_name() {
    let fx = fixture().await;
    conversions(&fx).await;
    let answer = get(
        &fx,
        &export_uri(
            "segment=leads&channel=all&integrationWhatsappId=1&from=2026-08-01&to=2026-08-10",
        ),
        Some(OWNER_TOKEN),
        None,
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert!(
        answer
            .header(header::CONTENT_DISPOSITION)
            .contains("filename=\"ads-all-leads-2026-08-01-2026-08-10.csv\"")
    );
    // The integration id is ignored (not part of the all-channel schema).
    assert_eq!(
        String::from_utf8_lossy(&answer.bytes[3..]),
        "Contact name,Phone,Ad ID,Occurred at,Channel\n\
         Ada Lovelace,'+12025550101,ad-1,2026-08-06T09:30:00.000Z,WhatsApp\n\
         Nguyễn Văn Bình,,ad-2,2026-08-07T09:30:00.000Z,WhatsApp\n"
    );
    // The old bookmark path answers the same.
    let old = get(
        &fx,
        &format!("/space/{WS}/ads/analytics/export?segment=leads&channel=all&from=2026-08-01&to=2026-08-10"),
        Some(OWNER_TOKEN),
        None,
    )
    .await;
    assert_eq!(old.bytes, answer.bytes);
}

// ---- api/imports/products/template ------------------------------------------

#[tokio::test]
async fn the_product_import_template_follows_the_locale_cookie() {
    let fx = fixture().await;
    let en = get(&fx, "/api/imports/products/template", None, None).await;
    assert_eq!(en.status, StatusCode::OK);
    assert_eq!(
        en.header(header::CONTENT_TYPE),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert_eq!(en.header(header::CACHE_CONTROL), "private, max-age=300");
    assert_eq!(en.bytes[..2], *b"PK");
    let vi = get(
        &fx,
        "/api/imports/products/template",
        None,
        Some("NEXT_LOCALE=vi"),
    )
    .await;
    assert_ne!(
        vi.header(header::CONTENT_DISPOSITION),
        en.header(header::CONTENT_DISPOSITION)
    );
}

// ---- me-extension-download-route.test.ts --------------------------------------

/// The "me" form 701 of contact 1 / contact inbox 11 (`src-1`, integration 101).
async fn me_form(fx: &Fixture) -> String {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (6, ?1, 'Wa', 'whatsapp', 'wa-6')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "lastName", "email", "gender", "locale", "workspaceId")
                   VALUES (1, 'Ada', 'Lovelace', 'ada@example.com', 'female', 'en_US', ?1)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "ContactInbox" ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
                   VALUES (11, 1, 1, 6, 'whatsapp', 'direct', 'src-1')"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "SystemField" ("id", "type", "payload") VALUES (701, 'me', ?1)"#,
                [json!({
                    "workspaceId": WS.to_string(), "channel": "whatsapp", "integrationId": "101",
                    "sourceId": "src-1", "contactInboxId": "11", "contactId": "1",
                })
                .to_string()],
            )?;
            Ok(())
        })
        .await
        .expect("seed me form");
    encryption().sign_me_link(&MeLinkParams {
        workspace_id: WS.to_string(),
        source_id: "src-1".to_string(),
        integration_id: "101".to_string(),
        form_id: "701".to_string(),
    })
}

#[tokio::test]
async fn me_download_returns_export_json_with_an_attachment_file_name() {
    let fx = fixture().await;
    let hash = me_form(&fx).await;
    let answer = get(
        &fx,
        &format!("/extensions/me/download?w={WS}&u=src-1&ib=101&id=701&hash={hash}"),
        None,
        None,
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    assert_eq!(
        answer.header(header::CONTENT_DISPOSITION),
        "attachment; filename=\"src-1.json\""
    );
    let body = answer.json();
    assert_eq!(
        body["data"],
        json!({
            "id": "src-1", "first_name": "Ada", "last_name": "Lovelace",
            "full_name": "Ada Lovelace", "email": "ada@example.com", "phone": "",
            "locale": "en_US", "timezone": null, "gender": "Female", "profile_pic": null,
        })
    );
    assert_eq!(body["messages"], json!([]));
    assert!(body["generated"].as_str().unwrap().ends_with(" (UTC)"));
}

#[tokio::test]
async fn me_download_returns_404_when_params_are_missing_or_invalid() {
    let fx = fixture().await;
    me_form(&fx).await;
    let missing = get(
        &fx,
        &format!("/extensions/me/download?w={WS}&u=src-1"),
        None,
        None,
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let invalid = get(
        &fx,
        &format!("/extensions/me/download?w={WS}&u=src-1&ib=101&id=701&hash=bad"),
        None,
        None,
    )
    .await;
    assert_eq!(invalid.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn me_download_returns_410_when_the_workspace_is_scheduled_for_deletion() {
    let fx = fixture().await;
    let hash = me_form(&fx).await;
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"UPDATE "Workspace" SET "scheduledDeletionAt" = 1 WHERE "id" = ?1"#,
                [WS],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let answer = get(
        &fx,
        &format!("/extensions/me/download?w={WS}&u=src-1&ib=101&id=701&hash={hash}"),
        None,
        None,
    )
    .await;
    assert_eq!(answer.status, StatusCode::GONE);
    assert_eq!(
        answer.json(),
        json!({ "code": "workspaceScheduledDeletion" })
    );
}

// ---- public-spec-json-route.test.ts --------------------------------------------

fn is_sha1_etag(etag: &str) -> bool {
    etag.len() == 42
        && etag.starts_with('"')
        && etag.ends_with('"')
        && etag[1..41]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[tokio::test]
async fn public_spec_returns_the_document_with_an_etag_and_cache_control() {
    let fx = fixture().await;
    let answer = get(&fx, "/api/public-spec.json", None, None).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.header(header::CONTENT_TYPE), "application/json");
    assert!(is_sha1_etag(answer.header(header::ETAG)));
    assert_eq!(
        answer.header(header::CACHE_CONTROL),
        "public, max-age=300, stale-while-revalidate=3600"
    );
    let body = answer.json();
    assert!(body.get("openapi").is_some());
    assert_eq!(body["info"]["title"], "ChatbotX");
    assert_eq!(body["servers"], json!([{ "url": "http://app.test/api" }]));
    assert_eq!(
        body["paths"]
            .as_object()
            .unwrap()
            .values()
            .map(|item| item.as_object().unwrap().len())
            .sum::<usize>(),
        chatbotx_api::routes::PUBLIC_ROUTES.len()
    );
}

#[tokio::test]
async fn public_spec_answers_304_and_scopes_the_etag_by_filter_and_tenant() {
    let fx = fixture().await;
    let first = get(&fx, "/api/public-spec.json", None, None).await;
    let etag = first.header(header::ETAG).to_string();
    let again = get(&fx, "/api/public-spec.json", None, None).await;
    assert_eq!(again.header(header::ETAG), etag);

    let app = chatbotx_api::public_spec::router(Arc::clone(&fx.api));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/public-spec.json")
                .header(header::HOST, "app.test")
                .header(header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(response.headers()[header::ETAG], etag.as_str());
    assert!(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .is_empty()
    );

    let filtered = get(&fx, "/api/public-spec.json?filter=contacts", None, None).await;
    assert_ne!(filtered.header(header::ETAG), etag);
    let paths = filtered.json()["paths"].as_object().unwrap().clone();
    assert!(!paths.is_empty());
    assert!(paths.keys().all(|path| path.starts_with("/v1/contacts")));

    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Tenant" ("id", "brandName") VALUES (1, 'OtherTenant')
                   ON CONFLICT ("id") DO UPDATE SET "brandName" = 'OtherTenant'"#,
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    // Without the enterprise features the brand name is not used.
    let community = get(&fx, "/api/public-spec.json", None, None).await;
    assert_eq!(community.header(header::ETAG), etag);
    assert_eq!(community.json()["info"]["title"], "ChatbotX");
    // A licensed enterprise edition: the tenant's name, its own ETag.
    let (token, keys) = rpc_common::license("enterprise");
    let mut licensed = (*fx.api).clone().with_license(Some(token), keys);
    licensed.edition = chatbotx_business::keys::Edition::Enterprise;
    let enterprise = Fixture {
        auth: Arc::clone(&fx.auth),
        api: Arc::new(licensed),
    };
    let other = get(&enterprise, "/api/public-spec.json", None, None).await;
    assert_ne!(other.header(header::ETAG), etag);
    assert_eq!(other.json()["info"]["title"], "OtherTenant");
}

#[tokio::test]
async fn the_scalar_ui_page_reads_the_public_spec() {
    let fx = fixture().await;
    let answer = get(&fx, "/api/scalar-ui", None, None).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.header(header::CONTENT_TYPE), "text/html");
    assert!(answer.text().contains("url: '/api/public-spec.json'"));
}
