// Slice P3.4b: still being ported.
#![allow(unused_variables, dead_code)]
//! Port of `packages/flow-config/src/import-export/reference-fields.ts`.

#[allow(unused_imports)]
use crate::flow_config::prelude::*;

pub fn reference_field_entity_kind() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"inputFieldId":"customField","outputFieldId":"customField","detailsOutputFieldId":"customField","outputCustomFieldId":"customField","customFieldId":"customField","dateTimeFieldId":"customField","startDateFieldId":"customField","endDateFieldId":"customField","contactFieldId":"customField","botFieldId":"botField","sequenceId":"sequence","leaderboardId":"leaderboard","aiAgentId":"aiAgent","integrationId":"integration","integrationSmtpId":"integration","integrationMessengerId":"integration","calendarId":"calendar","questionnaireId":"questionnaire","topicId":"couponTopic","inboxId":"inbox","personaId":"messengerPersona","spreadsheetId":"spreadsheet","miniAppId":"miniApp","pipelineId":"pipeline","destinationPipelineId":"pipeline","stageId":"pipelineStage","destinationStageId":"pipelineStage","pipelineFieldId":"pipelineField","placeCategoryId":"placeCategory"}"#).expect("valid JSON")
    });
    &V
}

pub const FLOW_REFERENCE_FIELD: &str = "flowId";

pub const CROSS_FLOW_NODE_FIELD: &str = "nodeId";

pub fn reference_array_field_entity_kind() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"tagIds":"tag","sequenceIds":"sequence","addonProductIds":"product","productIds":"product","pipelineFieldIds":"pipelineField","assigneeMemberIds":"workspaceMember","assigneeInboxTeamIds":"inboxTeam"}"#).expect("valid JSON")
    });
    &V
}

// HAND-PORT STRIP_UNRESOLVED_ARRAY_REFERENCE_KINDS (object): {}

// HAND-PORT isWorkspaceBoundReferenceValue:
// (kind,value)=>STRIP_UNRESOLVED_ARRAY_REFERENCE_KINDS.has(kind)?value.length>0&&!ASSIGNEE_SENTINELS.has(value):NUMERIC_ID_PATTERN.test(value)

// HAND-PORT referenceKindsWithPolicy:
// policy=>Object.entries(REFERENCE_KIND_POLICY).filter(([,kindPolicy])=>kindPolicy===policy).map(([kind])=>kind)

pub fn discriminated_reference_fields() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"sourceId":{"discriminatorKey":"sourceType","kindByDiscriminator":{"trigger":"trigger","webhook":"webhook"}},"value":{"discriminatorKey":"field","kindByDiscriminator":{"inbox":"inbox","tags":"tag","subscribedToDripCampaign":"sequence","conversationAssigned":"workspaceMember","entryPointsLinks":"reflink","completedWhatsAppFlows":"whatsappFlow","keywordsReceived":"automatedResponse","executedFlow":"flow","executedStep":"flowNode","broadcastSent":"broadcast","broadcastDelivered":"broadcast","broadcastSeen":"broadcast","broadcastClicked":"broadcast","broadcastFailed":"broadcast","boughtItems":"product","shoppingCartContainsItems":"product","commentedOnPost":"external","clickedAd":"external","emailSent":"external","emailDelivered":"external","emailOpened":"external","emailClicked":"external"},"requireSiblings":["field","operator"]}}"#).expect("valid JSON")
    });
    &V
}

pub fn prefixed_reference_entity_kind() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(
            r#"{"fn":"aiFunction","file":"aiFile","mcp":"aiMcpServer","bot_field":"botField"}"#,
        )
        .expect("valid JSON")
    });
    &V
}

// HAND-PORT PREFIXED_REFERENCE_FIELDS (object): {}

pub fn reference_kind_policy() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"flow":"copy","flowNode":"copy","aiAgent":"copy","aiFunction":"copy","aiFile":"copy","aiMcpServer":"copy","trigger":"copy","calendar":"copy","questionnaire":"copy","couponTopic":"copy","reflink":"copy","automatedResponse":"copy","customField":"naturalKey","botField":"naturalKey","tag":"naturalKey","folder":"naturalKey","pipeline":"naturalKey","pipelineStage":"naturalKey","pipelineField":"naturalKey","productCategory":"naturalKey","placeCategory":"naturalKey","sequence":"verifyInTarget","inbox":"verifyInTarget","product":"verifyInTarget","vendor":"verifyInTarget","integration":"verifyInTarget","messengerPersona":"verifyInTarget","spreadsheet":"verifyInTarget","miniApp":"verifyInTarget","leaderboard":"verifyInTarget","webhook":"verifyInTarget","decisionProfile":"verifyInTarget","userPersistentMenu":"verifyInTarget","whatsappFlow":"verifyInTarget","messengerTemplate":"verifyInTarget","whatsappTemplate":"verifyInTarget","broadcast":"strip","workspaceMember":"strip","inboxTeam":"strip","external":"external"}"#).expect("valid JSON")
    });
    &V
}

pub fn step_reference_overrides() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"activeCampaignSyncContact":{"tagIds":"external"},"assignConversation":{"assignedId":"workspaceMember"},"autoAssignConversation":{"assignedIds":"workspaceMember"},"evaluateDecision":{"contextCustomFieldIds":"customField","profileId":"decisionProfile"},"setMessengerUserPersistentMenu":{"userPersistentMenuId":"userPersistentMenu"},"whatsappFlow":{"flow.id":"whatsappFlow"},"sendMessengerTemplateMessage":{"template.id":"messengerTemplate"},"sendWaTemplateMessage":{"template.id":"whatsappTemplate"},"sendCollection":{"conditions.categoryId":"productCategory","conditions.subcategoryId":"productCategory","conditions.vendorId":"vendor"},"sendViewCatalog":{"thumbnailProductId":"external"}}"#).expect("valid JSON")
    });
    &V
}

pub const MAX_WALK_DEPTH: usize = 512;

// HAND-PORT isPlainObject:
// value=>typeof value==="object"&&value!==null&&!Array.isArray(value)

pub fn reference_slot_shape_by_key() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"botFieldId":"optional","detailsOutputFieldId":"optional","integrationId":"optional","integrationMessengerId":"optional","leaderboardId":"optional","personaId":"optional","sequenceId":"optional","destinationStageId":"optional","topicId":"optional","inboxId":"optional","userPersistentMenuId":"optional"}"#).expect("valid JSON")
    });
    &V
}

pub fn step_reference_slot_shape() -> &'static Value {
    static V: LazyLock<Value> = LazyLock::new(|| {
        serde_json::from_str(r#"{"assignConversation":{"assignedId":"emptiable"},"sendWaTemplateMessage":{"inboxId":"nullable","template.inboxId":"nullable"},"spreadsheetSendData":{"map.customFieldId":"optional"},"spreadsheetUpdateRow":{"map.customFieldId":"optional"},"whatsappFlow":{"inboxId":"nullable","flow.fieldMappings.customFieldId":"nullable"},"sendMessage":{"sequenceId":"required"},"coupon":{"topicId":"required"},"findPlacesNearMe":{"placeCategoryId":"nullable"},"markCouponUsed":{"topicId":"required"},"setUpCoupon":{"topicId":"required"}}"#).expect("valid JSON")
    });
    &V
}
