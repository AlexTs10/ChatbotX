//! `integrations/smtp` (`sendMail`: nodemailer `createTransport({ host,
//! port, secure: port === 465, auth })`) and the provider host map.
//!
//! One message per connection: implicit TLS on 465, else plain with
//! STARTTLS when the server offers it (nodemailer's default), `AUTH PLAIN`
//! (or `LOGIN`), one base64 `text/html` part. Like the original, the host is
//! the workspace's own setting (no SSRF check; the known providers are
//! pinned at create / update).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

const TIMEOUT: Duration = Duration::from_secs(30);

/// `smtpHostMap`: the pinned host / port of a known provider (`other` =
/// the caller's values).
pub fn provider_host(provider: &str) -> Option<(&'static str, u16)> {
    Some(match provider {
        "google" => ("smtp.gmail.com", 587),
        "outlook" => ("smtp.office365.com", 587),
        "yahoo" => ("smtp.mail.yahoo.com", 587),
        "sendgrid" => ("smtp.sendgrid.net", 587),
        "mailgun" => ("smtp.mailgun.org", 587),
        "amazon_ses" => ("email-smtp.us-east-1.amazonaws.com", 587),
        "zoho" => ("smtp.zoho.com", 587),
        "postmark" => ("smtp.postmarkapp.com", 587),
        "brevo" => ("smtp-relay.brevo.com", 587),
        _ => return None,
    })
}

pub const PROVIDERS: &[&str] = &[
    "google",
    "outlook",
    "yahoo",
    "sendgrid",
    "mailgun",
    "amazon_ses",
    "zoho",
    "postmark",
    "brevo",
    "other",
];

/// The connection settings (`smtpAuthSchema`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpAuth {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

impl SmtpAuth {
    pub fn from_value(auth: &Value) -> Option<SmtpAuth> {
        let text = |k: &str| {
            auth.get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        Some(SmtpAuth {
            host: text("host")?,
            port: auth
                .get("port")
                .and_then(Value::as_u64)
                .and_then(|p| u16::try_from(p).ok())
                .filter(|p| *p > 0)?,
            username: text("username")?,
            password: text("password")?,
        })
    }
}

/// One mail (`sendMail` options).
#[derive(Debug, Clone)]
pub struct Mail<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub subject: &'a str,
    pub html: &'a str,
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

fn tls_connector() -> Result<tokio_rustls::TlsConnector, String> {
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        use rustls_platform_verifier::ConfigVerifierExt as _;
        rustls::ClientConfig::with_platform_verifier()
            .map(Arc::new)
            .map_err(|e| e.to_string())
    });
    config.clone().map(tokio_rustls::TlsConnector::from)
}

async fn upgrade(stream: TcpStream, host: &str) -> Result<Box<dyn Stream>, String> {
    let name =
        rustls::pki_types::ServerName::try_from(host.to_owned()).map_err(|e| e.to_string())?;
    let tls = tls_connector()?
        .connect(name, stream)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Box::new(tls))
}

/// Reads one (multi-line) reply: the code and the text lines.
async fn reply<S: AsyncRead + Unpin>(
    reader: &mut BufReader<S>,
) -> Result<(u16, Vec<String>), String> {
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        if reader
            .read_line(&mut line)
            .await
            .map_err(|e| e.to_string())?
            == 0
        {
            return Err("SMTP connection closed".into());
        }
        let line = line.trim_end();
        let code = line
            .get(..3)
            .and_then(|c| c.parse::<u16>().ok())
            .ok_or_else(|| format!("Invalid SMTP reply: {line}"))?;
        lines.push(line.get(4..).unwrap_or_default().to_owned());
        if line.as_bytes().get(3) != Some(&b'-') {
            return Ok((code, lines));
        }
    }
}

