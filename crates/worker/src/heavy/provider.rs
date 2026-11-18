//! The provider calls of the heavy steps (the AI SDK `generateImage` /
//! `generateSpeech` wire formats, and `packages/ai/src/server/sam.ts`).
//! Keys come from the workspace's integration row through
//! `ai::model::native_model`, as the other AI steps.

use std::sync::LazyLock;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use crate::ai::llm::Model;

/// `aiTimeouts.aiTotal`.
pub const AI_TOTAL: Duration = Duration::from_secs(120);
/// `MUSE_BASE_URL`.
const MUSE_BASE_URL: &str = "https://api.meta.ai/v1";
const SAM_MODEL: &str = "sam-3.1";
/// `SAM_MAX_OUTPUT_TEXT_CHARS` (also bounds the whole SSE body here).
pub const SAM_MAX_BODY_CHARS: usize = 16_000_000;
const ERROR_BODY_CHARS: usize = 4096;

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(AI_TOTAL)
        .build()
        .unwrap_or_default()
});

/// A heavy-step failure with the `isRetryableHeavyError` verdict.
#[derive(Debug, Clone, PartialEq)]
pub struct HeavyError {
    pub message: String,
    pub retryable: bool,
    pub status: Option<u16>,
}

impl HeavyError {
    /// `ExpectedHeavyStepError`: a user-input failure, never retried.
    pub fn expected(message: impl Into<String>) -> HeavyError {
        HeavyError {
            message: message.into(),
            retryable: false,
            status: None,
        }
    }

    /// An unknown / transport failure: retried.
    pub fn transient(message: impl Into<String>) -> HeavyError {
        HeavyError {
            message: message.into(),
            retryable: true,
            status: None,
        }
    }

    /// A provider HTTP failure: only 408 / 429 / 5xx are retried.
    pub fn http(status: u16, message: impl Into<String>) -> HeavyError {
        HeavyError {
            message: message.into(),
            retryable: status == 408 || status == 429 || status >= 500,
            status: Some(status),
        }
    }
}

/// One generated file.
#[derive(Debug, Clone, PartialEq)]
pub struct Generated {
    pub bytes: Vec<u8>,
    pub media_type: String,
}

fn snippet(text: &str) -> String {
    text.chars().take(ERROR_BODY_CHARS).collect()
}

/// Sends `request`; a non-2xx answer is a [`HeavyError::http`].
async fn send(
    request: reqwest::RequestBuilder,
    label: &str,
) -> Result<(bytes::Bytes, Option<String>), HeavyError> {
    let response = request
        .send()
        .await
        .map_err(|e| HeavyError::transient(format!("Cannot connect to API: {e}")))?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = response
        .bytes()
        .await
        .map_err(|e| HeavyError::transient(e.to_string()))?;
    if !(200..300).contains(&status) {
        let text = String::from_utf8_lossy(&body);
        return Err(HeavyError::http(
            status,
            format!(
                "{label} request failed with status {status}: {}",
                snippet(&text)
            ),
        ));
    }
    Ok((body, content_type))
}

fn json_of(body: &[u8]) -> Result<Value, HeavyError> {
    serde_json::from_slice(body)
        .map_err(|e| HeavyError::transient(format!("Invalid JSON response: {e}")))
}

fn decode_b64(data: &str) -> Result<Vec<u8>, HeavyError> {
    STANDARD
        .decode(data.trim())
        .map_err(|e| HeavyError::transient(format!("Invalid base64 payload: {e}")))
}

/// The image media type from its bytes (the AI SDK detects it the same
/// way); PNG when unknown.
pub fn image_media_type(bytes: &[u8]) -> String {
    match chatbotx_core::utils::channel_safe_image::sniff_format(bytes) {
        Some(format) => format!("image/{format}"),
        None => "image/png".into(),
    }
}

/// What an image request carries besides the model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImageRequest<'a> {
    pub prompt: &'a str,
    /// `WxH` (OpenAI / Muse).
    pub size: Option<&'a str>,
    /// `W:H` (Gemini).
    pub aspect_ratio: Option<&'a str>,
    /// The OpenAI `quality` provider option.
    pub quality: Option<&'a str>,
    /// The input image of an edit.
    pub input: Option<&'a [u8]>,
}

