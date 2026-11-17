//! Ports of `apps/builder/__tests__/ai-mcp-server-bot-field-variable.test.ts`
//! and `ai-mcp-servers-public-resource.test.ts` (Q.D6): the request schema
//! (`parse_ai_mcp_server_input`, private = the builder form) and the public
//! resource redaction (`public_auth`).

use chatbotx_api::handlers::ai_mcp_servers::{parse_ai_mcp_server_input, public_auth};
use chatbotx_api::handlers::input::Input;
use serde_json::{Value, json};

fn template_input(token: &str) -> Value {
    json!({
        "auth": { "token": token, "type": "token" },
        "availableTools": {},
        "name": "MCP server",
        "selectedTools": [],
        "url": "https://mcp.example.test",
    })
}

fn parses(body: Value, private: bool) -> bool {
    let Value::Object(map) = body else {
        unreachable!()
    };
    let mut input = Input::from_object(map);
    parse_ai_mcp_server_input(&mut input, private).is_some() && input.finish().is_ok()
}

// describe("private MCP server token schema")

#[test]
fn accepts_static_text_and_bot_field_variables() {
    assert!(parses(template_input("static-token"), true));
    assert!(parses(template_input("Bearer {{bot_field:42}}"), true));
}

#[test]
fn rejects_contact_variables_without_changing_the_public_api_schema() {
    assert!(!parses(template_input("{{email}}"), true));
    assert!(parses(template_input("{{email}}"), false));
}

// describe("publicAIMcpServerResource auth redaction")

#[test]
fn strips_the_bearer_token_for_type_token() {
    let auth = public_auth(&json!({ "type": "token", "token": "super-secret-token" }));
    assert_eq!(auth, json!({ "type": "token" }));
    assert!(!auth.to_string().contains("super-secret-token"));
}

#[test]
fn strips_header_values_for_type_header() {
    let auth = public_auth(&json!({
        "type": "header",
        "headers": [{ "header": "X-Api-Key", "value": "super-secret-value" }],
    }));
    assert_eq!(
        auth,
        json!({ "type": "header", "headers": [{ "header": "X-Api-Key" }] })
    );
    assert!(!auth.to_string().contains("super-secret-value"));
}

#[test]
fn passes_through_type_none_with_no_secret_fields() {
    assert_eq!(
        public_auth(&json!({ "type": "none" })),
        json!({ "type": "none" })
    );
}