async fn command<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut BufReader<S>,
    line: &str,
    expect: u16,
) -> Result<Vec<String>, String> {
    stream
        .get_mut()
        .write_all(format!("{line}\r\n").as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let (code, lines) = reply(stream).await?;
    if code / 100 != expect / 100 {
        let verb = line.split_whitespace().next().unwrap_or("");
        let shown = if verb == "AUTH" || expect == 334 {
            "AUTH"
        } else {
            verb
        };
        return Err(format!("SMTP {shown} failed: {code} {}", lines.join(" ")));
    }
    Ok(lines)
}

/// The address of `Name <addr>` (or the value itself).
fn address(value: &str) -> &str {
    match (value.rfind('<'), value.rfind('>')) {
        (Some(start), Some(end)) if start < end => value[start + 1..end].trim(),
        _ => value.trim(),
    }
}

/// RFC 2047 for a non-ASCII header value.
fn header_text(value: &str) -> String {
    if value.is_ascii() {
        value.to_owned()
    } else {
        format!("=?UTF-8?B?{}?=", STANDARD.encode(value))
    }
}

fn message(mail: &Mail<'_>, now: chrono::DateTime<chrono::Utc>) -> String {
    let body = STANDARD.encode(mail.html);
    let wrapped: Vec<&str> = body
        .as_bytes()
        .chunks(76)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect();
    let domain = address(mail.from).rsplit('@').next().unwrap_or("localhost");
    let id = chatbotx_core::utils::id::create_id();
    format!(
        "From: {}\r\nTo: {}\r\nSubject: {}\r\nDate: {}\r\nMessage-ID: <{id}@{domain}>\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n.",
        mail.from,
        mail.to,
        header_text(mail.subject),
        now.to_rfc2822(),
        wrapped.join("\r\n"),
    )
}

async fn ehlo<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut BufReader<S>,
) -> Result<Vec<String>, String> {
    command(stream, "EHLO chatbotx", 250).await
}

async fn greet<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut BufReader<S>,
) -> Result<Vec<String>, String> {
    let (code, _) = reply(stream).await?;
    if code != 220 {
        return Err(format!("SMTP greeting failed: {code}"));
    }
    ehlo(stream).await
}

type Session = BufReader<Box<dyn Stream>>;

/// Connects, greets and (on 465 / when offered) switches to TLS; the
/// session and its EHLO capabilities.
async fn open(auth: &SmtpAuth) -> Result<(Session, Vec<String>), String> {
    let tcp = TcpStream::connect((auth.host.as_str(), auth.port))
        .await
        .map_err(|e| e.to_string())?;
    if auth.port == 465 {
        let mut stream = BufReader::new(upgrade(tcp, &auth.host).await?);
        let caps = greet(&mut stream).await?;
        return Ok((stream, caps));
    }
    let mut plain = BufReader::new(tcp);
    let caps = greet(&mut plain).await?;
    if caps.iter().any(|c| c.eq_ignore_ascii_case("STARTTLS")) {
        command(&mut plain, "STARTTLS", 220).await?;
        let mut stream = BufReader::new(upgrade(plain.into_inner(), &auth.host).await?);
        let caps = ehlo(&mut stream).await?;
        return Ok((stream, caps));
    }
    Ok((
        BufReader::new(Box::new(plain.into_inner()) as Box<dyn Stream>),
        caps,
    ))
}

/// `AUTH PLAIN`, or `AUTH LOGIN` when the server offers only that.
async fn authenticate(
    stream: &mut Session,
    auth: &SmtpAuth,
    caps: &[String],
) -> Result<(), String> {
    let methods = caps
        .iter()
        .find_map(|c| c.strip_prefix("AUTH ").or_else(|| c.strip_prefix("AUTH=")))
        .unwrap_or("PLAIN");
    if methods
        .split_whitespace()
        .any(|m| m.eq_ignore_ascii_case("PLAIN"))
    {
        let token = STANDARD.encode(format!("\0{}\0{}", auth.username, auth.password));
        command(stream, &format!("AUTH PLAIN {token}"), 235).await?;
    } else {
        command(stream, "AUTH LOGIN", 334).await?;
        command(stream, &STANDARD.encode(&auth.username), 334).await?;
        command(stream, &STANDARD.encode(&auth.password), 235).await?;
    }
    Ok(())
}

async fn deliver(auth: &SmtpAuth, mail: &Mail<'_>) -> Result<(), String> {
    for value in [mail.from, mail.to, mail.subject] {
        if value.contains(['\r', '\n']) {
            return Err("Invalid mail header".into());
        }
    }
    let (mut stream, caps) = open(auth).await?;
    authenticate(&mut stream, auth, &caps).await?;
    command(
        &mut stream,
        &format!("MAIL FROM:<{}>", address(mail.from)),
        250,
    )
    .await?;
    command(&mut stream, &format!("RCPT TO:<{}>", address(mail.to)), 250).await?;
    command(&mut stream, "DATA", 354).await?;
    command(&mut stream, &message(mail, chrono::Utc::now()), 250).await?;
    let _ = command(&mut stream, "QUIT", 221).await;
    Ok(())
}

