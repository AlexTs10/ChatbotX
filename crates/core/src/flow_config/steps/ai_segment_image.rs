//! Port of `packages/flow-config/src/steps/ai-segment-image.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const AI_SEGMENT_IMAGE_TARGET_MAX_LENGTH: usize = 160;

schema!(pub fn ai_segment_image_match_selection = z::enum_(&["all", "largest"]));

schema!(pub fn ai_segment_image_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("stepType", z::literal(json!("aiSegmentImage")).describe("Step type discriminator: \"aiSegmentImage\".")),
("inputFieldId", z::string().trim().min(1).refine(ai_segment_image_schema_input_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None).describe("Custom field holding the source image URL.")),
("target", z::string().trim().min(1).max(160).describe("Short concrete noun phrase naming the object to find, e.g. \"red car\" or \"product box\". Not a command or question.")),
("matchSelection", ai_segment_image_match_selection().describe("\"largest\" crops to the largest matched object's box; \"all\" crops to the area covering every match.")),
("outputFieldId", z::string().trim().min(1).refine(ai_segment_image_schema_output_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None).describe("Custom field that receives the cropped image URL: JPEG, or PNG when the source has real transparency. May equal inputFieldId to replace the source image.")),
("detailsOutputFieldId", z::string().trim().min(1).refine(ai_segment_image_schema_details_output_field_id_refine, Some("Value starting with \"bot_field:\" must be a valid bot field reference (e.g. \"bot_field:123\")."), None).optional().describe("Optional custom field that receives JSON detection details ({ count, boxes, crop, source }); boxes are half-open source pixels with an optional 0-1 confidence.")),
("states", z::tuple(vec![success_state_schema(),
skip_state_schema(),
error_state_schema()]).describe("Exactly [success, skip, error]: success = object found, skip = no object found, error = processing failed."))]).super_refine(ai_segment_image_schema_super_refine));

fn ai_segment_image_schema_input_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

fn ai_segment_image_schema_output_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

fn ai_segment_image_schema_details_output_field_id_refine(value: &Value) -> bool {
    field_reference_ok(value)
}

/// `aiSegmentImageDefaultFn(props)`.
pub fn ai_segment_image_default_fn(props: &Value) -> Value {
    spread3(
        json!({"id": create_id(), "inputFieldId": "", "target": "", "matchSelection": "largest", "outputFieldId": ""}),
        props,
        json!({"stepType": "aiSegmentImage", "states": [{"id": create_id(), "stateType": "success"}, {"id": create_id(), "stateType": "skip"}, {"id": create_id(), "stateType": "error"}]}),
    )
}

fn ai_segment_image_schema_super_refine(value: &Value, ctx: &mut RefineCtx) {
    if !truthy(value.get("detailsOutputFieldId")) {
        return;
    }
    if value["detailsOutputFieldId"] == value["outputFieldId"] {
        ctx.issue(
            path!["detailsOutputFieldId"],
            "Details field must differ from the output image field",
        );
    }
    if value["detailsOutputFieldId"] == value["inputFieldId"] {
        ctx.issue(
            path!["detailsOutputFieldId"],
            "Details field must differ from the source image field",
        );
    }
}