/// `generateImage({ model, prompt, size, aspectRatio, providerOptions })`:
/// the first image's bytes, or (dall-e) the URL it answered with.
pub enum ImageAnswer {
    Bytes(Generated),
    Url(String),
}

/// One `generateImage` call for `model` (`openai`, `muse`, `gemini`).
pub async fn generate_image(
    model: &Model,
    req: &ImageRequest<'_>,
) -> Result<ImageAnswer, HeavyError> {
    match model.provider.as_str() {
        "openai" | "muse" => openai_image(model, req).await,
        "gemini" => gemini_image(model, req).await,
        other => Err(HeavyError::expected(format!(
            "Provider {other} does not support image generation"
        ))),
    }
}

fn bearer(model: &Model) -> String {
    format!("Bearer {}", model.api_key.as_deref().unwrap_or(""))
}

/// The JSON body of `/images/generations`.
pub fn openai_generation_body(model: &Model, req: &ImageRequest<'_>) -> Value {
    let mut body = json!({ "model": model.model_id, "prompt": req.prompt, "n": 1 });
    if let Some(size) = req.size {
        body["size"] = json!(size);
    }
    if let Some(quality) = req.quality {
        body["quality"] = json!(quality);
    }
    // `@ai-sdk/openai-compatible` asks Muse for base64; OpenAI's gpt-image
    // models always answer base64 and dall-e's `response_format` is stripped
    // (`createDallEImageFetch`).
    if model.provider == "muse" {
        body["response_format"] = json!("b64_json");
    }
    body
}

/// A `multipart/form-data` body (`boundary`, bytes).
fn multipart(fields: &[(&str, String)], file: (&str, &str, &[u8])) -> (String, Vec<u8>) {
    let boundary = format!("----chatbotx{}", chatbotx_db::repo::new_id());
    let mut out = Vec::new();
    for (name, value) in fields {
        out.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    let (name, content_type, bytes) = file;
    let ext = content_type.rsplit('/').next().unwrap_or("png");
    out.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"image.{ext}\"\r\nContent-Type: {content_type}\r\n\r\n"
        )
        .as_bytes(),
    );
    out.extend_from_slice(bytes);
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (boundary, out)
}

async fn openai_image(model: &Model, req: &ImageRequest<'_>) -> Result<ImageAnswer, HeavyError> {
    let base = model.base_url.trim_end_matches('/');
    let request = match req.input {
        None => CLIENT
            .post(format!("{base}/images/generations"))
            .header("authorization", bearer(model))
            .header("content-type", "application/json")
            .body(openai_generation_body(model, req).to_string()),
        Some(input) => {
            let body = openai_generation_body(model, req);
            let fields: Vec<(&str, String)> = body
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| {
                    (
                        k.as_str(),
                        v.as_str().map_or_else(|| v.to_string(), str::to_owned),
                    )
                })
                .collect();
            let (boundary, bytes) = multipart(&fields, ("image", &image_media_type(input), input));
            CLIENT
                .post(format!("{base}/images/edits"))
                .header("authorization", bearer(model))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(bytes)
        }
    };
    let (body, _) = send(request, &model.provider).await?;
    parse_openai_image(&json_of(&body)?)
}

