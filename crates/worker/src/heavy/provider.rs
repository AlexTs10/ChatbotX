//! The provider calls of the heavy steps (the AI SDK `generateImage` /
//! `generateSpeech` / `transcribe` wire formats, `packages/ai/src/server/sam.ts`
//! and `muse-transcribe.ts`).
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

/// A `multipart/form-data` body (`boundary`, bytes); a file part is
/// `(name, filename, content type, bytes)`.
fn multipart(fields: &[(&str, String)], files: &[(&str, &str, &str, &[u8])]) -> (String, Vec<u8>) {
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
    for (name, filename, content_type, bytes) in files {
        out.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        out.extend_from_slice(bytes);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
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
            let content_type = image_media_type(input);
            let filename = format!("image.{}", content_type.rsplit('/').next().unwrap_or("png"));
            let (boundary, bytes) =
                multipart(&fields, &[("image", &filename, &content_type, input)]);
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

/// `transcribe({ model: openai.transcription(model), audio })`: the
/// transcript text. `media_type` names the upload's format.
pub async fn transcribe_openai(
    model: &Model,
    audio: &[u8],
    media_type: &str,
) -> Result<String, HeavyError> {
    let filename = format!("audio.{}", audio_extension(media_type));
    let fields = [
        ("model", model.model_id.clone()),
        ("response_format", "json".to_owned()),
    ];
    let (boundary, bytes) = multipart(&fields, &[("file", &filename, media_type, audio)]);
    let request = CLIENT
        .post(format!(
            "{}/audio/transcriptions",
            model.base_url.trim_end_matches('/')
        ))
        .header("authorization", bearer(model))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(bytes);
    let (body, _) = send(request, "openai transcription").await?;
    json_of(&body)?["text"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| HeavyError::transient("Invalid transcription response: no text"))
}

/// The upload file extension of a supported audio MIME type.
fn audio_extension(media_type: &str) -> &'static str {
    match media_type {
        "audio/flac" => "flac",
        "audio/m4a" | "audio/x-m4a" => "m4a",
        "audio/mp4" => "mp4",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/webm" => "webm",
        "audio/ogg" => "ogg",
        _ => "mp3",
    }
}

/// `MUSE_TRANSCRIBE_MAX_SECONDS`.
pub const MUSE_TRANSCRIBE_MAX_SECONDS: f64 = 600.0;
/// `MUSE_TRANSCRIBE_MAX_REQUEST_BYTES`.
pub const MUSE_TRANSCRIBE_MAX_REQUEST_BYTES: u64 = 32 * 1024 * 1024;
/// `MUSE_TRANSCRIBE_MULTIPART_OVERHEAD_BYTES`.
pub const MUSE_TRANSCRIBE_MULTIPART_OVERHEAD_BYTES: u64 = 64 * 1024;
const WAV_MAX_CHUNKS: usize = 64;
const WAV_UNKNOWN_SIZE: u32 = 0xffff_ffff;

/// `MuseWavFormatError`: `(reason, message)`.
#[derive(Debug, Clone, PartialEq)]
pub struct WavFormatError(pub &'static str, pub String);

/// `MuseWavInfo`.
#[derive(Debug, Clone, PartialEq)]
pub struct WavInfo {
    pub sample_rate: u32,
    pub data_bytes: u64,
    pub duration_seconds: f64,
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// `inspectMuseWav`: the RIFF header and chunk table only (at most 64
/// chunks). Muse takes WAV PCM 16-bit mono 16 / 24 kHz, at most 10 min.
pub fn inspect_muse_wav(bytes: &[u8]) -> Result<WavInfo, WavFormatError> {
    let err = |reason, message: String| Err(WavFormatError(reason, message));
    let length = bytes.len();
    if length < 4 || &bytes[0..4] != b"RIFF" {
        return err("notWav", "File is not a RIFF/WAV file".into());
    }
    if length < 12 {
        return err("malformed", "WAV header is truncated".into());
    }
    if &bytes[8..12] != b"WAVE" {
        return err("notWav", "RIFF file is not WAVE audio".into());
    }
    let riff_size = le32(bytes, 4);
    let streaming = riff_size == WAV_UNKNOWN_SIZE || riff_size == 0;
    let container_end = if streaming {
        length as u64
    } else {
        8 + u64::from(riff_size)
    };
    if container_end > length as u64 {
        return err(
            "malformed",
            format!("WAV declares {container_end} bytes but the file has {length}"),
        );
    }
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut data_bytes: Option<u64> = None;
    let mut offset: u64 = 12;
    let mut chunks = 0;
    while offset + 8 <= container_end && !(fmt.is_some() && data_bytes.is_some()) {
        chunks += 1;
        if chunks > WAV_MAX_CHUNKS {
            return err(
                "malformed",
                format!("WAV has more than {WAV_MAX_CHUNKS} chunks"),
            );
        }
        let at = offset as usize;
        let id = String::from_utf8_lossy(&bytes[at..at + 4]).into_owned();
        let size = le32(bytes, at + 4);
        let body = offset + 8;
        if id == "data" && size == WAV_UNKNOWN_SIZE {
            data_bytes = Some(container_end - body);
            break;
        }
        let padded_end = body + u64::from(size) + u64::from(size % 2);
        if padded_end > container_end {
            return err(
                "malformed",
                format!("WAV chunk \"{id}\" declares {size} bytes past the end of the file"),
            );
        }
        if id == "fmt " {
            if size < 16 {
                return err("malformed", "WAV fmt chunk is too short".into());
            }
            let b = body as usize;
            fmt = Some((
                le16(bytes, b),
                le16(bytes, b + 2),
                le32(bytes, b + 4),
                le16(bytes, b + 14),
            ));
        } else if id == "data" {
            data_bytes = Some(u64::from(size));
        }
        offset = padded_end;
    }
    let Some((format, channels, sample_rate, bits)) = fmt else {
        return err("malformed", "WAV has no fmt chunk".into());
    };
    if format != 1 {
        return err(
            "notPcm",
            format!("WAV audio format is {format}; Muse requires PCM (1)"),
        );
    }
    if channels != 1 {
        return err(
            "notMono",
            format!("WAV has {channels} channels; Muse requires mono"),
        );
    }
    if bits != 16 {
        return err(
            "not16Bit",
            format!("WAV is {bits}-bit; Muse requires 16-bit"),
        );
    }
    if !matches!(sample_rate, 16_000 | 24_000) {
        return err(
            "unsupportedSampleRate",
            format!("WAV sample rate is {sample_rate} Hz; Muse requires 16000 or 24000 Hz"),
        );
    }
    let data_bytes = match data_bytes {
        Some(n) if n > 0 => n,
        _ => return err("noAudioData", "WAV has no audio data".into()),
    };
    let duration_seconds = data_bytes as f64 / (f64::from(sample_rate) * 2.0);
    if duration_seconds > MUSE_TRANSCRIBE_MAX_SECONDS {
        return err(
            "tooLong",
            format!(
                "WAV is {} s long; Muse allows at most {MUSE_TRANSCRIBE_MAX_SECONDS} s",
                duration_seconds.ceil()
            ),
        );
    }
    Ok(WavInfo {
        sample_rate,
        data_bytes,
        duration_seconds,
    })
}

/// `describeMuseErrorBody`.
pub fn describe_muse_error_body(body: &str) -> Option<String> {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let detail = parsed.as_ref().and_then(|v| {
        let detail = match v.get("error") {
            Some(error) if error.is_object() => error,
            _ => v,
        };
        detail.is_object().then_some(detail)
    });
    let Some(detail) = detail else {
        let trimmed = body.trim();
        return (!trimmed.is_empty()).then(|| trimmed.chars().take(300).collect());
    };
    let text = |key: &str| {
        detail
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    let mut parts: Vec<String> = Vec::new();
    if let Some(message) = text("message") {
        parts.push(message.to_owned());
    }
    if let Some(kind) = text("type") {
        parts.push(format!("type={kind}"));
    }
    match detail.get("code") {
        Some(Value::String(code)) => parts.push(format!("code={code}")),
        Some(Value::Number(code)) => parts.push(format!("code={code}")),
        _ => {}
    }
    if let Some(param) = text("param") {
        parts.push(format!("param={param}"));
    }
    (!parts.is_empty()).then(|| parts.join(", ").chars().take(300).collect())
}

/// `isRetryableMuseStatus`.
pub fn is_retryable_muse_status(status: u16) -> bool {
    !matches!(status, 400 | 401 | 403 | 406 | 413)
        && (status == 408 || status == 429 || status >= 500)
}

/// `TranscribeWithMuseResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct MuseTranscript {
    pub text: String,
    pub session_id: String,
    pub audio_duration_ms: Option<f64>,
}

/// `transcribeWithMuse` (`POST /v1/asr/transcribe`): one attempt, the job
/// owns the retries.
pub async fn transcribe_with_muse(
    api_key: &str,
    audio: &[u8],
    model: &str,
    language_bias: &[Value],
    keywords: &[Value],
    session_id: &str,
    timeout: Duration,
) -> Result<MuseTranscript, HeavyError> {
    let request_bytes = audio.len() as u64 + MUSE_TRANSCRIBE_MULTIPART_OVERHEAD_BYTES;
    if request_bytes > MUSE_TRANSCRIBE_MAX_REQUEST_BYTES {
        return Err(HeavyError::expected(format!(
            "Audio is {} bytes; Muse requests must stay under {MUSE_TRANSCRIBE_MAX_REQUEST_BYTES} bytes",
            audio.len()
        )));
    }
    let mut body = json!({ "model": model, "audioEncoding": "WAV", "mode": "PUSH_TO_TALK" });
    if !language_bias.is_empty() {
        body["languageBias"] = json!(language_bias);
    }
    if !keywords.is_empty() {
        body["keywords"] = json!(keywords);
    }
    let request_json = body.to_string();
    let (boundary, bytes) = multipart(
        &[],
        &[
            (
                "request",
                "blob",
                "application/json",
                request_json.as_bytes(),
            ),
            ("audio", "audio.wav", "audio/wav", audio),
        ],
    );
    let mut url = url::Url::parse(&format!("{MUSE_BASE_URL}/asr/transcribe"))
        .map_err(|e| HeavyError::transient(e.to_string()))?;
    url.query_pairs_mut().append_pair("sessionId", session_id);
    let response = CLIENT
        .post(url)
        .header("accept", "application/json")
        .header("authorization", format!("Bearer {api_key}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .timeout(timeout)
        .body(bytes)
        .send()
        .await
        .map_err(|_| HeavyError::transient("Muse transcription request failed"))?;
    let status = response.status().as_u16();
    let body = response
        .bytes()
        .await
        .map_err(|_| HeavyError::transient("Muse transcription request failed"))?;
    if !(200..300).contains(&status) {
        let detail = describe_muse_error_body(&String::from_utf8_lossy(&body));
        let message = format!(
            "Muse transcription failed with status {status}{}",
            detail.map(|d| format!(": {d}")).unwrap_or_default()
        );
        return Err(HeavyError {
            message,
            retryable: is_retryable_muse_status(status),
            status: Some(status),
        });
    }
    let invalid = || HeavyError::expected("Muse transcription response is invalid");
    let parsed: Value = serde_json::from_slice(&body).map_err(|_| invalid())?;
    let text = parsed
        .get("transcript")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    Ok(MuseTranscript {
        text: text.to_owned(),
        session_id: parsed
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or(session_id)
            .to_owned(),
        audio_duration_ms: parsed
            .get("audioDurationMs")
            .and_then(Value::as_f64)
            .filter(|ms| *ms >= 0.0),
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

    fn wav(format: u16, channels: u16, rate: u32, bits: u16, data: u32) -> Vec<u8> {
        let mut b = b"RIFF".to_vec();
        b.extend_from_slice(&(36 + data).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&format.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&bits.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data.to_le_bytes());
        b.extend(std::iter::repeat_n(0u8, data as usize));
        b
    }

    #[test]
    fn muse_wav_check_follows_the_original() {
        let ok = inspect_muse_wav(&wav(1, 1, 16_000, 16, 32_000)).unwrap();
        assert_eq!((ok.sample_rate, ok.data_bytes), (16_000, 32_000));
        assert!((ok.duration_seconds - 1.0).abs() < 1e-9);
        let reason = |b: Vec<u8>| inspect_muse_wav(&b).unwrap_err().0;
        assert_eq!(reason(b"OggS....".to_vec()), "notWav");
        assert_eq!(reason(wav(3, 1, 16_000, 16, 4)), "notPcm");
        assert_eq!(reason(wav(1, 2, 16_000, 16, 4)), "notMono");
        assert_eq!(reason(wav(1, 1, 16_000, 24, 4)), "not16Bit");
        assert_eq!(reason(wav(1, 1, 44_100, 16, 4)), "unsupportedSampleRate");
        assert_eq!(reason(wav(1, 1, 16_000, 16, 0)), "noAudioData");
        let mut truncated = wav(1, 1, 16_000, 16, 8);
        truncated.truncate(30);
        assert_eq!(reason(truncated), "malformed");
        // A streaming header (unknown sizes) reads the rest of the file.
        let mut streaming = wav(1, 1, 24_000, 16, 48_000);
        streaming[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        streaming[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(inspect_muse_wav(&streaming).unwrap().data_bytes, 48_000);
        // 601 s at 16 kHz is too long.
        let mut long = wav(1, 1, 16_000, 16, 0);
        long[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        long[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        long.extend(std::iter::repeat_n(0u8, 601 * 32_000));
        assert_eq!(reason(long), "tooLong");
    }

    #[test]
    fn muse_errors_follow_the_original() {
        assert_eq!(
            describe_muse_error_body(
                r#"{"error":{"message":"bad audio","type":"invalid","code":400}}"#
            )
            .as_deref(),
            Some("bad audio, type=invalid, code=400")
        );
        assert_eq!(
            describe_muse_error_body(r#"{"message":"slow down"}"#).as_deref(),
            Some("slow down")
        );
        assert_eq!(
            describe_muse_error_body("  gateway  ").as_deref(),
            Some("gateway")
        );
        assert_eq!(describe_muse_error_body(""), None);
        assert!(is_retryable_muse_status(429) && is_retryable_muse_status(503));
        assert!(!is_retryable_muse_status(413) && !is_retryable_muse_status(404));
        assert_eq!(audio_extension("audio/x-m4a"), "m4a");
        assert_eq!(audio_extension("audio/mpga"), "mp3");
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
