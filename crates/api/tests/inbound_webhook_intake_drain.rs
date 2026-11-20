//! Q.P5-fixD: an early 429 (pre-auth rate limit) still reads the request
//! body, so the keep-alive connection stays usable (the edge proxy answered
//! 502 for requests queued on a connection the server had closed).

mod handlers_common;

use std::sync::Arc;
use std::time::Duration;

use chatbotx_api::inbound_webhook_intake::router;
use chatbotx_kv::{Kv, KvConfig, TestClock};
use handlers_common::fixture;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

const HEAD: &str = "POST /api/hooks/iwh_rrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrr HTTP/1.1\r\nhost: app\r\nx-forwarded-for: 203.0.113.9\r\ncontent-type: application/json\r\ncontent-length: 7\r\n\r\n";
const BODY: &str = "{\"a\":1}";

/// The status of the next response (`None` on EOF), its body skipped.
async fn status(stream: &mut TcpStream) -> Option<u16> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).await.ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
    let length: usize = text
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).await.ok()?;
    text.split(' ').nth(1)?.parse().ok()
}

#[tokio::test]
async fn a_rate_limited_hook_request_keeps_the_connection_open() {
    let mut fx = fixture().await;
    fx.state.kv = Some(Kv::start(
        Arc::clone(&fx.state.db),
        // A fixed clock: on a loaded host the 300 requests can take longer
        // than the 10 s window, and the window would reset before the 301st.
        Arc::new(TestClock::new(1_800_000_000_000)),
        KvConfig::default(),
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(Arc::new(fx.state.clone()));
    tokio::spawn(async move { axum::serve(listener, app).await });

    let mut stream = TcpStream::connect(addr).await.unwrap();
    for _ in 0..300 {
        stream
            .write_all(format!("{HEAD}{BODY}").as_bytes())
            .await
            .unwrap();
        assert_eq!(status(&mut stream).await, Some(404));
    }
    // The body arrives after the server has seen the headers.
    stream.write_all(HEAD.as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    stream.write_all(BODY.as_bytes()).await.unwrap();
    assert_eq!(status(&mut stream).await, Some(429));
    stream
        .write_all(format!("{HEAD}{BODY}").as_bytes())
        .await
        .unwrap();
    assert_eq!(status(&mut stream).await, Some(429));
}
