//! `packages/business/src/get-user-data` (`reply-format-validators.ts`,
//! `reply-input.combinators.ts`, `reply-input.types.ts`): whether a contact
//! message answers a Get User Data step of a reply format, and the value it
//! gives.

use std::sync::LazyLock;

use chatbotx_core::js;
use regex::Regex;
use serde_json::Value;

/// `ReplyInputAttachment`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReplyInputAttachment {
    pub file_type: String,
    /// Present on stored rows (`id`, `messageCreatedAt`).
    pub id: Option<i64>,
    pub message_created_at: Option<i64>,
    pub origin_path: String,
}

/// `ReplyInputMessage`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplyInputMessage {
    pub attachments: Vec<ReplyInputAttachment>,
    pub content_attributes: Option<Value>,
    pub content_type: String,
    pub text: Option<String>,
}

impl Default for ReplyInputMessage {
    fn default() -> ReplyInputMessage {
        ReplyInputMessage {
            attachments: Vec::new(),
            content_attributes: None,
            content_type: "text".into(),
            text: None,
        }
    }
}

/// `ReplyInputKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyInputKind {
    Text,
    Attachment,
    Location,
}

/// `ReplyAttachmentRef`: `(id, messageCreatedAt)`.
pub type ReplyAttachmentRef = (i64, i64);

/// `ReplyValidationResult`.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplyValidation {
    Ok {
        user_input: String,
        kind: ReplyInputKind,
        attachment: Option<ReplyAttachmentRef>,
    },
    Rejected(String),
}

/// `ReplyValidator`.
pub type ReplyValidator = Box<dyn Fn(&ReplyInputMessage) -> ReplyValidation + Send + Sync>;
type TextCheck = fn(&str) -> ReplyValidation;

/// `accepted`.
pub fn accepted(user_input: impl Into<String>, kind: ReplyInputKind) -> ReplyValidation {
    ReplyValidation::Ok {
        user_input: user_input.into(),
        kind,
        attachment: None,
    }
}

/// `rejected`.
pub fn rejected(reason: &str) -> ReplyValidation {
    ReplyValidation::Rejected(reason.to_owned())
}

fn text(check: TextCheck, message: &ReplyInputMessage) -> ReplyValidation {
    match message.text.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => check(t),
        None => rejected("getUserData: expected text input"),
    }
}

/// `fromText`.
pub fn from_text(check: TextCheck) -> ReplyValidator {
    Box::new(move |m| text(check, m))
}

/// `fromTextWhenAttachmentAbsent`.
pub fn from_text_when_attachment_absent(check: TextCheck) -> ReplyValidator {
    Box::new(move |m| {
        if m.attachments.is_empty() {
            text(check, m)
        } else {
            rejected("getUserData: unsupported file type")
        }
    })
}

/// `fromAttachment`.
pub fn from_attachment(accept: fn(&str) -> bool) -> ReplyValidator {
    Box::new(move |m| {
        let Some(file) = m.attachments.first() else {
            return rejected("getUserData: expected a file attachment");
        };
        if !accept(&file.file_type) {
            return rejected("getUserData: unsupported file type");
        }
        ReplyValidation::Ok {
            user_input: file.origin_path.clone(),
            kind: ReplyInputKind::Attachment,
            attachment: file.id.zip(file.message_created_at),
        }
    })
}

/// `Number(value)` of a JSON attribute.
fn number_of(value: Option<&Value>) -> f64 {
    match value {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => js::to_number(s),
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        _ => f64::NAN,
    }
}

/// `fromLocation`.
pub fn from_location(message: &ReplyInputMessage) -> ReplyValidation {
    if message.content_type != "location" {
        return rejected("getUserData: expected a location");
    }
    let attrs = message.content_attributes.as_ref();
    // `attrs.latitude ?? attrs.lat`.
    let pick = |a: &str, b: &str| {
        attrs
            .and_then(|x| x.get(a).filter(|v| !v.is_null()))
            .or_else(|| attrs.and_then(|x| x.get(b)))
    };
    let (latitude, longitude) = (
        number_of(pick("latitude", "lat")),
        number_of(pick("longitude", "long")),
    );
    if !(latitude.is_finite() && longitude.is_finite()) {
        return rejected("getUserData: invalid location");
    }
    accepted(
        format!(
            "{},{}",
            js::number_to_string(latitude),
            js::number_to_string(longitude)
        ),
        ReplyInputKind::Location,
    )
}

/// `firstAccepted`.
pub fn first_accepted(validators: Vec<ReplyValidator>) -> ReplyValidator {
    Box::new(move |m| {
        let mut last = None;
        for validate in &validators {
            match validate(m) {
                ok @ ReplyValidation::Ok { .. } => return ok,
                rejection => last = Some(rejection),
            }
        }
        last.unwrap_or_else(|| rejected("getUserData: no supported input received"))
    })
}