/// `data[0].b64_json`, else `data[0].url`.
pub fn parse_openai_image(body: &Value) -> Result<ImageAnswer, HeavyError> {
    let first = body.get("data").and_then(|d| d.get(0));
    if let Some(b64) = first
        .and_then(|f| f.get("b64_json"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        let bytes = decode_b64(b64)?;
        return Ok(ImageAnswer::Bytes(Generated {
            media_type: image_media_type(&bytes),
            bytes,
        }));
    }
    match first.and_then(|f| f.get("url")).and_then(Value::as_str) {
        Some(url) if !url.is_empty() => Ok(ImageAnswer::Url(url.to_owned())),
        _ => Err(HeavyError::transient("No image returned from provider")),
    }
}

/// The Gemini request: `gemini-*` models answer through `generateContent`
/// (`responseModalities: ["IMAGE"]`), Imagen through `:predict`.
pub fn gemini_request(
    model: &Model,
    req: &ImageRequest<'_>,
) -> Result<(String, Value), HeavyError> {
    let base = model.base_url.trim_end_matches('/');
    let id = &model.model_id;
    if id.starts_with("gemini-") {
        let mut parts = Vec::new();
        if let Some(input) = req.input {
            parts.push(json!({ "inlineData": { "mimeType": image_media_type(input), "data": STANDARD.encode(input) } }));
        }
        parts.push(json!({ "text": req.prompt }));
        let mut config = json!({ "responseModalities": ["IMAGE"] });
        if let Some(ratio) = req.aspect_ratio {
            config["imageConfig"] = json!({ "aspectRatio": ratio });
        }
        return Ok((
            format!("{base}/models/{id}:generateContent"),
            json!({ "contents": [{ "role": "user", "parts": parts }], "generationConfig": config }),
        ));
    }
    if req.input.is_some() {
        return Err(HeavyError::expected(format!(
            "Model {id} does not support image editing"
        )));
    }
    let mut parameters = json!({ "sampleCount": 1 });
    if let Some(ratio) = req.aspect_ratio {
        parameters["aspectRatio"] = json!(ratio);
    }
    Ok((
        format!("{base}/models/{id}:predict"),
        json!({ "instances": [{ "prompt": req.prompt }], "parameters": parameters }),
    ))
}

async fn gemini_image(model: &Model, req: &ImageRequest<'_>) -> Result<ImageAnswer, HeavyError> {
    let (url, body) = gemini_request(model, req)?;
    let request = CLIENT
        .post(url)
        .header("x-goog-api-key", model.api_key.as_deref().unwrap_or(""))
        .header("content-type", "application/json")
        .body(body.to_string());
    let (body, _) = send(request, "gemini").await?;
    parse_gemini_image(&json_of(&body)?).map(ImageAnswer::Bytes)
}

/// The first inline image part (`generateContent`) or prediction.
pub fn parse_gemini_image(body: &Value) -> Result<Generated, HeavyError> {
    let inline = body
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|p| p.get("inlineData").or_else(|| p.get("inline_data")));
    let (data, mime) = match inline {
        Some(inline) => (
            inline.get("data").and_then(Value::as_str),
            inline
                .get("mimeType")
                .or_else(|| inline.get("mime_type"))
                .and_then(Value::as_str),
        ),
        None => {
            let p = body.pointer("/predictions/0");
            (
                p.and_then(|p| p.get("bytesBase64Encoded"))
                    .and_then(Value::as_str),
                p.and_then(|p| p.get("mimeType")).and_then(Value::as_str),
            )
        }
    };
    let Some(data) = data.filter(|d| !d.is_empty()) else {
        return Err(HeavyError::transient("No image returned from provider"));
    };
    let bytes = decode_b64(data)?;
    Ok(Generated {
        media_type: mime.map_or_else(|| image_media_type(&bytes), str::to_owned),
        bytes,
    })
}

/// `generateSpeech({ model: openai.speech(model), text, voice,
/// instructions })`: MP3 bytes.
pub async fn generate_speech(
    model: &Model,
    text: &str,
    voice: &str,
    instructions: Option<&str>,
) -> Result<Generated, HeavyError> {
    let mut body = json!({
        "model": model.model_id,
        "input": text,
        "voice": voice,
        "response_format": "mp3",
    });
    if let Some(instructions) = instructions {
        body["instructions"] = json!(instructions);
    }
    let request = CLIENT
        .post(format!(
            "{}/audio/speech",
            model.base_url.trim_end_matches('/')
        ))
        .header("authorization", bearer(model))
        .header("content-type", "application/json")
        .body(body.to_string());
    let (bytes, _) = send(request, "openai speech").await?;
    Ok(Generated {
        bytes: bytes.to_vec(),
        media_type: "audio/mpeg".into(),
    })
}

/// One matched object; bounds are half-open source pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentationBox {
    pub object_id: u64,
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub confidence: Option<f64>,
}

