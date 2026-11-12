//! Port of `packages/flow-config/__tests__/send-multiple-images.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn step(images: Value) -> Value {
    json!({ "id": "1", "stepType": "sendMultipleImages", "images": images })
}

fn n_images(n: usize) -> Value {
    Value::Array(
        (0..n)
            .map(|i| json!({ "id": format!("img-{i}"), "mode": "url", "url": format!("https://example.com/{i}.png") }))
            .collect(),
    )
}

#[test]
fn sendmultipleimagesstepschema_accepts_2_images_min() {
    let result = fc::send_multiple_images_step_schema().safe_parse(&step(json!([
        { "id": "img-1", "mode": "url", "url": "https://example.com/a.png" },
        { "id": "img-2", "mode": "url", "url": "https://example.com/b.png" },
    ])));
    assert!(result.success);
}

#[test]
fn sendmultipleimagesstepschema_accepts_10_images_max() {
    let result = fc::send_multiple_images_step_schema().safe_parse(&step(n_images(10)));
    assert!(result.success);
}

#[test]
fn sendmultipleimagesstepschema_rejects_fewer_than_2_images() {
    let result = fc::send_multiple_images_step_schema().safe_parse(&step(json!([
        { "id": "img-1", "mode": "url", "url": "https://example.com/a.png" },
    ])));
    assert!(!result.success);
}

#[test]
fn sendmultipleimagesstepschema_rejects_more_than_10_images() {
    let result = fc::send_multiple_images_step_schema().safe_parse(&step(n_images(11)));
    assert!(!result.success);
}

#[test]
fn sendmultipleimagesstepschema_accepts_a_variable_placeholder_per_item_resolved_at_send_time() {
    let result = fc::send_multiple_images_step_schema().safe_parse(&step(json!([
        { "id": "img-1", "mode": "url", "url": "{{customField}}" },
        { "id": "img-2", "mode": "url", "url": "https://example.com/b.png" },
    ])));
    assert!(result.success);
}

#[test]
fn sendmultipleimagesstepschema_rejects_an_item_with_an_invalid_non_placeholder_non_url_url() {
    let result = fc::send_multiple_images_step_schema().safe_parse(&step(json!([
        { "id": "img-1", "mode": "url", "url": "not-a-url" },
        { "id": "img-2", "mode": "url", "url": "https://example.com/b.png" },
    ])));
    assert!(!result.success);
}

#[test]
fn sendmultipleimagesitemdefaultfn_defaults_to_file_mode_with_an_empty_url() {
    let item = fc::send_multiple_images_item_default_fn();
    assert_eq!(item["mode"], "file");
    assert_eq!(item["url"], "");
}

#[test]
fn sendmultipleimagesitemdefaultfn_two_calls_produce_different_ids() {
    let a = fc::send_multiple_images_item_default_fn();
    let b = fc::send_multiple_images_item_default_fn();
    assert_ne!(a["id"], b["id"]);
}

#[test]
fn sendmultipleimagesstepdefaultfn_steptype_is_sendmultipleimages() {
    let step = fc::send_multiple_images_step_default_fn();
    assert_eq!(step["stepType"], "sendMultipleImages");
}

#[test]
fn sendmultipleimagesstepdefaultfn_seeds_exactly_2_empty_image_slots_the_schema_minimum() {
    let step = fc::send_multiple_images_step_default_fn();
    assert_eq!(step["images"].as_array().unwrap().len(), 2);
}

#[test]
fn sendmultipleimagesstepdefaultfn_id_is_always_generated_non_empty_string() {
    let step = fc::send_multiple_images_step_default_fn();
    assert!(!step["id"].as_str().unwrap().is_empty());
}