/// `sendMail` (30 s for the whole exchange).
pub async fn send_mail(auth: &SmtpAuth, mail: &Mail<'_>) -> Result<(), String> {
    tokio::time::timeout(TIMEOUT, deliver(auth, mail))
        .await
        .map_err(|_| "SMTP timed out".to_owned())?
}

/// `transporter.verify()`: connect, (TLS,) authenticate, quit.
pub async fn verify(auth: &SmtpAuth) -> Result<(), String> {
    let probe = async {
        let (mut stream, caps) = open(auth).await?;
        authenticate(&mut stream, auth, &caps).await?;
        let _ = command(&mut stream, "QUIT", 221).await;
        Ok(())
    };
    tokio::time::timeout(TIMEOUT, probe)
        .await
        .map_err(|_| "SMTP timed out".to_owned())?
}

// ---- the `email` flow step ---------------------------------------------------

/// `UNSUBSCRIBE_PLACEHOLDER`.
pub const UNSUBSCRIBE_PLACEHOLDER: &str = "<<unsubscribeUrl>>";
const UNSUBSCRIBE_TTL_MS: i64 = 365 * 24 * 60 * 60 * 1000;

/// `buildUnsubscribeUrl`: `{appUrl}/unsubscribe?token=` + base64url of the
/// sealed `{ cid, wid, exp }`.
pub fn unsubscribe_url(app_url: &str, contact_id: i64, workspace_id: i64) -> crate::Result<String> {
    let exp = chrono::Utc::now().timestamp_millis() + UNSUBSCRIBE_TTL_MS;
    let payload = serde_json::json!({ "cid": contact_id.to_string(), "wid": workspace_id.to_string(), "exp": exp });
    let sealed = super::seal_auth(&payload)?;
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sealed);
    Ok(format!("{app_url}/unsubscribe?token={token}"))
}

/// `esc` of the MJML template.
fn esc(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// A resolved mail element (`MailElementSchema`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailElement {
    Heading(String),
    Text(String),
    Code(String),
    Image(Option<String>),
    Button {
        label: Option<String>,
        url: Option<String>,
    },
    Line,
    Spacing,
}