/// `segmentImage`: `None` = no match.
pub async fn segment_image(
    api_key: &str,
    image: &[u8],
    mime_type: &str,
    size: (u32, u32),
    target: &str,
) -> Result<Option<Vec<SegmentationBox>>, HeavyError> {
    let body = json!({
        "model": SAM_MODEL,
        "stream": true,
        "input": [{
            "type": "message",
            "role": "user",
            "content": [
                { "type": "input_text", "text": target },
                { "type": "input_image", "image_url": format!("data:{mime_type};base64,{}", STANDARD.encode(image)) },
            ],
        }],
        "metadata": { "include_confidence": "true", "mask_encoding": "one_bit" },
    });
    let request = CLIENT
        .post(format!("{MUSE_BASE_URL}/responses"))
        .header("accept", "text/event-stream")
        .header("authorization", format!("Bearer {api_key}"))
        .header("content-type", "application/json")
        .body(body.to_string());
    let (body, content_type) = send(request, "SAM").await?;
    let content_type = content_type.unwrap_or_default();
    if !content_type.contains("text/event-stream") {
        return Err(HeavyError::expected(format!(
            "SAM response is not an event stream: {}",
            if content_type.is_empty() {
                "unknown"
            } else {
                &content_type
            }
        )));
    }
    // ponytail: the stream is read whole (bounded) instead of event by event.
    if body.len() > SAM_MAX_BODY_CHARS {
        return Err(HeavyError::expected(format!(
            "SAM output is malformed: output text exceeds {SAM_MAX_BODY_CHARS} characters"
        )));
    }
    let text = read_output_text(&String::from_utf8_lossy(&body))?;
    let boxes = parse_output_text(&text, size)?;
    Ok((!boxes.is_empty()).then_some(boxes))
}

fn stream_error(message: impl Into<String>) -> HeavyError {
    HeavyError::expected(message)
}

fn invalid_output(message: impl std::fmt::Display) -> HeavyError {
    HeavyError::expected(format!("SAM output is malformed: {message}"))
}

/// `streamFailure`: only `rate_limit_exceeded` / `server_error` retry.
fn stream_failure(source: &str, body: Option<&Value>) -> HeavyError {
    let code = body.and_then(|b| b.get("code")).and_then(Value::as_str);
    let detail = body
        .and_then(|b| b.get("message"))
        .and_then(Value::as_str)
        .map(|m| format!(": {m}"))
        .unwrap_or_default();
    HeavyError {
        message: format!("SAM stream failed: {source}{detail}"),
        retryable: matches!(code, Some("rate_limit_exceeded" | "server_error")),
        status: None,
    }
}

/// The SSE `data:` payloads of `body`, in order (`[DONE]` dropped).
fn sse_events(body: &str) -> Result<Vec<Value>, HeavyError> {
    let mut events = Vec::new();
    for block in body.replace("\r\n", "\n").split("\n\n") {
        let data: Vec<&str> = block
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .map(|d| d.strip_prefix(' ').unwrap_or(d))
            .collect();
        if data.is_empty() {
            continue;
        }
        let data = data.join("\n");
        if data == "[DONE]" {
            continue;
        }
        let event: Value = serde_json::from_str(&data)
            .map_err(|_| stream_error("SAM stream event is not valid JSON"))?;
        if event
            .get("type")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Err(stream_error("SAM stream event has no type"));
        }
        events.push(event);
    }
    Ok(events)
}

fn lane_key(event: &Value) -> Result<String, HeavyError> {
    let item = event.get("item_id").and_then(Value::as_str);
    let output = event.get("output_index").and_then(Value::as_i64);
    let content = event.get("content_index").and_then(Value::as_i64);
    match (item, output, content) {
        (Some(i), Some(o), Some(c)) => Ok(format!("{i}:{o}:{c}")),
        _ => Err(stream_error(format!(
            "SAM {} event is malformed",
            event["type"].as_str().unwrap_or("")
        ))),
    }
}

