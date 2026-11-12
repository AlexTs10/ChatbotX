//! Live gateway helpers (spec 4.2): the axum router on `127.0.0.1:0`, a
//! `tokio-tungstenite` client, raw HTTP upgrade requests (`requestUpgrade`,
//! `requestHttp`, `startUpgrade`) and the token helpers of `gateway.test.ts`.

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use chatbotx_rt::token::{
    ChatScope, MemberConnect, purpose, sign_guest_connect_token, sign_member_connect_token,
};
use chatbotx_rt::{Clock, Realtime, RtConfig, SystemClock, router};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::{Fakes, IO_WAIT, secs, sign_raw_jwt};

/// `SECRET` of `gateway.test.ts`.
pub const SECRET: &str = "gateway-test-secret";
/// `CLOSE_ON_OPEN_MAX_MS`.
pub const CLOSE_ON_OPEN_MAX: Duration = Duration::from_millis(5000);

pub type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// `testGatewayOptions({ maxConnections: 10, secret: SECRET, .. })`.
pub fn gateway_config() -> RtConfig {
    let mut cfg = super::test_config(SECRET);
    cfg.max_connections = 10;
    cfg
}

/// A gateway listening on `127.0.0.1:<port>`.
pub struct TestGateway {
    pub rt: Realtime,
    pub fakes: Fakes,
    pub port: u16,
    server: JoinHandle<()>,
}

impl TestGateway {
    /// `createRealtimeGateway(options)` + `listen("127.0.0.1", 0)`.
    pub async fn start(cfg: RtConfig, fakes: Fakes) -> TestGateway {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let rt = Realtime::with_deps(cfg, fakes.deps(clock));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = router(rt.clone());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        TestGateway {
            rt,
            fakes,
            port,
            server,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("ws://127.0.0.1:{}{path}", self.port)
    }

    /// `gateway.close()`.
    pub async fn close(self) {
        self.rt.shutdown().await;
        self.server.abort();
    }
}

// ---- tokens ----

pub fn now_ms() -> i64 {
    SystemClock.now_ms()
}

/// `signMemberToken(member, secret)` with `issuedAtMs: Date.now()`.
pub fn member_token(workspace_id: &str, user_id: &str, secret: &str) -> String {
    sign_member_connect_token(
        &MemberConnect {
            workspace_id: workspace_id.to_owned(),
            user_id: user_id.to_owned(),
            chat_scope: ChatScope::All,
            team_ids: Vec::new(),
            issued_at_ms: now_ms(),
        },
        secret,
    )
    .unwrap()
}

/// `signGuestConnectToken({ guestConversationId, workspaceId }, secret)`.
pub fn guest_token(guest_conversation_id: &str, workspace_id: &str, secret: &str) -> String {
    sign_guest_connect_token(guest_conversation_id, workspace_id, secret, &SystemClock)
}

/// `signExpiredMemberToken`: exp = now - 120 s.
pub fn expired_member_token(workspace_id: &str, secret: &str) -> String {
    let now = secs(now_ms());
    sign_raw_jwt(
        &json!({
            "chatScope": "all",
            "purpose": purpose::MEMBER_CONNECT,
            "teamIds": [],
            "userId": "user-1",
            "iat": now,
            "aud": format!("workspace:{workspace_id}"),
            "exp": now - 120,
        }),
        secret,
    )
}

/// `signExpiredGuestToken`: exp = now - 120 s.
pub fn expired_guest_token(
    guest_conversation_id: &str,
    workspace_id: &str,
    secret: &str,
) -> String {
    let now = secs(now_ms());
    sign_raw_jwt(
        &json!({
            "guestConversationId": guest_conversation_id,
            "purpose": purpose::GUEST_CONNECT,
            "workspaceId": workspace_id,
            "iat": now,
            "aud": format!("guest:{guest_conversation_id}"),
            "exp": now - 120,
        }),
        secret,
    )
}

/// `signMemberTokenWithIatMs`: an explicit `iatMs`, exp = now + 60 s.
pub fn member_token_with_iat_ms(
    workspace_id: &str,
    user_id: &str,
    iat_ms: i64,
    secret: &str,
) -> String {
    let now = secs(now_ms());
    sign_raw_jwt(
        &json!({
            "chatScope": "all",
            "iatMs": iat_ms,
            "purpose": purpose::MEMBER_CONNECT,
            "teamIds": [],
            "userId": user_id,
            "iat": now,
            "aud": format!("workspace:{workspace_id}"),
            "exp": now + 60,
        }),
        secret,
    )
}

// ---- raw HTTP ----

fn status_code(response: &str) -> Option<u16> {
    let line = response.split("\r\n").next()?;
    let rest = line.strip_prefix("HTTP/1.1 ")?;
    rest.get(..3)?.parse().ok()
}

async fn read_status(mut socket: TcpStream) -> u16 {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let n = tokio::time::timeout(IO_WAIT, socket.read(&mut chunk))
            .await
            .expect("no HTTP response")
            .unwrap();
        assert!(n > 0, "connection closed before a status line: {buffer:?}");
        buffer.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buffer);
        if text.contains("\r\n\r\n") {
            return status_code(&text).unwrap_or_else(|| panic!("unexpected status line: {text}"));
        }
    }
}

