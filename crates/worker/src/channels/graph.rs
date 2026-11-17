//! The Meta Graph HTTP client the Messenger, Instagram, Instagram-Facebook
//! and Threads channels share (`lib/http-client.ts` of each integration:
//! a `ky` instance with a base URL, a timeout and the retry policy) and
//! `parseOriginError` (`exception.ts`): a failed call keeps the HTTP
//! status and Meta's `{ error: {..} }` fields.

use std::time::Duration;

use chatbotx_core::utils::graph_error::format_graph_error;
use serde_json::Value;

/// Status codes `ky` retries (`retry.statusCodes`).
const RETRY_STATUS: [u16; 7] = [408, 413, 429, 500, 502, 503, 504];

/// `ChannelErrorSource` of `parseOriginError`.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphError {
    pub http_status: u16,
    /// `error.code` as sent (a number or a string).
    pub code: Option<Value>,
    /// `error.error_subcode ?? error.subcode`.
    pub sub_code: Option<Value>,
    /// `error.type`.
    pub kind: Option<String>,
    /// `formatGraphError(error)`, or the transport error text.
    pub message: Option<String>,
    /// No HTTP answer (connection failure).
    pub transport: bool,
}

impl GraphError {
    /// A transport failure (no HTTP answer): `FALLBACK_HTTP_STATUS` 400.
    pub fn network(message: impl Into<String>) -> GraphError {
        GraphError {
            http_status: 400,
            code: None,
            sub_code: None,
            kind: None,
            message: Some(message.into()),
            transport: true,
        }
    }

    /// `parseOriginError` of an HTTP error with its JSON body.
    pub fn from_body(http_status: u16, body: &Value) -> GraphError {
        let error = body.get("error").filter(|e| e.is_object());
        let field = |k: &str| error.and_then(|e| e.get(k)).filter(|v| !v.is_null());
        GraphError {
            http_status,
            code: field("code").cloned(),
            sub_code: field("error_subcode").or_else(|| field("subcode")).cloned(),
            kind: field("type").and_then(Value::as_str).map(str::to_owned),
            message: format_graph_error(error),
            transport: false,
        }
    }

    /// `code` when it is a JSON number (`typeof code === "number"`).
    pub fn numeric_code(&self) -> Option<i64> {
        self.code.as_ref().and_then(Value::as_i64)
    }

