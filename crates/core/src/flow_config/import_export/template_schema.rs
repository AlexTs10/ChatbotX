//! Port of `packages/flow-config/src/import-export/template-schema.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub const TEMPLATE_EXPORT_FORMAT_VERSION: usize = 1;

pub const TEMPLATE_EXPORT_FORMAT_VERSION_V2: usize = 2;

schema!(pub fn template_custom_field_manifest_entry_schema = z::object(vec![("name", z::string().trim().min(1)),
("type", deps::custom_field_types()),
("folderSourceId", z::string().optional())]));

schema!(pub fn template_tag_manifest_entry_schema = z::object(vec![("name", z::string().trim().min(1)),
("folderSourceId", z::string().optional())]));

schema!(pub fn template_product_category_manifest_entry_schema = z::object(vec![("name", z::string().trim().min(1)),
("parentSourceId", z::string().nullable()),
("rank", z::number().int().optional())]));

schema!(pub fn template_place_category_manifest_entry_schema = z::object(vec![("name", z::string().trim().min(1))]));

schema!(pub fn template_folder_manifest_entry_schema = z::object(vec![("name", z::string().trim().min(1)),
("folderType", z::string()),
("parentSourceId", z::string().nullable())]));

schema!(pub fn template_pipeline_manifest_entry_schema = z::object(vec![("name", z::string().trim().min(1)),
("referenceName", z::string().trim().min(1)),
("type", z::enum_(&["opportunity", "ticket", "custom"])),
("currency", z::string().nullable().default(json!(null))),
("settings", z::record(z::string(), z::unknown()).default(json!({}))),
("stages", z::array(z::object(vec![("sourceId", z::string()),
("name", z::string().trim().min(1)),
("description", z::string().nullable().default(json!(null))),
("textColor", z::string().nullable().default(json!(null))),
("backgroundColor", z::string().nullable().default(json!(null)))])).default(json!([]))),
("fields", z::array(z::object(vec![("sourceId", z::string()),
("name", z::string().trim().min(1)),
("referenceName", z::string().trim().min(1)),
("type", deps::custom_field_types()),
("required", z::boolean().default(json!(false)))])).default(json!([])))]));

schema!(pub fn template_inbox_manifest_entry_schema = z::object(vec![("channelType", z::string().min(1)),
("name", z::string())]));

schema!(pub fn template_manifests_schema = z::object(vec![("customFields", z::record(z::string(), template_custom_field_manifest_entry_schema()).default(json!({}))),
("tags", z::record(z::string(), template_tag_manifest_entry_schema()).default(json!({}))),
("productCategories", z::record(z::string(), template_product_category_manifest_entry_schema()).default(json!({}))),
("folders", z::record(z::string(), template_folder_manifest_entry_schema()).default(json!({}))),
("pipelines", z::record(z::string(), template_pipeline_manifest_entry_schema()).default(json!({}))),
("inboxes", z::record(z::string(), template_inbox_manifest_entry_schema()).default(json!({}))),
("placeCategories", z::record(z::string(), template_place_category_manifest_entry_schema()).default(json!({})))]));

schema!(pub fn template_flow_node_schema = z::union(vec![flow_version_schema(),
z::custom(template_flow_node_schema_1_custom, Some("Invalid flow node")).error("Invalid flow node")]));

schema!(pub fn template_flow_entry_schema = z::intersection(z::object(vec![("sourceId", z::string()),
("folderId", z::string().nullable().optional()),
("published", z::boolean().optional())]), z::object(vec![("name", z::string().trim().min(1).max(255)),
("active", z::boolean()),
("enableInInbox", z::boolean()),
("startNodeId", z::string()),
("nodes", z::array(template_flow_node_schema())),
("edges", z::array(edge_schema()))])));

schema!(pub fn template_resources_schema = z::object(vec![("flows", z::array(template_flow_entry_schema()).default(json!([]))),
("products", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("aiFunctions", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("aiAgents", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("calendars", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("webchats", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("keywords", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("entryPointLinks", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("triggers", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("fbCommentAutomations", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("sequences", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("aiMcpServers", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("aiFiles", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("questionnaires", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("couponTopics", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("pipelines", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("settings", z::object(vec![("savedReplies", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([]))),
("botFields", z::array(z::intersection(z::object(vec![("sourceId", z::string())]), z::record(z::string(), z::unknown()))).default(json!([])))]).default(json!({"savedReplies":[],"botFields":[]})))]));

schema!(pub fn template_item_permissions_schema = z::record(z::string(), z::record(z::string(), z::object(vec![("allowEdit", z::boolean())]))));

schema!(pub fn template_export_schema = z::object(vec![("formatVersion", z::union(vec![z::literal(json!(1)),
z::literal(json!(2))])),
("permissions", template_item_permissions_schema().optional()),
("exportedAt", z::string()),
("source", z::object(vec![("workspaceId", z::string()),
("tenantId", z::string())])),
("manifests", template_manifests_schema()),
("resources", template_resources_schema())]));

// HAND-PORT parseTemplateExport:
// raw=>{const preParsed=z.object({formatVersion:z.unknown()}).safeParse(raw);if(preParsed.success&&!SUPPORTED_FORMAT_VERSIONS.includes(preParsed.data.formatVersion)){return{ok:false,reason:`Unsupported template export format version: ${String(preParsed.data.formatVersion)}`}}const result=templateExportSchema.safeParse(raw);if(!result.success){return{ok:false,reason:result.error.message}}return{ok:true,data:result.data}}

// node=>isRecord(node)&&typeof node.id==="string"&&typeof node.type==="string"&&isRecord(node.data)&&isRecord(node.data.details)&&(!("steps"in node.data.details)||Array.isArray(node.data.details.steps))
fn template_flow_node_schema_1_custom(value: &Value) -> bool {
    todo!("P3.4")
}

// ---- stage 1 API (hand-ported in stage 2) ----

pub fn parse_template_export(raw: &Value) -> Result<Value, String> {
    todo!("P3.4")
}
