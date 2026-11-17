//! Own tests (Q.D11 slice 4) of `GET /v1/error-logs` (`errorLogs.list`): the
//! public pick of columns and the pinned newest-first order. The scope test
//! (`error-logs-public-scope`) is ported in P3.5.

mod handlers_common;

use handlers_common::{WS, call, fixture};
use serde_json::json;

#[tokio::test]
async fn lists_the_public_columns_newest_first_with_a_keyword() {
    let fx = fixture().await;
    fx.state
        .db
        .write(|tx| {
            for (id, detail, created) in [(1, "old timeout", 1_000), (2, "new timeout", 2_000), (3, "other", 3_000)] {
                tx.execute(
                    r#"INSERT INTO "ErrorLog" ("id", "workspaceId", "action", "detail", "httpCode", "createdAt", "updatedAt", "sourceId", "stackTrace") VALUES (?1, ?2, 'smtp', ?3, '500', ?4, ?4, 'psid', '    at x')"#,
                    chatbotx_db::rusqlite::params![id, WS, detail, created],
                )?;
            }
            Ok(())
        })
        .await
        .expect("seed");
    let (status, body) = call(
        &fx,
        "errorLogs.list",
        WS,
        &[],
        Some("keyword=timeout"),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["pageCount"], 1);
    let data = body["data"].as_array().expect("data");
    assert_eq!(
        data.iter().map(|r| r["id"].clone()).collect::<Vec<_>>(),
        vec![json!("2"), json!("1")]
    );
    let mut keys: Vec<&str> = data[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "action",
            "contactId",
            "createdAt",
            "detail",
            "httpCode",
            "id",
            "updatedAt",
            "workspaceId"
        ]
    );
}