/// `readOutputText`: the finalized text of the single output-text lane.
pub fn read_output_text(body: &str) -> Result<String, HeavyError> {
    struct Lane {
        key: String,
        deltas: String,
        last: Option<String>,
    }
    let mut lane: Option<Lane> = None;
    fn open(lane: &mut Option<Lane>, key: String) -> Result<&mut Lane, HeavyError> {
        if lane.as_ref().is_some_and(|l| l.key != key) {
            return Err(stream_error(
                "SAM stream has more than one output text lane",
            ));
        }
        Ok(lane.get_or_insert(Lane {
            key,
            deltas: String::new(),
            last: None,
        }))
    }
    fn finalize(lane: &mut Option<Lane>, key: String, text: &str) -> Result<(), HeavyError> {
        let current = open(lane, key)?;
        if current.last.as_deref().is_some_and(|f| f != text) {
            return Err(stream_error(
                "SAM stream finalized its text lane twice, differently",
            ));
        }
        if !current.deltas.is_empty() && current.deltas != text {
            return Err(stream_error("SAM finalized text does not match its deltas"));
        }
        current.last = Some(text.to_owned());
        Ok(())
    }
    for event in sse_events(body)? {
        match event["type"].as_str().unwrap_or("") {
            "response.output_text.delta" => {
                let key = lane_key(&event)?;
                let delta = event.get("delta").and_then(Value::as_str).ok_or_else(|| {
                    stream_error("SAM response.output_text.delta event is malformed")
                })?;
                let current = open(&mut lane, key)?;
                if current.last.is_some() {
                    return Err(stream_error(
                        "SAM stream sent text after finalizing its lane",
                    ));
                }
                current.deltas.push_str(delta);
                if current.deltas.len() > SAM_MAX_BODY_CHARS {
                    return Err(invalid_output(format!(
                        "output text exceeds {SAM_MAX_BODY_CHARS} characters"
                    )));
                }
            }
            "response.output_text.done" => {
                let key = lane_key(&event)?;
                let text = event.get("text").and_then(Value::as_str).unwrap_or("");
                finalize(&mut lane, key, text)?;
            }
            "response.content_part.done" => {
                let key = lane_key(&event)?;
                match event.pointer("/part/type").and_then(Value::as_str) {
                    Some("refusal") => return Err(stream_error("SAM refused the request")),
                    Some("output_text") => {
                        let text = event
                            .pointer("/part/text")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        finalize(&mut lane, key, text)?;
                    }
                    _ => {}
                }
            }
            "response.refusal.delta" | "response.refusal.done" => {
                return Err(stream_error("SAM refused the request"));
            }
            "response.failed" => {
                return Err(stream_failure(
                    "response.failed",
                    event.pointer("/response/error"),
                ));
            }
            "error" => return Err(stream_failure("error event", Some(&event))),
            "response.incomplete" => {
                let reason = event
                    .pointer("/response/incomplete_details/reason")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                return Err(stream_error(format!("SAM stream ended early: {reason}")));
            }
            "response.completed" => {
                return match lane {
                    None => completed_output_text(&event),
                    Some(Lane {
                        last: Some(text), ..
                    }) => Ok(text),
                    Some(_) => Err(stream_error(
                        "SAM completed without finalizing its text lane",
                    )),
                };
            }
            _ => {}
        }
    }
    Err(stream_error("SAM stream ended before response.completed"))
}

/// `completedOutputText`.
fn completed_output_text(event: &Value) -> Result<String, HeavyError> {
    let mut texts = Vec::new();
    for item in event
        .pointer("/response/output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for part in item
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match part.get("type").and_then(Value::as_str) {
                Some("refusal") => return Err(stream_error("SAM refused the request")),
                Some("output_text") => {
                    texts.push(
                        part.get("text")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    );
                }
                _ => {}
            }
        }
    }
    match texts.len() {
        0 => Err(stream_error("SAM completed without output text")),
        1 => Ok(texts.remove(0)),
        _ => Err(stream_error(
            "SAM stream has more than one output text lane",
        )),
    }
}

fn read_integer(fields: &[(String, String)], key: &str) -> Result<u32, HeavyError> {
    fields
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or_else(|| invalid_output(format!("box field \"{key}\" is not a non-negative integer")))
}

/// `box;x1=..;y1=..;x2=..;y2=..;w=..;h=..[;c=..]`.
fn parse_box(body: &str, object_id: u64, size: (u32, u32)) -> Result<SegmentationBox, HeavyError> {
    let mut fields: Vec<(String, String)> = Vec::new();
    for field in body.split(';').skip(1) {
        let Some(sep) = field.find('=').filter(|i| *i > 0) else {
            return Err(invalid_output(format!("box field \"{field}\" has no key")));
        };
        let key = &field[..sep];
        if fields.iter().any(|(k, _)| k == key) {
            return Err(invalid_output(format!("box repeats field \"{key}\"")));
        }
        fields.push((key.to_owned(), field[sep + 1..].to_owned()));
    }
    let (w, h) = (read_integer(&fields, "w")?, read_integer(&fields, "h")?);
    if (w, h) != size {
        return Err(invalid_output(format!(
            "frame {w}x{h} does not match the source image {}x{}",
            size.0, size.1
        )));
    }
    let x1 = read_integer(&fields, "x1")?;
    let y1 = read_integer(&fields, "y1")?;
    let x2 = read_integer(&fields, "x2")?;
    let y2 = read_integer(&fields, "y2")?;
    if x1 > x2 || y1 > y2 || x2 >= w || y2 >= h {
        return Err(invalid_output(format!(
            "box {x1},{y1},{x2},{y2} is outside its frame"
        )));
    }
    let confidence = fields
        .iter()
        .find(|(k, _)| k == "c")
        .and_then(|(_, v)| v.parse::<f64>().ok())
        .filter(|c| (0.0..=1.0).contains(c));
    Ok(SegmentationBox {
        object_id,
        left: x1,
        top: y1,
        right: x2 + 1,
        bottom: y2 + 1,
        confidence,
    })
}

