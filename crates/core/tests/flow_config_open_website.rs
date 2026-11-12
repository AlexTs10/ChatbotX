//! Port of `packages/flow-config/__tests__/open-website.test.ts`.

mod flow_config_support;
use flow_config_support::*;

fn with_url(url: &str) -> Value {
    spread(&fc::open_website_step_default_fn(), json!({ "url": url }))
}

#[test]
fn openwebsitestepschema_accepts_absolute_urls() {
    let value = with_url("https://example.test/booking/slug");
    assert!(fc::open_website_step_schema().safe_parse(&value).success);
}

#[test]
fn openwebsitestepschema_accepts_url_templates_with_variable_placeholders() {
    let value = with_url("https://example.test/booking/slug?u={{user_id}}");
    assert!(fc::open_website_step_schema().safe_parse(&value).success);
}

#[test]
fn openwebsitestepschema_accepts_a_bare_variable_that_resolves_to_a_url_at_runtime() {
    let value = with_url("{{booking_link}}");
    assert!(fc::open_website_step_schema().safe_parse(&value).success);
}

#[test]
fn openwebsitestepschema_rejects_non_url_strings() {
    let value = with_url("not a url");
    assert!(!fc::open_website_step_schema().safe_parse(&value).success);
}
