//! Port of `packages/flow-config/src/publish-schema.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn publish_flow_schema = z::object(vec![("nodes", z::array(flow_version_schema()).super_refine(|nodes, ctx| refine_steps_by_channel(nodes.as_array().map_or(&[][..], Vec::as_slice), ctx)).describe("Raw flow node graph, as sent by the builder UI.")),
("edges", z::array(edge_schema()).describe("Raw flow edge graph, as sent by the builder UI."))]));
