//! Outgoing mail for the auth endpoints (`@chatbotx.io/mail` send helpers).
//!
//! [`SmtpMailer`] speaks plain SMTP (`smtp://user:pass@host:port`, `EHLO`,
//! optional `AUTH PLAIN`, one text/plain message). TLS (`smtps://`, STARTTLS)
//! and the branded HTML templates come with the mail port.

use std::future::Future;
use std::pin::Pin;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// A boxed send future.
pub type SendFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// Sends one plain-text mail.
pub trait Mailer: Send + Sync {
    fn send<'a>(&'a self, to: &'a str, subject: &'a str, text: &'a str) -> SendFuture<'a>;
}

/// A mailer that drops every mail (no `SMTP_SERVER`).
pub struct NoMailer;

impl Mailer for NoMailer {
    fn send<'a>(&'a self, to: &'a str, _subject: &'a str, _text: &'a str) -> SendFuture<'a> {
        Box::pin(async move {
            tracing::warn!(to, "SMTP_SERVER is not set; mail dropped");
            Ok(())
        })
    }
}

/// Plain SMTP from `SMTP_SERVER` (`smtp://[user:pass@]host[:port]`) and `SMTP_FROM`.
#[derive(Debug, Clone)]
pub struct SmtpMailer {
    host: String,
    port: u16,
    credentials: Option<(String, String)>,
    from: String,
}

impl SmtpMailer {
    /// Parses `SMTP_SERVER`; `None` for anything but `smtp://`.
    pub fn from_url(url: &str, from: &str) -> Option<SmtpMailer> {
        let rest = url.strip_prefix("smtp://")?.trim_end_matches('/');
        let (credentials, address) = match rest.rsplit_once('@') {
            Some((userinfo, address)) => {
                let (user, pass) = userinfo.split_once(':').unwrap_or((userinfo, ""));
                (Some((user.to_string(), pass.to_string())), address)
            }
            None => (None, rest),
        };
        let (host, port) = match address.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().ok()?),
            None => (address, 25),
        };
        Some(SmtpMailer {
            host: host.to_string(),
            port,
            credentials,
            from: from.to_string(),
        })
    }

    async fn deliver(&self, to: &str, subject: &str, text: &str) -> Result<(), String> {
        let stream = TcpStream::connect((self.host.as_str(), self.port))
            .await
            .map_err(|error| format!("connect: {error}"))?;
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        expect_reply(&mut reader, 220).await?;
        command(&mut writer, &mut reader, "EHLO chatbotx\r\n", 250).await?;
        if let Some((user, pass)) = &self.credentials {
            let token = STANDARD.encode(format!("\0{user}\0{pass}"));
            command(
                &mut writer,
                &mut reader,
                &format!("AUTH PLAIN {token}\r\n"),
                235,
            )
            .await?;
        }
        command(
            &mut writer,
            &mut reader,
            &format!("MAIL FROM:<{}>\r\n", self.from),
            250,
        )
        .await?;
        command(
            &mut writer,
            &mut reader,
            &format!("RCPT TO:<{to}>\r\n"),
            250,
        )
        .await?;
        command(&mut writer, &mut reader, "DATA\r\n", 354).await?;
        let body = text
            .replace("\r\n", "\n")
            .replace('\n', "\r\n")
            .replace("\r\n.", "\r\n..");
        let message = format!(
            "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{body}\r\n.\r\n",
            from = self.from
        );
        command(&mut writer, &mut reader, &message, 250).await?;
        command(&mut writer, &mut reader, "QUIT\r\n", 221).await
    }
}

async fn expect_reply<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    code: u16,
) -> Result<(), String> {
    loop {
        let mut line = String::new();
        if reader
            .read_line(&mut line)
            .await
            .map_err(|e| e.to_string())?
            == 0
        {
            return Err("connection closed".to_string());
        }
        let status: u16 = line.get(..3).and_then(|s| s.parse().ok()).unwrap_or(0);
        if line.as_bytes().get(3) == Some(&b'-') {
            continue;
        }
        return if status == code {
            Ok(())
        } else {
            Err(format!("expected {code}, got {}", line.trim_end()))
        };
    }
}

async fn command<W, R>(writer: &mut W, reader: &mut R, line: &str, code: u16) -> Result<(), String>
where
    W: tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncBufRead + Unpin,
{
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    expect_reply(reader, code).await
}

impl Mailer for SmtpMailer {
    fn send<'a>(&'a self, to: &'a str, subject: &'a str, text: &'a str) -> SendFuture<'a> {
        Box::pin(self.deliver(to, subject, text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_smtp_url_of_the_parity_stack() {
        let mailer = SmtpMailer::from_url("smtp://username:password@mailhog:1025", "a@b.test")
            .expect("smtp url");
        assert_eq!(mailer.host, "mailhog");
        assert_eq!(mailer.port, 1025);
        assert_eq!(
            mailer.credentials,
            Some(("username".to_string(), "password".to_string()))
        );
        assert!(SmtpMailer::from_url("smtps://mail:465", "a@b.test").is_none());
    }
}