/// `EMAIL_PATTERN` / the questionnaire `emailPattern`.
pub static EMAIL_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$").expect("valid regex"));
/// `PHONE_PATTERN` / the questionnaire `phoneRegex` (ASCII digits).
pub static PHONE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\+?([0-9][0-9\-. ]+)?(\([0-9\-. ]+\))?[0-9\-. ]+[0-9]$").expect("valid regex")
});

/// `Number.parseFloat(text)` is not `NaN`: a decimal (or `Infinity`) prefix
/// after leading whitespace.
fn parses_as_float(text: &str) -> bool {
    static RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[+-]?(Infinity|[0-9]+(\.[0-9]*)?|\.[0-9]+)").expect("valid regex")
    });
    RE.is_match(js::trim_start(text))
}

fn as_number(text: &str) -> ReplyValidation {
    if parses_as_float(text) {
        accepted(text, ReplyInputKind::Text)
    } else {
        rejected("getUserData: invalid number")
    }
}

fn as_email(text: &str) -> ReplyValidation {
    if EMAIL_PATTERN.is_match(text) {
        accepted(text, ReplyInputKind::Text)
    } else {
        rejected("getUserData: invalid email address")
    }
}

fn as_phone(text: &str) -> ReplyValidation {
    if PHONE_PATTERN.is_match(text) {
        accepted(text, ReplyInputKind::Text)
    } else {
        rejected("getUserData: invalid phone number")
    }
}

fn as_url(text: &str) -> ReplyValidation {
    if url::Url::parse(text).is_ok() {
        accepted(text, ReplyInputKind::Text)
    } else {
        rejected("getUserData: invalid link")
    }
}

fn as_iso_date(text: &str) -> ReplyValidation {
    match js::parse_date(text).and_then(js::to_iso_string) {
        Some(iso) => accepted(iso, ReplyInputKind::Text),
        None => rejected("getUserData: invalid date"),
    }
}

fn as_is(text: &str) -> ReplyValidation {
    accepted(text, ReplyInputKind::Text)
}

fn as_coordinate_pair(text: &str) -> ReplyValidation {
    use chatbotx_core::utils::coordinates::{COORDINATE_PAIR_PATTERN, parse_coordinate_pair};
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(COORDINATE_PAIR_PATTERN).expect("valid regex"));
    if !RE.is_match(text) {
        return rejected("getUserData: expected a location");
    }
    match parse_coordinate_pair(text) {
        Some(pair) => accepted(
            format!(
                "{},{}",
                js::number_to_string(pair.latitude),
                js::number_to_string(pair.longitude)
            ),
            ReplyInputKind::Location,
        ),
        None => rejected("getUserData: invalid location"),
    }
}

/// The reply format codes (`ReplyFormat`).
pub const REPLY_FORMATS: &[&str] = &[
    "RF01", "RF02", "RF03", "RF04", "RF05", "RF06", "RF07", "RF08", "RF09", "RF10", "RF11",
];

/// `replyFormatValidators[replyFormat]`; an unknown code is free text.
pub fn reply_format_validator(reply_format: &str) -> ReplyValidator {
    match reply_format {
        "RF01" => from_text_when_attachment_absent(as_number),
        "RF03" => from_text_when_attachment_absent(as_email),
        "RF04" => from_text_when_attachment_absent(as_phone),
        "RF05" => first_accepted(vec![
            from_attachment(|t| t == "image"),
            from_text_when_attachment_absent(as_is),
        ]),
        "RF06" => first_accepted(vec![
            from_attachment(|_| true),
            from_text_when_attachment_absent(as_is),
        ]),
        "RF07" => from_text_when_attachment_absent(as_url),
        // A real pin first; typed "lat,lng" is the omnichannel fallback.
        "RF08" => Box::new(|m| {
            if m.content_type == "location" {
                from_location(m)
            } else {
                text_when_absent(as_coordinate_pair, m)
            }
        }),
        "RF09" | "RF10" => from_text_when_attachment_absent(as_iso_date),
        "RF11" => first_accepted(vec![
            from_attachment(|_| true),
            Box::new(from_location),
            from_text(as_is),
        ]),
        _ => from_text_when_attachment_absent(as_is),
    }
}

fn text_when_absent(check: TextCheck, m: &ReplyInputMessage) -> ReplyValidation {
    from_text_when_attachment_absent(check)(m)
}

/// `validateReplyInput`.
pub fn validate_reply_input(reply_format: &str, message: &ReplyInputMessage) -> ReplyValidation {
    reply_format_validator(reply_format)(message)
}
