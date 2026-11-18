//! `installed-resource.service.ts` guards (`assertDeletable`,
//! `assertEditable` / `assertResourceEditable`) and `lock-policy.ts`
//! (`TEMPLATE_LOCKED_CONTENT_KEYS`; Q.D4 slice 6), called from the other
//! domains' writers inside their write transaction.
//!
//! ponytail: no `tpl-res:` advisory locks (SQLite has one writer, so a guard
//! and the write it protects already serialize against a sync, restore or
//! uninstall).

use chatbotx_db::repo::template_installed_resource::{TemplateResourcePermission, find_blocking};
use chatbotx_db::repo::template_installed_resource_uninstall::find_tombstoned;
use chatbotx_db::rusqlite::Connection;

use crate::errors::{BusinessError, Result};

/// `TEMPLATE_LOCKED_CONTENT_KEYS`: the content fields of each kind.
pub const LOCKED_CONTENT_KEYS: &[(&str, &[&str])] = &[
    (
        "flow",
        &["name", "nodes", "edges", "startNodeId", "currentVersionId"],
    ),
    (
        "aiAgent",
        &[
            "name",
            "prompt",
            "actionPrompt",
            "actionRules",
            "messages",
            "models",
            "temperature",
            "maxOutputTokens",
            "tools",
            "isRichResponse",
            "webSearchAuthorizedDomains",
        ],
    ),
    (
        "aiFunction",
        &[
            "name",
            "purpose",
            "dataCollect",
            "outputMessage",
            "triggerFlowId",
        ],
    ),
    (
        "calendar",
        &[
            "name",
            "description",
            "timezone",
            "durationMinutes",
            "bufferAfterMinutes",
            "locationType",
            "locationDetail",
            "scheduleWindowType",
            "scheduleWindowConfig",
            "maxAppointmentsPerUser",
            "dailyLimitEnabled",
            "maxPerDay",
            "allowGroupMeeting",
            "maxPerSlot",
            "confirmationMessage",
            "confirmationFlowId",
            "cancellationFlowId",
            "externalEventTitleTemplate",
            "externalEventDescriptionTemplate",
            "externalEventAttendeesTemplate",
            "availability",
            "reminders",
        ],
    ),
    (
        "integrationWebchat",
        &[
            "name",
            "conversationStarters",
            "persistentMenus",
            "brandColor",
            "hideHeader",
            "showLogo",
            "hideMessageInput",
            "customCss",
            "welcomeFlowId",
        ],
    ),
    (
        "automatedResponse",
        &["keywords", "conditions", "text", "flowId"],
    ),
    ("reflink", &["name", "flowId", "customFieldId", "qrStyles"]),
    ("trigger", &["name", "actions", "conditions"]),
    (
        "fbCommentAutomation",
        &[
            "name",
            "post",
            "privateReply",
            "publicReply",
            "includeKeywords",
            "excludeKeywords",
            "excludeKeywordsType",
            "options",
            "hideComments",
            "replyAfter",
            "startTime",
            "endTime",
        ],
    ),
    (
        "product",
        &[
            "name",
            "shortDescription",
            "longDescription",
            "price",
            "currency",
            "taxes",
            "discount",
            "images",
            "tags",
            "sku",
            "categoryId",
            "subcategoryId",
            "vendorId",
            "allowSpecialRequest",
            "isAddonOnly",
            "variants",
            "options",
            "addons",
        ],
    ),
    ("savedReply", &["shortcut", "text", "images"]),
    ("botField", &["name", "type", "description"]),
    ("customField", &["name", "type", "description"]),
    ("tag", &["name"]),
    ("folder", &["name"]),
    ("sequence", &["name", "steps"]),
    ("aiMcpServer", &["name", "selectedTools"]),
    ("aiFile", &["chunkingConfig"]),
    (
        "questionnaire",
        &[
            "name",
            "enableScore",
            "enableRetryMessages",
            "enableCustomFieldMapping",
            "triggerFlowId",
            "questions",
        ],
    ),
    ("couponTopic", &["name", "description", "expiresAt"]),
    (
        "pipeline",
        &["name", "referenceName", "settings", "stages", "fields"],
    ),
    ("productCategory", &["name", "parentId", "imageUrl"]),
    ("placeCategory", &[]),
];

/// `templateAllowDeleteViolationException`.
pub fn delete_violation() -> BusinessError {
    BusinessError::exception(
        "This resource was installed from a template that disallows deletion",
        "templateAllowDeleteViolation",
        400,
    )
}

/// `templateAllowEditViolationException`.
pub fn edit_violation() -> BusinessError {
    BusinessError::exception(
        "This resource was installed from a template that disallows editing",
        "templateAllowEditViolation",
        403,
    )
}

/// `assertDeletable`: a copy whose template forbids deleting it blocks the
/// delete (a reused row, a detached copy or one whose template was deleted
/// never does).
pub fn assert_deletable(
    conn: &Connection,
    workspace_id: i64,
    kind: &str,
    ids: &[i64],
) -> Result<()> {
    if find_blocking(
        conn,
        workspace_id,
        kind,
        ids,
        TemplateResourcePermission::AllowDelete,
        &[],
    )?
    .is_empty()
    {
        Ok(())
    } else {
        Err(delete_violation())
    }
}

/// `assertResourceEditable`: a copy being uninstalled rejects every write; a
/// write touching one of the kind's content fields is refused when the
/// template forbids editing the copy.
pub fn assert_editable(
    conn: &Connection,
    workspace_id: i64,
    kind: &str,
    ids: &[i64],
    changed_keys: &[&str],
) -> Result<()> {
    if !find_tombstoned(conn, workspace_id, kind, ids)?.is_empty() {
        return Err(BusinessError::exception(
            "This template is being uninstalled",
            "templateUninstallInProgress",
            409,
        ));
    }
    let locked = LOCKED_CONTENT_KEYS
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or(&[][..], |(_, keys)| *keys);
    if !changed_keys.iter().any(|key| locked.contains(key)) {
        return Ok(());
    }
    if find_blocking(
        conn,
        workspace_id,
        kind,
        ids,
        TemplateResourcePermission::AllowEdit,
        &[],
    )?
    .is_empty()
    {
        Ok(())
    } else {
        Err(edit_violation())
    }
}

/// [`assert_editable`] for a content write path (an update that may set any
/// of the kind's content fields).
///
/// ponytail: the writer does not diff fields; a content update counts as
/// touching locked content (toggles and moves have their own writers).
pub fn assert_content_editable(
    conn: &Connection,
    workspace_id: i64,
    kind: &str,
    ids: &[i64],
) -> Result<()> {
    let keys = LOCKED_CONTENT_KEYS
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or(&[][..], |(_, keys)| *keys);
    assert_editable(conn, workspace_id, kind, ids, keys)
}
