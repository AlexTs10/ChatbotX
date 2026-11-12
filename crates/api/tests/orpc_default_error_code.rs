//! Port of `apps/builder/__tests__/orpc-default-error-code.test.ts`.

use chatbotx_api::errors::ChatbotXException;
use chatbotx_api::orpc::common_api_error;

// A `ChatbotXException` thrown without a code carries `systemError` / 400, and
// the oRPC layer forwards that code unchanged. It must be a declared common
// code, otherwise every such rejection is served as `defined: false`.
#[test]
fn default_chatbotxexception_code_is_declared_once_for_every_public_route_with_its_real_status() {
    let error = ChatbotXException::new("Broadcast is not a draft");

    assert_eq!(error.code, "systemError");
    let declared = common_api_error(&error.code).expect("declared");
    assert_eq!(declared.status, error.http_status_code);
}