/// After `<0f>`: comma-separated `<id><|box;…|>[<|mask;…|>…]` objects.
fn parse_frame_objects(line: &str, size: (u32, u32)) -> Result<Vec<SegmentationBox>, HeavyError> {
    let mut boxes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut pos = 0;
    let bytes = line.as_bytes();
    while pos < line.len() {
        let start = pos;
        while pos < line.len() && bytes[pos].is_ascii_digit() {
            pos += 1;
        }
        let Some(object_id) = line[start..pos].parse::<u64>().ok() else {
            return Err(invalid_output(format!(
                "expected an object id at column {start}"
            )));
        };
        if !seen.insert(object_id) {
            return Err(invalid_output(format!(
                "object {object_id} appears twice in one frame"
            )));
        }
        let mut found = None;
        while line[pos..].starts_with("<|") {
            let Some(end) = line[pos + 2..].find("|>").map(|i| i + pos + 2) else {
                return Err(invalid_output(format!(
                    "unterminated token at column {pos}"
                )));
            };
            let body = &line[pos + 2..end];
            pos = end + 2;
            if body == "box" || body.starts_with("box;") {
                if found.is_some() {
                    return Err(invalid_output(format!("object {object_id} has two boxes")));
                }
                found = Some(parse_box(body, object_id, size)?);
            }
        }
        let Some(found) = found else {
            return Err(invalid_output(format!("object {object_id} has no box")));
        };
        boxes.push(found);
        if pos < line.len() {
            if bytes[pos] != b',' {
                return Err(invalid_output(format!("unexpected text at column {pos}")));
            }
            pos += 1;
            if pos == line.len() {
                return Err(invalid_output("frame ends with a separator"));
            }
        }
    }
    Ok(boxes)
}

