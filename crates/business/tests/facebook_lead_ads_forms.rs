//! Q.P5-fixD: `listFacebookLeadAdsFormsPublicResponse` strips unknown keys.

use chatbotx_business::facebook_lead_ads::public_form;
use serde_json::json;

#[test]
fn public_form_keeps_only_the_schema_keys() {
    let form = json!({
        "id": "1001", "name": "Spring", "status": "ACTIVE", "locale": "en_US",
        "questions": [{ "key": "email", "label": "Email", "type": "EMAIL", "id": "q1", "options": [] }],
    });
    assert_eq!(
        public_form(form),
        json!({ "id": "1001", "name": "Spring", "status": "ACTIVE",
                "questions": [{ "key": "email", "label": "Email", "type": "EMAIL", "id": "q1" }] })
    );
    assert_eq!(
        public_form(json!({ "id": "2", "name": "Old", "status": "ARCHIVED" })),
        json!({ "id": "2", "name": "Old", "status": "ARCHIVED" })
    );
}
