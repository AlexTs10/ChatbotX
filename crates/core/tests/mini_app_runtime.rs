//! Ports of `packages/mini-app/__tests__/custom-fields.test.ts` and
//! `uploads.test.ts`.

mod mini_app_fixtures;

use std::collections::BTreeMap;

use chatbotx_core::mini_app::{
    Definition, apply_custom_field_mappings, build_upload_path, collect_custom_field_mappings,
    format_answer_for_custom_field, parse_definition, resolve_upload_rules, sanitize_answers, sniff_mime_type,
    to_flow_json, verify_upload,
};
use chatbotx_core::utils::sanitize_file_name::sanitize_file_name;
use mini_app_fixtures::two_screen_definition;
use serde_json::json;

fn two_screens() -> Definition {
    parse_definition(&two_screen_definition()).unwrap()
}

fn mappings(entries: &[(&str, Option<&str>)]) -> BTreeMap<String, Option<String>> {
    entries.iter().map(|(k, v)| ((*k).to_owned(), v.map(str::to_owned))).collect()
}

#[test]
fn custom_field_mappings_are_collected_by_input_name_and_never_reach_flow_json() {
    let mut definition = two_screens();
    apply_custom_field_mappings(&mut definition, &mappings(&[("full_name", Some("cf-1"))]));
    assert_eq!(collect_custom_field_mappings(&definition), BTreeMap::from([("full_name".into(), "cf-1".into())]));
    assert!(!to_flow_json(&definition).to_string().contains("cf-1"));
}

#[test]
fn custom_field_mappings_can_be_cleared_and_ignore_unknown_names() {
    let mut definition = two_screens();
    apply_custom_field_mappings(&mut definition, &mappings(&[("full_name", Some("cf-1")), ("nope", Some("cf-2"))]));
    apply_custom_field_mappings(&mut definition, &mappings(&[("full_name", None)]));
    assert!(collect_custom_field_mappings(&definition).is_empty());
}

#[test]
fn format_answer_for_custom_field_formats_each_answer_shape() {
    assert_eq!(format_answer_for_custom_field(&json!("Ana")).as_deref(), Some("Ana"));
    assert_eq!(format_answer_for_custom_field(&json!(true)).as_deref(), Some("true"));
    assert_eq!(format_answer_for_custom_field(&json!(["a", "b"])).as_deref(), Some("a, b"));
    assert_eq!(
        format_answer_for_custom_field(&json!({ "start-date": "2026-01-01", "end-date": "2026-01-03" })).as_deref(),
        Some("2026-01-01 - 2026-01-03")
    );
    assert_eq!(format_answer_for_custom_field(&json!([])), None);
    assert_eq!(format_answer_for_custom_field(&json!("")), None);
}

const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0, 0];
const PDF: &[u8] = b"%PDF-1.7\n";

fn uploads_definition() -> Definition {
    parse_definition(&json!({ "screens": [{ "key": "s1", "id": "WELCOME", "title": "", "children": [
        { "id": "p", "type": "PhotoPicker", "props": { "name": "photo", "label": "Photo",
            "max-file-size-kb": 100, "max-uploaded-photos": 2 } },
        { "id": "d", "type": "DocumentPicker", "props": { "name": "doc", "label": "Doc",
            "allowed-mime-types": ["application/pdf"] } },
        { "id": "t", "type": "TextInput", "props": { "name": "full_name", "label": "Name" } }
    ] }] }))
    .unwrap()
}

#[test]
fn sniff_mime_type_identifies_files_from_their_first_bytes() {
    assert_eq!(sniff_mime_type(JPEG, "application/pdf").as_deref(), Some("image/jpeg"));
    assert_eq!(sniff_mime_type(PDF, "image/png").as_deref(), Some("application/pdf"));
    assert_eq!(sniff_mime_type(b"RIFF\0\0\0\0WEBPVP8 ", "").as_deref(), Some("image/webp"));
}

#[test]
fn sniff_mime_type_trusts_office_and_text_types_only_when_the_bytes_agree() {
    let zip = &[0x50, 0x4b, 0x03, 0x04];
    let docx = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    assert_eq!(sniff_mime_type(zip, docx).as_deref(), Some(docx));
    assert_eq!(sniff_mime_type(zip, "application/zip"), None);
    assert_eq!(sniff_mime_type(b"a,b\n1,2", "text/csv").as_deref(), Some("text/csv"));
    assert_eq!(sniff_mime_type(b"<html><script>x</script>", "text/plain"), None);
}

#[test]
fn sniff_mime_type_rejects_html_renamed_to_an_image() {
    assert_eq!(sniff_mime_type(b"<!doctype html><svg/>", "image/png"), None);
}

#[test]
fn verify_upload_applies_the_inputs_size_and_count_limits() {
    let photo = resolve_upload_rules(&uploads_definition(), "photo").unwrap();
    assert!(photo.is_photo);
    assert_eq!((photo.max_bytes, photo.max_files), (100 * 1024, 2));
    assert!(resolve_upload_rules(&uploads_definition(), "full_name").is_none());
}

#[test]
fn verify_upload_accepts_allowed_types_and_rejects_the_rest() {
    let definition = uploads_definition();
    let photo = resolve_upload_rules(&definition, "photo").unwrap();
    let doc = resolve_upload_rules(&definition, "doc").unwrap();
    assert_eq!(verify_upload(&photo, 10, JPEG, "image/jpeg"), Ok("image/jpeg".into()));
    assert_eq!(verify_upload(&photo, 10, PDF, "image/jpeg"), Err("type_not_allowed"));
    assert_eq!(verify_upload(&doc, 10, JPEG, "image/jpeg"), Err("type_not_allowed"));
    assert_eq!(verify_upload(&photo, 200 * 1024, JPEG, ""), Err("too_large"));
    assert_eq!(verify_upload(&photo, 0, JPEG, ""), Err("empty"));
}

#[test]
fn file_names_and_paths_sanitizes_names_and_keeps_the_extension() {
    assert_eq!(sanitize_file_name("CMND mặt trước.JPG"), "cmnd-mat-truoc.jpg");
    assert_eq!(sanitize_file_name("Đơn đăng ký (1).pdf"), "don-dang-ky-1.pdf");
    assert_eq!(sanitize_file_name("###.png"), "file.png");
}

#[test]
fn file_names_and_paths_stores_a_contacts_files_in_the_contact_folder_anonymous_ones_under_the_mini_app() {
    assert_eq!(build_upload_path("1", "9", Some("7"), "k", "a.jpg"), "public/space/1/contacts/7/mini-apps/k/a.jpg");
    assert_eq!(build_upload_path("1", "9", None, "k", "a.jpg"), "public/space/1/mini-apps/9/anonymous/k/a.jpg");
}

#[test]
fn file_answers_accept_only_upload_ids_for_file_inputs() {
    let id = "AAAAAAAAAAAAAAAAAAAAAA";
    let answers =
        sanitize_answers(&uploads_definition(), &json!({ "photo": [id], "doc": ["https://evil.example/x.pdf"] }));
    assert_eq!(serde_json::Value::Object(answers.unwrap()), json!({ "photo": [id] }));
}

#[test]
fn file_answers_store_public_urls_comma_joined_in_a_custom_field() {
    let file = |url: &str| json!({ "uploadId": "x", "url": url, "name": "a", "mimeType": "image/jpeg", "size": 1 });
    assert_eq!(
        format_answer_for_custom_field(&json!([file("https://a/1.jpg"), file("https://a/2.jpg")])).as_deref(),
        Some("https://a/1.jpg, https://a/2.jpg")
    );
}