fn section(inner: &str) -> String {
    format!(r#"<tr><td style="padding:10px 25px;">{inner}</td></tr>"#)
}

/// `renderDynamicEmailHtml` (`buildMjmlTemplate`), as plain table HTML.
/// ponytail: no MJML engine in Rust; the same blocks and styles, one column.
pub fn render_email_html(subject: &str, preheader: &str, elements: &[MailElement]) -> String {
    let body: String = elements
        .iter()
        .map(|element| match element {
            // Raw HTML on purpose (as `mj-raw`): the unsubscribe anchor renders.
            MailElement::Heading(text) => section(&format!(
                r#"<div style="font-size:20px;font-weight:700;color:#1d1c1d;line-height:28px;padding:0;">{text}</div>"#
            )),
            MailElement::Text(text) => section(&format!(
                r#"<div style="font-size:16px;color:#3c3f44;line-height:24px;padding:0;">{text}</div>"#
            )),
            MailElement::Code(text) => section(&format!(
                r#"<div style="font-family:monospace,'Courier New';font-size:14px;color:#3c3f44;line-height:22px;background-color:#f4f4f5;padding:12px;">{}</div>"#,
                esc(text).replace('\n', "<br />")
            )),
            MailElement::Image(Some(url)) => section(&format!(
                r#"<img src="{}" alt="" style="display:block;width:100%;max-width:550px;border:0;" />"#,
                esc(url)
            )),
            MailElement::Button { label, url: Some(url) } => section(&format!(
                r#"<a href="{}" style="display:inline-block;background:#111111;color:#ffffff;font-size:14px;font-weight:700;border-radius:4px;padding:10px 25px;text-decoration:none;">{}</a>"#,
                esc(url),
                esc(label.as_deref().unwrap_or("Click here"))
            )),
            MailElement::Line => section(r#"<p style="border-top:1px solid #e8e8e8;margin:0;"></p>"#),
            MailElement::Spacing => r#"<tr><td style="padding:12px 0;"></td></tr>"#.to_owned(),
            MailElement::Image(None) | MailElement::Button { url: None, .. } => String::new(),
        })
        .collect();
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>{}</title></head><body style="margin:0;background-color:#ffffff;"><div style="display:none;max-height:0;overflow:hidden;">{}</div><table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="max-width:600px;margin:0 auto;font-family:Arial,sans-serif;">{body}<tr><td style="padding:16px 25px 0 25px;font-size:12px;color:#888;">⚡ Built with chatbotx.io</td></tr></table></body></html>"#,
        esc(subject),
        esc(preheader),
    )
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use super::*;

    /// A plain SMTP server that records the client lines.
    async fn server() -> (u16, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(socket);
            let mut seen = Vec::new();
            stream.get_mut().write_all(b"220 hi\r\n").await.unwrap();
            let mut in_data = false;
            loop {
                let mut line = String::new();
                if stream.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                let line = line.trim_end().to_owned();
                if in_data {
                    if line == "." {
                        in_data = false;
                        stream.get_mut().write_all(b"250 queued\r\n").await.unwrap();
                    }
                    seen.push(line);
                    continue;
                }
                let answer: &[u8] = match line.split_whitespace().next().unwrap_or("") {
                    "EHLO" => b"250-hello\r\n250 AUTH LOGIN PLAIN\r\n",
                    "AUTH" => b"235 ok\r\n",
                    "DATA" => {
                        in_data = true;
                        b"354 go\r\n"
                    }
                    "QUIT" => b"221 bye\r\n",
                    _ => b"250 ok\r\n",
                };
                seen.push(line);
                stream.get_mut().write_all(answer).await.unwrap();
            }
            seen
        });
        (port, handle)
    }

    #[tokio::test]
    async fn sends_one_html_mail() {
        let (port, handle) = server().await;
        let auth = SmtpAuth {
            host: "127.0.0.1".into(),
            port,
            username: "u".into(),
            password: "p".into(),
        };
        let mail = Mail {
            from: "Shop <shop@example.com>",
            to: "ada@example.com",
            subject: "Hällo",
            html: "<b>hi</b>",
        };
        send_mail(&auth, &mail).await.expect("sent");
        let seen = handle.await.unwrap();
        assert!(seen.contains(&format!("AUTH PLAIN {}", STANDARD.encode("\0u\0p"))));
        assert!(seen.contains(&"MAIL FROM:<shop@example.com>".to_owned()));
        assert!(seen.contains(&"RCPT TO:<ada@example.com>".to_owned()));
        assert!(seen.contains(&format!(
            "Subject: =?UTF-8?B?{}?=",
            STANDARD.encode("Hällo")
        )));
        assert!(seen.contains(&STANDARD.encode("<b>hi</b>")));
    }

    #[tokio::test]
    async fn rejects_header_injection() {
        let auth = SmtpAuth {
            host: "127.0.0.1".into(),
            port: 1,
            username: "u".into(),
            password: "p".into(),
        };
        let mail = Mail {
            from: "a@b.co",
            to: "x@y.co\r\nBcc: z@z.co",
            subject: "s",
            html: "",
        };
        assert_eq!(
            send_mail(&auth, &mail).await.unwrap_err(),
            "Invalid mail header"
        );
    }

    #[test]
    fn renders_blocks() {
        let html = render_email_html(
            "S",
            "pre <x>",
            &[
                MailElement::Text("<a href=\"u\">Unsubscribe</a>".into()),
                MailElement::Code("a<b\nc".into()),
                MailElement::Button {
                    label: None,
                    url: Some("https://x.test/?a=1&b=2".into()),
                },
                MailElement::Button {
                    label: Some("No".into()),
                    url: None,
                },
            ],
        );
        assert!(html.contains("<a href=\"u\">Unsubscribe</a>"));
        assert!(html.contains("a&lt;b<br />c"));
        assert!(html.contains("https://x.test/?a=1&amp;b=2"));
        assert!(html.contains("Click here"));
        assert!(!html.contains(">No<"));
        assert!(html.contains("pre &lt;x&gt;"));
    }

    #[test]
    fn pins_known_hosts() {
        assert_eq!(provider_host("google"), Some(("smtp.gmail.com", 587)));
        assert_eq!(provider_host("other"), None);
        assert_eq!(address("Name <a@b.co>"), "a@b.co");
    }
}