    /// `Number(subCode)` (a numeric string counts).
    pub fn numeric_sub_code(&self) -> Option<i64> {
        match self.sub_code.as_ref()? {
            Value::Number(n) => n.as_i64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    /// `code` as text (`String(code)`), `"-1"` (`UNKNOWN_ERROR.code`) when absent.
    pub fn code_text(&self) -> String {
        match &self.code {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => "-1".into(),
        }
    }

    /// `isDataTooLargeGraphError`: code 1, no subcode, "reduce the amount of data".
    pub fn is_data_too_large(&self) -> bool {
        self.numeric_code() == Some(1)
            && self.sub_code.is_none()
            && self
                .message
                .as_deref()
                .is_some_and(|m| m.to_lowercase().contains("reduce the amount of data"))
    }
}

/// One request: method, path below the base URL, bearer token, query and
/// JSON body.
#[derive(Debug, Clone)]
pub struct GraphRequest {
    pub method: reqwest::Method,
    pub path: String,
    pub bearer: Option<String>,
    pub query: Vec<(String, String)>,
    pub json: Option<Value>,
    /// `retry` (`0` for the non-idempotent sends).
    pub retries: u32,
}

impl GraphRequest {
    pub fn new(method: reqwest::Method, path: impl Into<String>) -> GraphRequest {
        GraphRequest {
            method,
            path: path.into(),
            bearer: None,
            query: Vec::new(),
            json: None,
            retries: 3,
        }
    }

    pub fn get(path: impl Into<String>) -> GraphRequest {
        GraphRequest::new(reqwest::Method::GET, path)
    }

    pub fn post(path: impl Into<String>) -> GraphRequest {
        GraphRequest::new(reqwest::Method::POST, path)
    }

    pub fn delete(path: impl Into<String>) -> GraphRequest {
        GraphRequest::new(reqwest::Method::DELETE, path)
    }

    pub fn bearer(mut self, token: &str) -> GraphRequest {
        self.bearer = Some(token.to_owned());
        self
    }

    pub fn query(mut self, key: &str, value: impl Into<String>) -> GraphRequest {
        self.query.push((key.to_owned(), value.into()));
        self
    }

    pub fn json(mut self, body: Value) -> GraphRequest {
        self.json = Some(body);
        self
    }

    pub fn retries(mut self, retries: u32) -> GraphRequest {
        self.retries = retries;
        self
    }
}

/// A `ky` instance: base URL, timeout and retry backoff limit.
#[derive(Clone)]
pub struct GraphHttp {
    client: reqwest::Client,
    pub base_url: String,
    backoff_limit: Duration,
}

impl GraphHttp {
    pub fn new(
        base_url: impl Into<String>,
        timeout: Duration,
        backoff_limit: Duration,
    ) -> GraphHttp {
        GraphHttp {
            client: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .unwrap_or_default(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            backoff_limit,
        }
    }

    /// `facebookGraphClient` (30 s, 3 retries, 1 s backoff limit).
    pub fn facebook(base_url: impl Into<String>) -> GraphHttp {
        GraphHttp::new(base_url, Duration::from_secs(30), Duration::from_secs(1))
    }

    async fn once(&self, request: &GraphRequest) -> Result<Value, GraphError> {
        let mut url = format!("{}/{}", self.base_url, request.path.trim_start_matches('/'));
        if !request.query.is_empty() {
            let query: String = url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(&request.query)
                .finish();
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&query);
        }
        let mut builder = self.client.request(request.method.clone(), url);
        if let Some(token) = &request.bearer {
            builder = builder.bearer_auth(token);
        }
        if let Some(body) = &request.json {
            builder = builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_string());
        }
        let response = builder.send().await.map_err(|e| {
            let mut error = GraphError::network(e.to_string());
            error.transport = !e.is_timeout();
            error
        })?;
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if (200..300).contains(&status) {
            return Ok(parsed);
        }
        Err(GraphError::from_body(status, &parsed))
    }

    /// Sends with `ky`'s retry: the listed statuses, at most `retries`
    /// more times, `0.3 s * 2^(n-1)` capped at the backoff limit; a
    /// "reduce the amount of data" error is not retried (`shouldRetryGraphRequest`).
    pub async fn send(&self, request: GraphRequest) -> Result<Value, GraphError> {
        let mut attempt = 0;
        loop {
            match self.once(&request).await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    // `ky` also retries a transport failure (not a timeout).
                    let retryable = (error.transport || RETRY_STATUS.contains(&error.http_status))
                        && !error.is_data_too_large();
                    if attempt >= request.retries || !retryable {
                        return Err(error);
                    }
                    attempt += 1;
                    let delay = Duration::from_millis(300 * (1u64 << (attempt - 1).min(10)));
                    tokio::time::sleep(delay.min(self.backoff_limit)).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parse_origin_error_reads_the_graph_fields() {
        let error = GraphError::from_body(
            400,
            &json!({ "error": { "message": "Invalid parameter", "type": "OAuthException", "code": 100, "error_subcode": 33 } }),
        );
        assert_eq!(error.numeric_code(), Some(100));
        assert_eq!(error.numeric_sub_code(), Some(33));
        assert_eq!(error.kind.as_deref(), Some("OAuthException"));
        assert_eq!(
            error.message.as_deref(),
            Some("(#100 - 33) Invalid parameter")
        );
        let plain = GraphError::from_body(502, &Value::Null);
        assert_eq!((plain.code_text(), plain.message), ("-1".to_owned(), None));
    }

    #[test]
    fn data_too_large_is_code_1_without_subcode() {
        let error = GraphError::from_body(
            500,
            &json!({ "error": { "message": "Please reduce the amount of data you're asking for, then retry your request", "code": 1 } }),
        );
        assert!(error.is_data_too_large());
        let other = GraphError::from_body(
            500,
            &json!({ "error": { "message": "Unknown", "code": 1 } }),
        );
        assert!(!other.is_data_too_large());
    }
}
