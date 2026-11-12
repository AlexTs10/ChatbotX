//! `ChatbotXException` of `@chatbotx.io/business/errors` (the parts the API layer uses).

/// A business error with a code and an HTTP status (default `systemError` / 400).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatbotXException {
    pub message: String,
    pub code: String,
    pub http_status_code: u16,
}

impl ChatbotXException {
    /// `new ChatbotXException(message)`: code `systemError`, status 400.
    pub fn new(message: impl Into<String>) -> ChatbotXException {
        let _ = message;
        todo!("P3.5a")
    }

    /// `new ChatbotXException(message, code, httpStatusCode)`.
    pub fn with_code(
        message: impl Into<String>,
        code: &str,
        http_status_code: u16,
    ) -> ChatbotXException {
        let _ = (message, code, http_status_code);
        todo!("P3.5a")
    }
}
