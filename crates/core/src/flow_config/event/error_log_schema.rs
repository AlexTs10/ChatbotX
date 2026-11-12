//! Port of `packages/flow-config/src/event/error-log-schema.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

schema!(pub fn error_log_event_type_schema = z::enum_(&["error-log:recorded"]));

schema!(pub fn error_log_recorded_payload_schema = z::object(vec![("id", z::string()),
("workspaceId", z::string()),
("provider", deps::error_log_providers()),
("contactId", z::string().optional()),
("sourceId", z::string().optional()),
("error", z::object(vec![("message", z::string()),
("httpCode", z::string().nullable()),
("stackTrace", z::string().optional())]))]));

// HAND-PORT errorLogEventSchemas (object): {"error-log:recorded":"<schema errorLogRecordedPayloadSchema>"}
