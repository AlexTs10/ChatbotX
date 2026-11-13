//! `packages/business/src/errors.ts`: the exceptions a service throws.
//!
//! A [`BusinessError::Exception`] is a `ChatbotXException` (message, code, HTTP
//! status, optional structured `data`); the API maps it to an oRPC error with
//! the same code and status. A repository failure stays [`BusinessError::Repo`]
//! (an unexpected error: 500 at the API).

use std::fmt;

use chatbotx_db::repo;
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
