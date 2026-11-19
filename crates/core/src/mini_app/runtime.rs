//! `custom-fields.ts`, `answers.ts`, `uploads.ts`: what the server does with
//! a definition at submit and upload time.

use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Map, Value};

use super::{Definition, Node, collect_input_names, input_name, node_name, walk, walk_mut};

pub const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;
pub const MAX_UPLOADS_PER_INPUT: usize = 30;
pub const WHATSAPP_MAX_FILES_PER_RESPONSE: usize = 10;
pub const UPLOAD_SNIFF_BYTES: usize = 4096;
pub const MAX_ANSWER_TEXT_LENGTH: usize = 4096;
pub const MAX_ANSWERS_BYTES: usize = 64 * 1024;
const MAX_SELECTED_IDS: usize = 200;
const MAX_SELECTED_ID_LENGTH: usize = 200;
const DATE_LENGTH: usize = 10;

// ------------------------------------------------------ custom fields

/// `collectCustomFieldMappings`: input name → custom field id.
pub fn collect_custom_field_mappings(definition: &Definition) -> BTreeMap<String, String> {
    let mut mappings = BTreeMap::new();
    for screen in &definition.screens {
        walk(&screen.children, &mut |v| {
            if let (Some(name), Some(id)) = (input_name(v.node), v.node.custom_field_id.as_ref()) {
                mappings.insert(name.to_owned(), id.clone());
            }
        });
    }
    mappings
}

/// `applyCustomFieldMappings`: `None` (or "") removes a mapping; names not
/// listed keep theirs.
pub fn apply_custom_field_mappings(
    definition: &mut Definition,
    mappings: &BTreeMap<String, Option<String>>,
) {
    for screen in &mut definition.screens {
        walk_mut(&mut screen.children, &mut |node: &mut Node| {
            let Some(name) = input_name(node).map(str::to_owned) else {
                return;
            };
            if let Some(id) = mappings.get(&name) {
                node.custom_field_id = id.clone().filter(|id| !id.is_empty());
            }
        });
    }
}

/// `formatAnswerForCustomField`.
pub fn format_answer_for_custom_field(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) if s.is_empty() => None,
        v if is_file_answer(v) => Some(
            v.as_array()
                .into_iter()
                .flatten()
                .map(|f| crate::js::string_of(f.get("url").unwrap_or(&Value::Null)))
                .collect::<Vec<_>>()
                .join(", "),
        ),
        Value::Array(items) if items.is_empty() => None,
        Value::Array(items) => Some(
            items
                .iter()
                .map(crate::js::string_of)
                .collect::<Vec<_>>()
                .join(", "),
        ),
        Value::Object(_) => crate::utils::form_answer_format::format_date_range(value),
        other => Some(crate::js::string_of(other)),
    }
}

// ------------------------------------------------------------ answers

/// `isFileAnswer`.
pub fn is_file_answer(value: &Value) -> bool {
    value.as_array().is_some_and(|items| {
        !items.is_empty()
            && items.iter().all(|i| {
                i.as_object()
                    .is_some_and(|o| o.contains_key("url") && o.contains_key("uploadId"))
            })
    })
}

fn short(text: &str, max: usize) -> bool {
    crate::js::utf16_len(text) <= max
}

fn answer_value(value: &Value) -> Option<Value> {
    match value {
        Value::String(s) if short(s, MAX_ANSWER_TEXT_LENGTH) => Some(value.clone()),
        Value::Number(_) | Value::Bool(_) => Some(value.clone()),
        Value::Array(items)
            if items.len() <= MAX_SELECTED_IDS
                && items
                    .iter()
                    .all(|i| i.as_str().is_some_and(|s| short(s, MAX_SELECTED_ID_LENGTH))) =>
        {
            Some(value.clone())
        }
        Value::Object(o) => {
            let date = |key: &str| {
                o.get(key)
                    .and_then(Value::as_str)
                    .filter(|s| short(s, DATE_LENGTH))
            };
            let (start, end) = (date("start-date")?, date("end-date")?);
            Some(serde_json::json!({ "start-date": start, "end-date": end }))
        }
        _ => None,
    }
}

