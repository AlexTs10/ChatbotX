//! `packages/business/src/errors.ts`: the exceptions a service throws.
//!
//! A [`BusinessError::Exception`] is a `ChatbotXException` (message, code, HTTP
//! status, optional structured `data`); the API maps it to an oRPC error with
//! the same code and status. A repository failure stays [`BusinessError::Repo`]
//! (an unexpected error: 500 at the API).

use std::fmt;
use std::sync::LazyLock;

use chatbotx_db::repo;
use regex::Regex;
use serde_json::Value;

/// An error of a business service.
#[derive(Debug)]
pub enum BusinessError {
    /// `ChatbotXException(message, code, httpStatusCode)` (+ `field` / `data`).
    Exception {
        message: String,
        code: String,
        status: u16,
        field: Option<String>,
        data: Option<Value>,
    },
    /// A repository / database error (rethrown as is in the original).
    Repo(repo::Error),
}

/// Result of a business service call.
pub type Result<T, E = BusinessError> = std::result::Result<T, E>;

impl BusinessError {
    /// `new ChatbotXException(message, code, status)`.
    pub fn exception(message: impl Into<String>, code: &str, status: u16) -> BusinessError {
        BusinessError::Exception {
            message: message.into(),
            code: code.to_string(),
            status,
            field: None,
            data: None,
        }
    }

    /// `notFoundException(message)` (`notFound`, 404).
    pub fn not_found(message: impl Into<String>) -> BusinessError {
        BusinessError::exception(message, "notFound", 404)
    }

    /// `validationException(field, message)` (`validation`, 422).
    pub fn validation(field: &str, message: impl Into<String>) -> BusinessError {
        BusinessError::Exception {
            message: message.into(),
            code: "validation".to_string(),
            status: 422,
            field: Some(field.to_string()),
            data: None,
        }
    }

    /// `conflictException(message, code)` (409).
    pub fn conflict(message: impl Into<String>, code: &str) -> BusinessError {
        BusinessError::exception(message, code, 409)
    }

    /// The exception code (`None` for a repository error).
    pub fn code(&self) -> Option<&str> {
        match self {
            BusinessError::Exception { code, .. } => Some(code),
            BusinessError::Repo(_) => None,
        }
    }

    /// The HTTP status (`None` for a repository error).
    pub fn status(&self) -> Option<u16> {
        match self {
            BusinessError::Exception { status, .. } => Some(*status),
            BusinessError::Repo(_) => None,
        }
    }

    /// The message.
    pub fn message(&self) -> String {
        match self {
            BusinessError::Exception { message, .. } => message.clone(),
            BusinessError::Repo(error) => error.message(),
        }
    }
}

impl fmt::Display for BusinessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BusinessError::Exception { message, code, .. } => write!(f, "{code}: {message}"),
            BusinessError::Repo(error) => write!(f, "{}", error.message()),
        }
    }
}

impl std::error::Error for BusinessError {}

impl From<repo::Error> for BusinessError {
    fn from(error: repo::Error) -> BusinessError {
        BusinessError::Repo(error)
    }
}

impl From<chatbotx_db::DbError> for BusinessError {
    fn from(error: chatbotx_db::DbError) -> BusinessError {
        BusinessError::Repo(repo::Error::Db(error))
    }
}

// ---- errors.ts: sanitizePublicText --------------------------------------

const KEYS: &str = "access[_-]?token|refresh[_-]?token|auth[_-]?token|session[_-]?token|id[_-]?token|client[_-]?id[_-]?token|password|secret|client[_-]?secret|consumer[_-]?secret|app[_-]?secret|private[_-]?key|api[_-]?key|authorization";

static QUERY_DUMP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)failed\s+query:").unwrap());

/// The `/failed\s+query:/i` check of `toPublicErrorMessage` (a SQL dump).
pub fn is_query_dump(message: &str) -> bool {
    QUERY_DUMP.is_match(message)
}
static AUTHORIZATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bAuthorization\s*[:=]\s*(?:Basic|Bearer)\s+[A-Za-z0-9._~+/=-]+").unwrap()
});
static BEARER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+").unwrap());
// The original's `(["']?)key\1 ... (["'])...\4` back-references, spelled out
// as alternatives (the `regex` crate has no back-references).
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"(?i)(?:"({KEYS})"|'({KEYS})'|({KEYS}))(\s*[:=]\s*)("[^"'\r\n]*"|'[^"'\r\n]*'|[^,\s}}&]+)"#
    ))
    .unwrap()
});
static QUERY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)([?&](?:access_token|refresh_token|auth_token|session_token|id_token|client_id_token|token|password|secret|client_secret|consumer_secret|app_secret|private_key|api_key|authorization)=)[^&\s]+").unwrap()
});
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// `sanitizePublicText` (500 characters, URLs kept).
pub fn sanitize_public_text(value: &str) -> String {
    let flat: String = value
        .chars()
        .map(|c| {
            if (c as u32) < 32 || c as u32 == 127 {
                ' '
            } else {
                c
            }
        })
        .collect();
    let text = AUTHORIZATION.replace_all(&flat, "Authorization: [REDACTED]");
    let text = BEARER.replace_all(&text, "Bearer [REDACTED]");
    let text = ASSIGNMENT.replace_all(&text, |c: &regex::Captures| {
        let (quote, key) = match (c.get(1), c.get(2), c.get(3)) {
            (Some(k), _, _) => ("\"", k.as_str()),
            (_, Some(k), _) => ("'", k.as_str()),
            (_, _, k) => ("", k.map_or("", |k| k.as_str())),
        };
        let value = &c[5];
        let value_quote = match value.chars().next() {
            Some(q @ ('"' | '\'')) if value.len() > 1 => q.to_string(),
            _ => String::new(),
        };
        let redacted = if value_quote.is_empty() {
            "[REDACTED]".to_owned()
        } else {
            format!("{value_quote}[REDACTED]{value_quote}")
        };
        format!("{quote}{key}{quote}{}{redacted}", &c[4])
    });
    let text = QUERY.replace_all(&text, "${1}[REDACTED]");
    let text = WHITESPACE.replace_all(&text, " ");
    text.trim().chars().take(500).collect()
}
