//! Port of `packages/flow-config/src/steps/sendgrid-add-contact.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn send_grid_merge_field_mapping_schema = z::object(vec![("contactFieldId", z::string().trim().min(1)),
("sendGridField", z::string().trim().min(1))]));

schema!(pub fn send_grid_add_contact_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("sendGridAddContact")).describe("Step type discriminator: \"sendGridAddContact\".")),
("listId", z::string().transform_opt(send_grid_add_contact_schema_list_id_transform).optional()),
("emailField", z::string().trim().min(1)),
("phoneField", z::string().transform_opt(send_grid_add_contact_schema_list_id_transform).optional()),
("mergeFields", z::array(send_grid_merge_field_mapping_schema())),
("states", z::tuple(vec![success_state_schema(),
error_state_schema()]))]).super_refine(send_grid_add_contact_schema_super_refine));

default_fn!(pub fn send_grid_add_contact_default_fn = json!({"id": create_id(), "stepType": "sendGridAddContact", "emailField": "email", "mergeFields": [], "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "error"}]}));

fn send_grid_add_contact_schema_list_id_transform(
    value: Option<Value>,
    _ctx: &mut RefineCtx,
) -> Option<Value> {
    // `(value) => value.trim() || undefined`
    match value {
        Some(Value::String(s)) => {
            let t = crate::js::trim(&s);
            (!t.is_empty()).then(|| Value::String(t.to_owned()))
        }
        other => other,
    }
}

fn send_grid_add_contact_schema_super_refine(data: &Value, ctx: &mut RefineCtx) {
    report_duplicates(
        ctx,
        arr_field(data, "mergeFields"),
        |m| key_of(&m["sendGridField"]),
        |i, k| {
            (
                path!["mergeFields", i, "sendGridField"],
                format!("Duplicate SendGrid field mapping: \"{k}\""),
            )
        },
    );
}
