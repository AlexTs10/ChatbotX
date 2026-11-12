//! Port of `packages/flow-config/src/steps/find-places-near-me.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn find_places_near_me_step_schema = z::object(vec![("id", z::string().regex(r#"^\d+$"#, "").describe("Step id (numeric string), unique within the flow.")),
("nodeId", z::string().optional().describe("Id of the node containing this step. Optional.")),
("stepType", z::literal(json!("findPlacesNearMe")).describe("Step type discriminator: \"findPlacesNearMe\".")),
("placeCategoryId", z::string().regex(r#"^\d+$"#, "").nullable().describe("Place category id from placeCategories.list; null = all locations."))]));

default_fn!(pub fn find_places_near_me_step_default_fn = json!({"id": create_id(), "stepType": "findPlacesNearMe", "placeCategoryId": null}));