/// `parseOutputText`: one frame (`<0f>`); prose lines are no match; a later
/// line for the same object replaces the earlier one.
pub fn parse_output_text(text: &str, size: (u32, u32)) -> Result<Vec<SegmentationBox>, HeavyError> {
    let mut by_object: Vec<SegmentationBox> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let frame = line
            .strip_prefix('<')
            .and_then(|rest| rest.find("f>").map(|i| &rest[..i]))
            .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()));
        let Some(frame) = frame else {
            if line.starts_with('<') || line.contains("<|") {
                return Err(invalid_output("line does not start with a frame marker"));
            }
            continue;
        };
        if frame != "0" {
            return Err(invalid_output(format!("image output has frame {frame}")));
        }
        for found in parse_frame_objects(&line[frame.len() + 3..], size)? {
            match by_object
                .iter_mut()
                .find(|b| b.object_id == found.object_id)
            {
                Some(slot) => *slot = found,
                None => by_object.push(found),
            }
        }
    }
    Ok(by_object)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::llm::{Transform, Wire};

    fn model(provider: &str, id: &str) -> Model {
        Model {
            provider: provider.into(),
            wire: Wire::OpenAi,
            base_url: "https://p.test/v1".into(),
            api_key: Some("k".into()),
            model_id: id.into(),
            transform: Transform::None,
        }
    }

    #[test]
    fn sam_boxes_parse_and_reject_bad_frames() {
        let text = "Found:\n<0f>1<|box;x1=0;y1=0;x2=3;y2=1;w=10;h=5;c=0.9|><|mask;xx|>,2<|box;x1=4;y1=1;x2=9;y2=4;w=10;h=5|>\n";
        let boxes = parse_output_text(text, (10, 5)).unwrap();
        assert_eq!(boxes.len(), 2);
        assert_eq!(
            (boxes[0].right, boxes[0].bottom, boxes[0].confidence),
            (4, 2, Some(0.9))
        );
        assert_eq!(
            (boxes[1].left, boxes[1].right, boxes[1].confidence),
            (4, 10, None)
        );
        assert!(
            parse_output_text("no objects here", (10, 5))
                .unwrap()
                .is_empty()
        );
        assert!(parse_output_text("<0f>1<|box;x1=0;y1=0;x2=3;y2=1;w=9;h=5|>", (10, 5)).is_err());
        assert!(parse_output_text("<1f>1<|box;x1=0;y1=0;x2=3;y2=1;w=10;h=5|>", (10, 5)).is_err());
        assert!(parse_output_text("<0f>1<|box;x1=0;y1=0;x2=10;y2=1;w=10;h=5|>", (10, 5)).is_err());
    }

    #[test]
    fn sam_stream_reads_one_lane() {
        let ev = |v: Value| format!("data: {v}\n\n");
        let lane = json!({ "item_id": "i", "output_index": 0, "content_index": 0 });
        let mut delta = lane.clone();
        delta["type"] = json!("response.output_text.delta");
        delta["delta"] = json!("<0f>");
        let mut done = lane.clone();
        done["type"] = json!("response.content_part.done");
        done["part"] = json!({ "type": "output_text", "text": "<0f>" });
        let body = [
            ev(delta),
            ev(done),
            ev(json!({ "type": "response.completed" })),
            "data: [DONE]\n\n".into(),
        ]
        .concat();
        assert_eq!(read_output_text(&body).unwrap(), "<0f>");
        let completed = ev(
            json!({ "type": "response.completed", "response": { "output": [{ "content": [{ "type": "output_text", "text": "none" }] }] } }),
        );
        assert_eq!(read_output_text(&completed).unwrap(), "none");
        let failed = ev(json!({ "type": "error", "code": "server_error", "message": "busy" }));
        let err = read_output_text(&failed).unwrap_err();
        assert!(err.retryable && err.message.contains("busy"));
        assert!(!read_output_text("").unwrap_err().retryable);
    }

    #[test]
    fn image_requests_follow_the_provider_wire() {
        let req = ImageRequest {
            prompt: "a cat",
            size: Some("1024x1024"),
            quality: Some("high"),
            ..ImageRequest::default()
        };
        let body = openai_generation_body(&model("openai", "gpt-image-1"), &req);
        assert_eq!(
            body,
            json!({ "model": "gpt-image-1", "prompt": "a cat", "n": 1, "size": "1024x1024", "quality": "high" })
        );
        let muse = openai_generation_body(&model("muse", "muse-image-1.0"), &req);
        assert_eq!(muse["response_format"], "b64_json");

        let gem = ImageRequest {
            prompt: "a cat",
            aspect_ratio: Some("16:9"),
            input: Some(b"\x89PNG\r\n\x1a\nxxxx"),
            ..ImageRequest::default()
        };
        let (url, body) = gemini_request(&model("gemini", "gemini-3.1-flash-image"), &gem).unwrap();
        assert!(url.ends_with("/models/gemini-3.1-flash-image:generateContent"));
        assert_eq!(
            body["generationConfig"]["imageConfig"]["aspectRatio"],
            "16:9"
        );
        assert_eq!(
            body["contents"][0]["parts"][0]["inlineData"]["mimeType"],
            "image/png"
        );
        assert!(gemini_request(&model("gemini", "imagen-4"), &gem).is_err());

        let png = STANDARD.encode(b"\x89PNG\r\n\x1a\nxxxx");
        let Ok(ImageAnswer::Bytes(image)) =
            parse_openai_image(&json!({ "data": [{ "b64_json": png }] }))
        else {
            panic!("expected bytes");
        };
        assert_eq!(image.media_type, "image/png");
        assert!(matches!(
            parse_openai_image(&json!({ "data": [{ "url": "https://x.test/a.png" }] })),
            Ok(ImageAnswer::Url(_))
        ));
        let inline = json!({ "candidates": [{ "content": { "parts": [{ "text": "hi" }, { "inlineData": { "mimeType": "image/jpeg", "data": png } }] } }] });
        assert_eq!(
            parse_gemini_image(&inline).unwrap().media_type,
            "image/jpeg"
        );
        assert!(parse_gemini_image(&json!({})).unwrap_err().retryable);
        assert!(HeavyError::http(429, "x").retryable && !HeavyError::http(400, "x").retryable);
    }
}
