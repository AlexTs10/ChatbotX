//! `send_pinned`: a redirect is followed with a GET on the pinned address
//! (undici `fetch` through the pinned dispatcher), without the credential on
//! another origin.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener};

use chatbotx_business::decision::transport::send_pinned;

/// Reads one request head (+ body) and answers it.
fn answer(listener: &TcpListener, response: &str) -> String {
    let (mut stream, _) = listener.accept().unwrap();
    let mut request = Vec::new();
    let mut buf = [0u8; 8192];
    let head = loop {
        let n = stream.read(&mut buf).unwrap();
        request.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&request).to_string();
        if let Some((head, rest)) = text.split_once("\r\n\r\n") {
            let length = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(|v| v.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if rest.len() >= length {
                break head.to_ascii_lowercase();
            }
        }
    };
    stream.write_all(response.as_bytes()).unwrap();
    head
}

#[tokio::test]
async fn a_redirect_is_followed_with_a_get_on_the_pinned_address() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let first = answer(
            &listener,
            &format!(
                "HTTP/1.1 302 Found\r\nlocation: http://target.parity.test:{port}/next\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            ),
        );
        let second = answer(
            &listener,
            "HTTP/1.1 404 Not Found\r\ncontent-length: 2\r\nconnection: close\r\n\r\nno",
        );
        (first, second)
    });
    let result = send_pinned(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        &format!("http://router.parity.test:{port}/v1/decide"),
        "sk-secret",
        b"{}".to_vec(),
    )
    .await;
    assert_eq!(result, Ok((404, "no".to_owned())));
    let (first, second) = server.join().unwrap();
    assert!(first.starts_with("post /v1/decide "), "{first}");
    assert!(first.contains("authorization: bearer sk-secret"), "{first}");
    assert!(second.starts_with("get /next "), "{second}");
    assert!(
        second.contains(&format!("host: target.parity.test:{port}")),
        "{second}"
    );
    assert!(!second.contains("authorization"), "{second}");
    assert!(!second.contains("content-type"), "{second}");
}