fn upload_ids(value: &Value) -> Option<Value> {
    let items = value.as_array()?;
    let ok = (1..=MAX_UPLOADS_PER_INPUT).contains(&items.len())
        && items.iter().all(|i| i.as_str().is_some_and(is_upload_id));
    ok.then(|| value.clone())
}

/// `sanitizeMiniAppAnswers`: the known answers that pass their shape;
/// `None` when the input is not an object or too large.
pub fn sanitize_answers(definition: &Definition, input: &Value) -> Option<Map<String, Value>> {
    let object = input.as_object()?;
    if crate::js::utf16_len(&crate::js::json_stringify(input)) > MAX_ANSWERS_BYTES {
        return None;
    }
    let names: HashSet<String> = definition
        .screens
        .iter()
        .flat_map(collect_input_names)
        .collect();
    let file_names = collect_file_input_names(definition);
    let mut answers = Map::new();
    for (name, value) in object {
        if !names.contains(name) {
            continue;
        }
        let parsed = if file_names.contains(name) {
            upload_ids(value)
        } else {
            answer_value(value)
        };
        if let Some(parsed) = parsed {
            answers.insert(name.clone(), parsed);
        }
    }
    Some(answers)
}

// ------------------------------------------------------------ uploads

/// `MINI_APP_UPLOAD_ID_PATTERN`.
pub fn is_upload_id(text: &str) -> bool {
    text.len() == 22
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub const PHOTO_MIME_TYPES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/webp",
    "image/heic",
    "image/heif",
];
const OOXML_MIME_TYPES: &[&str] = &[
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
];
const OLE_MIME_TYPES: &[&str] = &[
    "application/msword",
    "application/vnd.ms-excel",
    "application/vnd.ms-powerpoint",
];
const TEXT_MIME_TYPES: &[&str] = &["text/plain", "text/csv"];

fn default_document_mime_types() -> Vec<String> {
    ["application/pdf"]
        .iter()
        .chain(OLE_MIME_TYPES)
        .chain(OOXML_MIME_TYPES)
        .chain(TEXT_MIME_TYPES)
        .chain(PHOTO_MIME_TYPES)
        .map(|s| (*s).to_owned())
        .collect()
}

fn ascii_at(bytes: &[u8], offset: usize, len: usize) -> String {
    bytes
        .iter()
        .skip(offset)
        .take(len)
        .map(|b| *b as char)
        .collect()
}

/// `sniffMimeType`: the type the first bytes prove, or `None`.
pub fn sniff_mime_type(head: &[u8], declared: &str) -> Option<String> {
    if head.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg".into());
    }
    if head.starts_with(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Some("image/png".into());
    }
    if ascii_at(head, 0, 4) == "RIFF" && ascii_at(head, 8, 4) == "WEBP" {
        return Some("image/webp".into());
    }
    const HEIC: &[&str] = &[
        "heic", "heix", "hevc", "hevx", "heim", "heis", "mif1", "msf1",
    ];
    if ascii_at(head, 4, 4) == "ftyp" && HEIC.contains(&ascii_at(head, 8, 4).as_str()) {
        return Some("image/heic".into());
    }
    if ascii_at(head, 0, 5) == "%PDF-" {
        return Some("application/pdf".into());
    }
    let declared = declared.to_lowercase();
    if head.starts_with(&[0x50, 0x4b, 0x03, 0x04]) {
        return OOXML_MIME_TYPES
            .contains(&declared.as_str())
            .then_some(declared);
    }
    if head.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) {
        return OLE_MIME_TYPES
            .contains(&declared.as_str())
            .then_some(declared);
    }
    if TEXT_MIME_TYPES.contains(&declared.as_str()) {
        static MARKUP: OnceLock<Regex> = OnceLock::new();
        let markup = MARKUP.get_or_init(|| {
            Regex::new(r"(?i)<\s*(!doctype|html|script|svg|iframe|\?xml)").expect("regex")
        });
        let binary = head.contains(&0);
        return (!binary && !markup.is_match(&String::from_utf8_lossy(head))).then_some(declared);
    }
    None
}