fn upgrade_request(path: &str) -> String {
    [
        format!("GET {path} HTTP/1.1"),
        "Host: 127.0.0.1".to_owned(),
        "Connection: Upgrade".to_owned(),
        "Upgrade: websocket".to_owned(),
        "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==".to_owned(),
        "Sec-WebSocket-Version: 13".to_owned(),
        String::new(),
        String::new(),
    ]
    .join("\r\n")
}

/// `requestUpgrade`: the status code of a raw WebSocket upgrade request.
pub async fn request_upgrade(port: u16, path: &str) -> u16 {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    socket
        .write_all(upgrade_request(path).as_bytes())
        .await
        .unwrap();
    read_status(socket).await
}

/// `requestHttp`: the status code of a plain GET.
pub async fn request_http(port: u16, path: &str) -> u16 {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    socket.write_all(request.as_bytes()).await.unwrap();
    read_status(socket).await
}

/// `startUpgrade`: writes an upgrade request and returns the open TCP
/// stream without reading the response (drop it to abort).
pub async fn start_upgrade(port: u16, path: &str) -> TcpStream {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    socket
        .write_all(upgrade_request(path).as_bytes())
        .await
        .unwrap();
    socket
}

// ---- WebSocket client ----

/// `new WebSocket(url)` + `waitForOpen`.
pub async fn connect(url: &str) -> Client {
    let (client, _response) = tokio::time::timeout(IO_WAIT, tokio_tungstenite::connect_async(url))
        .await
        .expect("socket did not open in time")
        .expect("socket errored before opening");
    client
}

/// One observed event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    Text(String),
    /// A close frame `(code, reason)`.
    Close(u16, String),
    /// The stream ended without a close frame.
    Ended,
}

/// The next text frame, close frame or end (pings/pongs skipped).
pub async fn next_event(client: &mut Client) -> Observed {
    loop {
        match client.next().await {
            Some(Ok(Message::Text(text))) => return Observed::Text(text.to_string()),
            Some(Ok(Message::Close(Some(frame)))) => {
                return Observed::Close(u16::from(frame.code), frame.reason.to_string());
            }
            Some(Ok(Message::Close(None))) => return Observed::Close(1005, String::new()),
            Some(Ok(_)) => continue,
            Some(Err(_)) | None => return Observed::Ended,
        }
    }
}

/// `waitForClose`: skips text frames; `(code, reason)` of the close frame.
pub async fn wait_for_close(client: &mut Client) -> (u16, String) {
    tokio::time::timeout(IO_WAIT, async {
        loop {
            match next_event(client).await {
                Observed::Text(_) => continue,
                Observed::Close(code, reason) => return (code, reason),
                Observed::Ended => return (1006, String::new()),
            }
        }
    })
    .await
    .expect("no close in time")
}

/// `waitForMessage`: the first text frame.
pub async fn wait_for_message(client: &mut Client) -> String {
    tokio::time::timeout(IO_WAIT, async {
        match next_event(client).await {
            Observed::Text(text) => text,
            other => panic!("expected a text frame, got {other:?}"),
        }
    })
    .await
    .expect("no message in time")
}

/// Whether no close frame (or end) arrives before `deadline`; text frames
/// are skipped. Returns the close when one arrives.
pub async fn close_before(
    client: &mut Client,
    deadline: tokio::time::Instant,
) -> Option<(u16, String)> {
    loop {
        match tokio::time::timeout_at(deadline, next_event(client)).await {
            Err(_) => return None,
            Ok(Observed::Text(_)) => continue,
            Ok(Observed::Close(code, reason)) => return Some((code, reason)),
            Ok(Observed::Ended) => return Some((1006, String::new())),
        }
    }
}

/// `await delay(ms); expect(socket.readyState).toBe(WebSocket.OPEN)`.
pub async fn stays_open(client: &mut Client, ms: u64) -> bool {
    close_before(
        client,
        tokio::time::Instant::now() + Duration::from_millis(ms),
    )
    .await
    .is_none()
}

/// `socket.close()` + `waitForClose(socket)`.
pub async fn close_client(mut client: Client) {
    let _ = client.close(None).await;
    let _ = tokio::time::timeout(IO_WAIT, async {
        while let Some(Ok(_)) = client.next().await {}
    })
    .await;
}

/// The `retryAfter` of a 4003 close reason `{"retryAfter":<ms>}`.
pub fn retry_after(reason: &str) -> u64 {
    let value: serde_json::Value = serde_json::from_str(reason).expect("JSON close reason");
    value["retryAfter"].as_u64().expect("retryAfter")
}

/// `client.send` of a text frame (the server ignores client frames).
pub async fn send_text(client: &mut Client, text: &str) {
    client.send(Message::text(text)).await.unwrap();
}
