//! `packages/business/__tests__/billing.service.test.ts` (Q.D11 slice 10)
//! against a one-shot local HTTP listener instead of a stubbed `fetch`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

use chatbotx_business::billing::{Provision, provision_default_plan};
use chatbotx_business::keys::Edition;

/// Answers one request with `status`; returns the raw request.
fn portal(status: u16) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).unwrap();
            request.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&request).to_string();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if body.len() >= length {
                    break;
                }
            }
        }
        let reply =
            format!("HTTP/1.1 {status} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
        stream.write_all(reply.as_bytes()).unwrap();
        String::from_utf8_lossy(&request).to_string()
    });
    (url, handle)
}

#[tokio::test]
async fn is_a_no_op_off_cloud() {
    for edition in [Edition::Community, Edition::Enterprise] {
        let out = provision_default_plan(edition, "http://127.0.0.1:9", 1, Some(2)).await;
        assert_eq!(out, Provision::Skipped);
    }
}

#[tokio::test]
async fn posts_user_id_and_tenant_id_to_the_portal_on_cloud() {
    let (url, handle) = portal(200);
    let out = provision_default_plan(Edition::Cloud, &format!("{url}/"), 11, Some(1)).await;
    assert_eq!(out, Provision::Answered(200));
    let request = handle.join().unwrap();
    assert!(
        request.starts_with("POST /portal/api/users/provision HTTP/1.1"),
        "{request}"
    );
    let body = request.split_once("\r\n\r\n").unwrap().1;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(body).unwrap(),
        serde_json::json!({ "userId": "11", "tenantId": "1" })
    );
}

/// "swallows and logs a portal request failure" / "logs a non-OK portal
/// response without throwing".
#[tokio::test]
async fn a_failure_or_a_non_ok_status_is_not_an_error() {
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    assert!(matches!(
        provision_default_plan(Edition::Cloud, &url, 11, None).await,
        Provision::Failed(_)
    ));

    let (url, handle) = portal(500);
    assert_eq!(
        provision_default_plan(Edition::Cloud, &url, 11, None).await,
        Provision::Answered(500)
    );
    let request = handle.join().unwrap();
    assert!(request.ends_with(r#"{"userId":"11"}"#), "{request}");
}