/// `MiniAppUploadRules`.
#[derive(Debug, Clone, PartialEq)]
pub struct UploadRules {
    pub is_photo: bool,
    pub max_bytes: usize,
    pub max_files: usize,
    pub allowed_mime_types: Vec<String>,
}

fn file_pickers(definition: &Definition) -> Vec<&Node> {
    let mut pickers = Vec::new();
    for screen in &definition.screens {
        walk(&screen.children, &mut |v| {
            if matches!(v.node.kind.as_str(), "PhotoPicker" | "DocumentPicker") {
                pickers.push(v.node);
            }
        });
    }
    pickers
}

fn read_number(value: Option<&Value>, fallback: f64) -> f64 {
    value
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .unwrap_or(fallback)
}

/// `resolveUploadRules`: the rules of the file picker named `input_name`.
pub fn resolve_upload_rules(definition: &Definition, input_name: &str) -> Option<UploadRules> {
    let node = file_pickers(definition)
        .into_iter()
        .find(|n| node_name(n) == Some(input_name))?;
    let is_photo = node.kind == "PhotoPicker";
    let size_kb = read_number(
        node.props.get("max-file-size-kb"),
        (MAX_UPLOAD_BYTES / 1024) as f64,
    );
    let files_key = if is_photo {
        "max-uploaded-photos"
    } else {
        "max-uploaded-documents"
    };
    let max_files = read_number(node.props.get(files_key), MAX_UPLOADS_PER_INPUT as f64);
    let allowed_mime_types = if is_photo {
        PHOTO_MIME_TYPES.iter().map(|s| (*s).to_owned()).collect()
    } else {
        match node.props.get("allowed-mime-types") {
            Some(Value::Array(types)) if !types.is_empty() => types
                .iter()
                .map(|t| crate::js::string_of(t).to_lowercase())
                .collect(),
            _ => default_document_mime_types(),
        }
    };
    Some(UploadRules {
        is_photo,
        max_bytes: (size_kb * 1024.0).min(MAX_UPLOAD_BYTES as f64).max(0.0) as usize,
        max_files: max_files.min(MAX_UPLOADS_PER_INPUT as f64).max(0.0) as usize,
        allowed_mime_types,
    })
}

/// `verifyUpload`: the sniffed MIME type, or the rejection reason
/// (`empty`, `too_large`, `type_not_allowed`).
pub fn verify_upload(
    rules: &UploadRules,
    size: usize,
    head: &[u8],
    declared: &str,
) -> Result<String, &'static str> {
    if size == 0 {
        return Err("empty");
    }
    if size > rules.max_bytes {
        return Err("too_large");
    }
    static BLOCKED: OnceLock<Regex> = OnceLock::new();
    let blocked = BLOCKED.get_or_init(|| {
        Regex::new("(?i)(html|svg|xml|javascript|ecmascript|x-sh|x-php|x-msdownload|x-executable)")
            .expect("regex")
    });
    let Some(mime) = sniff_mime_type(head, declared) else {
        return Err("type_not_allowed");
    };
    let allowed = rules.allowed_mime_types.contains(&mime)
        || (mime == "image/heic" && rules.allowed_mime_types.iter().any(|t| t == "image/heif"));
    if blocked.is_match(&mime) || !allowed {
        return Err("type_not_allowed");
    }
    Ok(mime)
}

/// `buildUploadPath`.
pub fn build_upload_path(
    workspace_id: &str,
    mini_app_id: &str,
    contact_id: Option<&str>,
    random_key: &str,
    file_name: &str,
) -> String {
    match contact_id {
        Some(contact) => format!(
            "public/space/{workspace_id}/contacts/{contact}/mini-apps/{random_key}/{file_name}"
        ),
        None => format!(
            "public/space/{workspace_id}/mini-apps/{mini_app_id}/anonymous/{random_key}/{file_name}"
        ),
    }
}

/// `collectFileInputNames`.
pub fn collect_file_input_names(definition: &Definition) -> HashSet<String> {
    file_pickers(definition)
        .into_iter()
        .filter_map(node_name)
        .map(str::to_owned)
        .collect()
}
