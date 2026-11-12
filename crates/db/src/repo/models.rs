//! Row structs of every app table, generated from the migrations DDL by the P3.2 porter
//! (one field per column in column order, `ALTER TABLE ... ADD COLUMN` columns last; `bool`
//! for `IN (0, 1)` columns, [`Json`] for `json_valid` columns, `Option` for nullable columns).

use super::Json;

table_row! {
    /// Row of `"AIAgent"`.
    pub struct AiAgent in "AIAgent" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        prompt: Option<String> = "prompt",
        action_prompt: Option<String> = "actionPrompt",
        action_rules: Json = "actionRules",
        messages: Json = "messages",
        is_default: bool = "isDefault",
        is_rich_response: bool = "isRichResponse",
        tools: Json = "tools",
        web_search_authorized_domains: Json = "webSearchAuthorizedDomains",
        models: Json = "models",
        temperature: f64 = "temperature",
        max_output_tokens: i64 = "maxOutputTokens",
    }
}
table_row! {
    /// Row of `"AIConversationEmbedding"`.
    pub struct AiConversationEmbedding in "AIConversationEmbedding" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        source_id: i64 = "sourceId",
        workspace_id: i64 = "workspaceId",
        conversation_id: i64 = "conversationId",
        chunk_index: i64 = "chunkIndex",
        content: String = "content",
        embedding: Option<Vec<u8>> = "embedding",
        status: String = "status",
        error_message: Option<String> = "errorMessage",
    }
}
table_row! {
    /// Row of `"AIConversationSource"`.
    pub struct AiConversationSource in "AIConversationSource" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        conversation_id: i64 = "conversationId",
        message_id: i64 = "messageId",
        attachment_id: Option<i64> = "attachmentId",
        source_type: String = "sourceType",
        status: String = "status",
        source_key: String = "sourceKey",
        content_hash: Option<String> = "contentHash",
        mime_type: Option<String> = "mimeType",
        title: Option<String> = "title",
        metadata: Option<Json> = "metadata",
        summary: Option<String> = "summary",
        error_message: Option<String> = "errorMessage",
    }
}
table_row! {
    /// Row of `"AIEmbedding"`.
    pub struct AiEmbedding in "AIEmbedding" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        content: String = "content",
        embedding: Option<Vec<u8>> = "embedding",
        status: String = "status",
        chunk_index: Option<i64> = "chunkIndex",
        workspace_id: i64 = "workspaceId",
        ai_file_id: i64 = "aiFileId",
    }
}
table_row! {
    /// Row of `"AIFile"`.
    pub struct AiFile in "AIFile" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        path: String = "path",
        size: i64 = "size",
        mime_type: String = "mimeType",
        chunking: Option<Json> = "chunking",
        chunking_config: Option<Json> = "chunkingConfig",
        processing_run: Option<Json> = "processingRun",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"AIFunction"`.
    pub struct AiFunction in "AIFunction" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        purpose: Option<String> = "purpose",
        data_collect: Option<Json> = "dataCollect",
        output_message: Option<String> = "outputMessage",
        trigger_flow_id: Option<i64> = "triggerFlowId",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"AIHandoverBulkRun"`.
    pub struct AiHandoverBulkRun in "AIHandoverBulkRun" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
        channel: String = "channel",
        revision: i64 = "revision",
        action: String = "action",
        status: String = "status",
        message: Option<String> = "message",
        requested_by_user_id: Option<i64> = "requestedByUserId",
        requested_at: i64 = "requestedAt",
        started_at: Option<i64> = "startedAt",
        finished_at: Option<i64> = "finishedAt",
        last_heartbeat_at: Option<i64> = "lastHeartbeatAt",
        paused_until: Option<i64> = "pausedUntil",
        claim_token: Option<String> = "claimToken",
        attempts: i64 = "attempts",
        chunk_seq: i64 = "chunkSeq",
        total_count: Option<i64> = "totalCount",
        processed_count: i64 = "processedCount",
        skipped_count: i64 = "skippedCount",
        failed_count: i64 = "failedCount",
        cursor_contact_inbox_id: Option<String> = "cursorContactInboxId",
        in_flight_from_id: Option<String> = "inFlightFromId",
        in_flight_to_id: Option<String> = "inFlightToId",
        current_error: Option<String> = "currentError",
    }
}
table_row! {
    /// Row of `"AIHandoverSettings"`.
    pub struct AiHandoverSettings in "AIHandoverSettings" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
        channel: String = "channel",
        enabled: bool = "enabled",
        schedule_enabled: bool = "scheduleEnabled",
        time_ranges: Json = "timeRanges",
        goto_flow_id: Option<i64> = "gotoFlowId",
        return_message: Option<String> = "returnMessage",
        pause_bot_waiting_for_staff: bool = "pauseBotWaitingForStaff",
        apply_to_all_customers: bool = "applyToAllCustomers",
        apply_to_all_revision: i64 = "applyToAllRevision",
        apply_to_all_message: Option<String> = "applyToAllMessage",
        apply_to_all_requested_by_user_id: Option<i64> = "applyToAllRequestedByUserId",
    }
}
table_row! {
    /// Row of `"AIMCPServer"`.
    pub struct AiMcpServer in "AIMCPServer" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        url: String = "url",
        auth: Json = "auth",
        available_tools: Json = "availableTools",
        selected_tools: Json = "selectedTools",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"Account"`.
    pub struct Account in "Account" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        account_id: String = "accountId",
        provider_id: String = "providerId",
        access_token: Option<String> = "accessToken",
        access_token_expires_at: Option<i64> = "accessTokenExpiresAt",
        refresh_token: Option<String> = "refreshToken",
        refresh_token_expires_at: Option<i64> = "refreshTokenExpiresAt",
        scope: Option<String> = "scope",
        id_token: Option<String> = "idToken",
        password: Option<String> = "password",
        user_id: i64 = "userId",
        tenant_id: i64 = "tenantId",
    }
}
table_row! {
    /// Row of `"AdsConversionEvent"`.
    pub struct AdsConversionEvent in "AdsConversionEvent" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        channel: String = "channel",
        integration_whatsapp_id: Option<i64> = "integrationWhatsappId",
        integration_messenger_id: Option<i64> = "integrationMessengerId",
        integration_instagram_id: Option<i64> = "integrationInstagramId",
        waba_id: Option<String> = "wabaId",
        source: String = "source",
        event_type: String = "eventType",
        ctwa_clid: Option<String> = "ctwaClid",
        ad_id: Option<String> = "adId",
        contact_inbox_id: Option<i64> = "contactInboxId",
        referral_event_id: Option<i64> = "referralEventId",
        currency: Option<String> = "currency",
        value: Option<String> = "value",
        order_id: Option<String> = "orderId",
        contents: Option<Json> = "contents",
        occurred_at: i64 = "occurredAt",
        source_event_id: String = "sourceEventId",
        capi_status: String = "capiStatus",
        capi_sent_at: Option<i64> = "capiSentAt",
    }
}
table_row! {
    /// Row of `"AdsConversionRule"`.
    pub struct AdsConversionRule in "AdsConversionRule" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        channel: String = "channel",
        integration_whatsapp_id: Option<i64> = "integrationWhatsappId",
        integration_facebook_ads_id: Option<i64> = "integrationFacebookAdsId",
        integration_messenger_id: Option<i64> = "integrationMessengerId",
        integration_instagram_id: Option<i64> = "integrationInstagramId",
        ad_account_id: Option<String> = "adAccountId",
        event_type: String = "eventType",
        trigger: Json = "trigger",
        mark_as: Option<String> = "markAs",
        enabled: bool = "enabled",
    }
}
table_row! {
    /// Row of `"AnalyticsAgentResponseEvent"`.
    pub struct AnalyticsAgentResponseEvent in "AnalyticsAgentResponseEvent" {
        event_id: String = "eventId",
        workspace_id: i64 = "workspaceId",
        conversation_id: i64 = "conversationId",
        contact_id: i64 = "contactId",
        admin_id: i64 = "adminId",
        occurred_at: i64 = "occurredAt",
        awaiting_since: i64 = "awaitingSince",
        measured_from: i64 = "measuredFrom",
        response_seconds: i64 = "responseSeconds",
        working_response_seconds: Option<i64> = "workingResponseSeconds",
        is_first_response: bool = "isFirstResponse",
        working_hours_id: Option<i64> = "workingHoursId",
        sla_target_seconds: Option<i64> = "slaTargetSeconds",
        sla_met: Option<bool> = "slaMet",
        channel: Option<String> = "channel",
        inbox_id: Option<i64> = "inboxId",
        metadata: Option<Json> = "metadata",
        inserted_at: i64 = "insertedAt",
    }
}
table_row! {
    /// Row of `"AnalyticsBotMessageEvent"`.
    pub struct AnalyticsBotMessageEvent in "AnalyticsBotMessageEvent" {
        event_id: String = "eventId",
        workspace_id: i64 = "workspaceId",
        message_id: i64 = "messageId",
        conversation_id: i64 = "conversationId",
        occurred_at: i64 = "occurredAt",
        has_response: bool = "hasResponse",
        response_type: Option<String> = "responseType",
        route_type: Option<String> = "routeType",
        result: Option<String> = "result",
        ai_provider: Option<String> = "aiProvider",
        channel: Option<String> = "channel",
        inbox_id: Option<i64> = "inboxId",
        source: Option<String> = "source",
        metadata: Option<Json> = "metadata",
        inserted_at: i64 = "insertedAt",
    }
}
table_row! {
    /// Row of `"AnalyticsContactEvent"`.
    pub struct AnalyticsContactEvent in "AnalyticsContactEvent" {
        event_id: String = "eventId",
        workspace_id: i64 = "workspaceId",
        contact_id: i64 = "contactId",
        event_type: String = "eventType",
        occurred_at: i64 = "occurredAt",
        source: Option<String> = "source",
        source_id: Option<String> = "sourceId",
        channel: Option<String> = "channel",
        inbox_id: Option<i64> = "inboxId",
        language: Option<String> = "language",
        country: Option<String> = "country",
        sender_type: Option<String> = "senderType",
        admin_id: Option<i64> = "adminId",
        metadata: Option<Json> = "metadata",
        inserted_at: i64 = "insertedAt",
    }
}
table_row! {
    /// Row of `"AnalyticsConversationEvent"`.
    pub struct AnalyticsConversationEvent in "AnalyticsConversationEvent" {
        event_id: String = "eventId",
        workspace_id: i64 = "workspaceId",
        conversation_id: i64 = "conversationId",
        event_type: String = "eventType",
        occurred_at: i64 = "occurredAt",
        from_assignee: Option<i64> = "fromAssignee",
        to_assignee: Option<i64> = "toAssignee",
        channel: Option<String> = "channel",
        inbox_id: Option<i64> = "inboxId",
        metadata: Option<Json> = "metadata",
        inserted_at: i64 = "insertedAt",
    }
}
table_row! {
    /// Row of `"AnalyticsEmailTopic"`.
    pub struct AnalyticsEmailTopic in "AnalyticsEmailTopic" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        topic_id: Option<i64> = "topicId",
        topic_reference_id: Option<i64> = "topicReferenceId",
        workspace_id: i64 = "workspaceId",
        contact_id: Option<i64> = "contactId",
        conversation_id: Option<i64> = "conversationId",
        contact_inbox_id: Option<i64> = "contactInboxId",
        email: String = "email",
        token: String = "token",
        delivered_at: Option<i64> = "deliveredAt",
        failed_at: Option<i64> = "failedAt",
        first_seen_at: Option<i64> = "firstSeenAt",
        last_seen_at: Option<i64> = "lastSeenAt",
        seen_count: i64 = "seenCount",
        first_clicked_at: Option<i64> = "firstClickedAt",
        last_clicked_at: Option<i64> = "lastClickedAt",
        click_count: i64 = "clickCount",
    }
}
table_row! {
    /// Row of `"AnalyticsManifestStatus"`.
    pub struct AnalyticsManifestStatus in "AnalyticsManifestStatus" {
        object_key: String = "objectKey",
        status: String = "status",
        attempts: i64 = "attempts",
        ingested_at: Option<i64> = "ingestedAt",
        last_error: Option<String> = "lastError",
    }
}
table_row! {
    /// Row of `"AnalyticsMessageEvent"`.
    pub struct AnalyticsMessageEvent in "AnalyticsMessageEvent" {
        event_id: String = "eventId",
        workspace_id: i64 = "workspaceId",
        contact_id: i64 = "contactId",
        event_type: String = "eventType",
        occurred_at: i64 = "occurredAt",
        sender_type: Option<String> = "senderType",
        admin_id: Option<i64> = "adminId",
        channel: Option<String> = "channel",
        inbox_id: Option<i64> = "inboxId",
        source: Option<String> = "source",
        source_id: Option<String> = "sourceId",
        metadata: Option<Json> = "metadata",
        inserted_at: i64 = "insertedAt",
    }
}
table_row! {
    /// Row of `"Appointment"`.
    pub struct Appointment in "Appointment" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        calendar_id: i64 = "calendarId",
        contact_id: i64 = "contactId",
        conversation_id: Option<i64> = "conversationId",
        contact_inbox_id: Option<i64> = "contactInboxId",
        start_at: i64 = "startAt",
        end_at: i64 = "endAt",
        invitee_timezone: String = "inviteeTimezone",
        status: String = "status",
        location_type: String = "locationType",
        location_detail: Option<String> = "locationDetail",
        external_event_id: Option<String> = "externalEventId",
        external_event_integration_id: Option<i64> = "externalEventIntegrationId",
        external_event_provider_calendar_id: Option<String> = "externalEventProviderCalendarId",
        external_sync_status: Option<String> = "externalSyncStatus",
        cancelled_at: Option<i64> = "cancelledAt",
        deleted_at: Option<i64> = "deletedAt",
    }
}
table_row! {
    /// Row of `"AppointmentCalendar"`.
    pub struct AppointmentCalendar in "AppointmentCalendar" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        description: Option<String> = "description",
        active: bool = "active",
        timezone: String = "timezone",
        duration_minutes: i64 = "durationMinutes",
        buffer_after_minutes: Option<i64> = "bufferAfterMinutes",
        location_type: String = "locationType",
        location_detail: Option<String> = "locationDetail",
        schedule_window_type: String = "scheduleWindowType",
        schedule_window_config: Json = "scheduleWindowConfig",
        max_appointments_per_user: Option<i64> = "maxAppointmentsPerUser",
        daily_limit_enabled: bool = "dailyLimitEnabled",
        max_per_day: Option<i64> = "maxPerDay",
        allow_group_meeting: bool = "allowGroupMeeting",
        max_per_slot: Option<i64> = "maxPerSlot",
        confirmation_message: Option<String> = "confirmationMessage",
        confirmation_flow_id: Option<i64> = "confirmationFlowId",
        cancellation_flow_id: Option<i64> = "cancellationFlowId",
        external_connection_id: Option<i64> = "externalConnectionId",
        external_event_title_template: Option<String> = "externalEventTitleTemplate",
        external_event_description_template: Option<String> = "externalEventDescriptionTemplate",
        external_event_attendees_template: Option<String> = "externalEventAttendeesTemplate",
        public_link_slug: String = "publicLinkSlug",
        deleted_at: Option<i64> = "deletedAt",
    }
}
table_row! {
    /// Row of `"AppointmentCalendarAvailability"`.
    pub struct AppointmentCalendarAvailability in "AppointmentCalendarAvailability" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        calendar_id: i64 = "calendarId",
        weekday: i64 = "weekday",
        start_minute: i64 = "startMinute",
        end_minute: i64 = "endMinute",
    }
}
table_row! {
    /// Row of `"AppointmentCalendarReminder"`.
    pub struct AppointmentCalendarReminder in "AppointmentCalendarReminder" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        calendar_id: i64 = "calendarId",
        flow_id: i64 = "flowId",
        timing_value: i64 = "timingValue",
        timing_unit: String = "timingUnit",
    }
}
table_row! {
    /// Row of `"AppointmentReminderDispatch"`.
    pub struct AppointmentReminderDispatch in "AppointmentReminderDispatch" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        appointment_id: i64 = "appointmentId",
        reminder_config_id: i64 = "reminderConfigId",
        contact_inbox_id: Option<i64> = "contactInboxId",
        send_at: i64 = "sendAt",
        status: String = "status",
        job_id: String = "jobId",
        sent_at: Option<i64> = "sentAt",
        cancelled_at: Option<i64> = "cancelledAt",
        failed_reason: Option<String> = "failedReason",
    }
}
table_row! {
    /// Row of `"Attachment"`.
    pub struct Attachment in "Attachment" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        conversation_id: i64 = "conversationId",
        message_id: i64 = "messageId",
        message_created_at: i64 = "messageCreatedAt",
        file_type: String = "fileType",
        source_id: Option<String> = "sourceId",
        mime_type: String = "mimeType",
        width: Option<i64> = "width",
        height: Option<i64> = "height",
        size: i64 = "size",
        thumbnail_path: Option<String> = "thumbnailPath",
        origin_path: String = "originPath",
        name: Option<String> = "name",
        keep_original: Option<bool> = "keepOriginal",
    }
}
table_row! {
    /// Row of `"AuditLog"`.
    pub struct AuditLog in "AuditLog" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        action: String = "action",
        detail: String = "detail",
        ip_address: Option<String> = "ipAddress",
        user_agent: Option<String> = "userAgent",
        source: Option<String> = "source",
        workspace_id: i64 = "workspaceId",
        user_id: Option<i64> = "userId",
    }
}
table_row! {
    /// Row of `"AutomatedResponse"`.
    pub struct AutomatedResponse in "AutomatedResponse" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        folder_id: Option<i64> = "folderId",
        keywords: Json = "keywords",
        conditions: Option<Json> = "conditions",
        status: bool = "status",
        text: Option<String> = "text",
        flow_id: Option<i64> = "flowId",
        r#type: String = "type",
    }
}
table_row! {
    /// Row of `"AutomationThrottle"`.
    pub struct AutomationThrottle in "AutomationThrottle" {
        workspace_id: i64 = "workspaceId",
        contact_inbox_id: i64 = "contactInboxId",
        throttle_type: String = "throttleType",
        subject_id: i64 = "subjectId",
        last_triggered_at: i64 = "lastTriggeredAt",
        claim_id: String = "claimId",
    }
}
table_row! {
    /// Row of `"BotField"`.
    pub struct BotField in "BotField" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        r#type: String = "type",
        value: Option<String> = "value",
        description: Option<String> = "description",
        folder_id: Option<i64> = "folderId",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"Broadcast"`.
    pub struct Broadcast in "Broadcast" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        workspace_id: i64 = "workspaceId",
        flow_id: Option<i64> = "flowId",
        integration_whatsapp_id: Option<i64> = "integrationWhatsappId",
        integration_messenger_id: Option<i64> = "integrationMessengerId",
        template_id: Option<i64> = "templateId",
        template_data: Option<Json> = "templateData",
        target_mode: String = "targetMode",
        status: String = "status",
        schedules_type: String = "schedulesType",
        schedules_at: i64 = "schedulesAt",
        contact_filter: Option<Json> = "contactFilter",
        subaction: String = "subaction",
        channel: String = "channel",
        contact_count: Option<i64> = "contactCount",
        handoff_completed_at: Option<i64> = "handoffCompletedAt",
        deleted_at: Option<i64> = "deletedAt",
        resume_count: i64 = "resumeCount",
        audience_range_start: Option<i64> = "audienceRangeStart",
        audience_range_end: Option<i64> = "audienceRangeEnd",
        send_rate_per_minute: Option<i64> = "sendRatePerMinute",
        parent_broadcast_id: Option<i64> = "parentBroadcastId",
        occurrence_at: Option<i64> = "occurrenceAt",
    }
}
table_row! {
    /// Row of `"BroadcastRecurrence"`.
    pub struct BroadcastRecurrence in "BroadcastRecurrence" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        broadcast_id: i64 = "broadcastId",
        rule: Json = "rule",
        next_run_at: Option<i64> = "nextRunAt",
        accounted_through_at: Option<i64> = "accountedThroughAt",
        occurrence_count: i64 = "occurrenceCount",
        last_occurrence_at: Option<i64> = "lastOccurrenceAt",
        paused_at: Option<i64> = "pausedAt",
        ended_at: Option<i64> = "endedAt",
    }
}
table_row! {
    /// Row of `"BroadcastRecurrenceOccurrence"`.
    pub struct BroadcastRecurrenceOccurrence in "BroadcastRecurrenceOccurrence" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        recurrence_id: i64 = "recurrenceId",
        local_slot: String = "localSlot",
        occurrence_at: i64 = "occurrenceAt",
        outcome: String = "outcome",
        reason: Option<String> = "reason",
        child_broadcast_id: Option<i64> = "childBroadcastId",
        materialized_at: i64 = "materializedAt",
        superseded_count: i64 = "supersededCount",
    }
}
table_row! {
    /// Row of `"BroadcastTarget"`.
    pub struct BroadcastTarget in "BroadcastTarget" {
        broadcast_id: i64 = "broadcastId",
        inbox_id: i64 = "inboxId",
        flow_id: Option<i64> = "flowId",
        template_id: Option<i64> = "templateId",
        template_data: Option<Json> = "templateData",
    }
}
table_row! {
    /// Row of `"Cart"`.
    pub struct Cart in "Cart" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        contact_id: i64 = "contactId",
        contact_inbox_id: Option<i64> = "contactInboxId",
        items: Json = "items",
        discount_code: Option<String> = "discountCode",
        last_activity_at: i64 = "lastActivityAt",
        abandoned_notified_at: Option<i64> = "abandonedNotifiedAt",
    }
}
table_row! {
    /// Row of `"ChannelPost"`.
    pub struct ChannelPost in "ChannelPost" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
        channel: String = "channel",
        integration_id: i64 = "integrationId",
        source_account_id: String = "sourceAccountId",
        external_post_id: String = "externalPostId",
        caption: Option<String> = "caption",
        media_type: Option<String> = "mediaType",
        thumbnail: Option<String> = "thumbnail",
        permalink: Option<String> = "permalink",
        published_at: Option<i64> = "publishedAt",
        metadata_fetched_at: Option<i64> = "metadataFetchedAt",
        metadata_attempted_at: Option<i64> = "metadataAttemptedAt",
    }
}
table_row! {
    /// Row of `"CoexistSyncRun"`.
    pub struct CoexistSyncRun in "CoexistSyncRun" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        channel: String = "channel",
        r#type: String = "type",
        status: String = "status",
        trigger_source: String = "triggerSource",
        started_at: Option<i64> = "startedAt",
        finished_at: Option<i64> = "finishedAt",
        last_heartbeat_at: Option<i64> = "lastHeartbeatAt",
        total_scan: i64 = "totalScan",
        current_scan: i64 = "currentScan",
        current_step: Option<String> = "currentStep",
        last_synced_at: Option<i64> = "lastSyncedAt",
        current_page_number: i64 = "currentPageNumber",
        imported_contact_count: i64 = "importedContactCount",
        imported_message_count: i64 = "importedMessageCount",
        skipped_count: i64 = "skippedCount",
        failed_count: i64 = "failedCount",
        attempts: i64 = "attempts",
        current_error: Option<String> = "currentError",
        last_phase: Option<i64> = "lastPhase",
        last_chunk_order: Option<i64> = "lastChunkOrder",
        sync_progress: i64 = "syncProgress",
        messenger_sync_phase: String = "messengerSyncPhase",
        pending_patches: Option<Json> = "pendingPatches",
        claim_token: Option<String> = "claimToken",
        scan_from_at: Option<i64> = "scanFromAt",
        requested_by_user_id: Option<i64> = "requestedByUserId",
        resume_cursor: Option<String> = "resumeCursor",
    }
}
table_row! {
    /// Row of `"CommentAutomation"`.
    pub struct CommentAutomation in "CommentAutomation" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        workspace_id: i64 = "workspaceId",
        folder_id: Option<i64> = "folderId",
        r#type: String = "type",
        inbox_id: Option<i64> = "inboxId",
        is_active: bool = "isActive",
        start_time: Option<String> = "startTime",
        end_time: Option<String> = "endTime",
        replies_count: i64 = "repliesCount",
        sent_count: i64 = "sentCount",
        delivered_count: i64 = "deliveredCount",
        seen_count: i64 = "seenCount",
        clicked_count: i64 = "clickedCount",
        failed_count: i64 = "failedCount",
        missed_count: i64 = "missedCount",
        moderated_count: i64 = "moderatedCount",
        post: Json = "post",
        private_reply: Json = "privateReply",
        public_reply: Json = "publicReply",
        include_keywords: Json = "includeKeywords",
        exclude_keywords: Json = "excludeKeywords",
        exclude_keywords_type: String = "excludeKeywordsType",
        options: Json = "options",
        hide_comments: Json = "hideComments",
        reply_after: Json = "replyAfter",
    }
}
table_row! {
    /// Row of `"CommentAutomationEvent"`.
    pub struct CommentAutomationEvent in "CommentAutomationEvent" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        automation_id: i64 = "automationId",
        contact_id: Option<i64> = "contactId",
        contact_inbox_id: Option<i64> = "contactInboxId",
        post_id: String = "postId",
        comment_id: String = "commentId",
        comment_text: Option<String> = "commentText",
        reply_channel: String = "replyChannel",
        reply_type: String = "replyType",
        reply_text: Option<String> = "replyText",
        status: String = "status",
        delivered_at: Option<i64> = "deliveredAt",
        seen_at: Option<i64> = "seenAt",
        clicked_at: Option<i64> = "clickedAt",
        failed_at: Option<i64> = "failedAt",
        error_detail: Option<String> = "errorDetail",
        http_code: Option<String> = "httpCode",
        occurred_at: i64 = "occurredAt",
    }
}
table_row! {
    /// Row of `"CommentAutomationMiss"`.
    pub struct CommentAutomationMiss in "CommentAutomationMiss" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        automation_id: i64 = "automationId",
        contact_id: Option<i64> = "contactId",
        contact_inbox_id: Option<i64> = "contactInboxId",
        post_id: String = "postId",
        comment_id: String = "commentId",
        comment_text: Option<String> = "commentText",
        reason: String = "reason",
        occurred_at: i64 = "occurredAt",
    }
}
table_row! {
    /// Row of `"CommentAutomationReply"`.
    pub struct CommentAutomationReply in "CommentAutomationReply" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        automation_id: i64 = "automationId",
        contact_id: i64 = "contactId",
        post_id: String = "postId",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"CommentModerationLog"`.
    pub struct CommentModerationLog in "CommentModerationLog" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        automation_id: i64 = "automationId",
        contact_id: Option<i64> = "contactId",
        contact_inbox_id: Option<i64> = "contactInboxId",
        post_id: String = "postId",
        comment_id: String = "commentId",
        comment_text: Option<String> = "commentText",
        source: String = "source",
        rule: Option<String> = "rule",
        matched_keyword: Option<String> = "matchedKeyword",
        categories: Json = "categories",
        reason: Option<String> = "reason",
        action: String = "action",
        status: String = "status",
        error_detail: Option<String> = "errorDetail",
        provider: Option<String> = "provider",
        model_id: Option<String> = "modelId",
        occurred_at: i64 = "occurredAt",
    }
}
table_row! {
    /// Row of `"Condition"`.
    pub struct Condition in "Condition" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        trigger_id: Option<i64> = "triggerId",
        webhook_id: Option<i64> = "webhookId",
        r#type: String = "type",
        source_id: Option<String> = "sourceId",
        operator: Option<String> = "operator",
        value: Option<Json> = "value",
    }
}
table_row! {
    /// Row of `"ConnectSession"`.
    pub struct ConnectSession in "ConnectSession" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        provider: String = "provider",
        purpose: String = "purpose",
        target_connection_id: Option<i64> = "targetConnectionId",
        actor_user_id: Option<i64> = "actorUserId",
        actor_token_id: Option<i64> = "actorTokenId",
        platform_owner_id: Option<String> = "platformOwnerId",
        origin_host: Option<String> = "originHost",
        return_url: Option<String> = "returnUrl",
        state_nonce_hash: String = "stateNonceHash",
        status: String = "status",
        step: String = "step",
        next_action: Option<Json> = "nextAction",
        encrypted_auth: Option<Json> = "encryptedAuth",
        targets: Json = "targets",
        target_claims: Json = "targetClaims",
        result_connection_ids: Json = "resultConnectionIds",
        results: Json = "results",
        error_code: Option<String> = "errorCode",
        expires_at: i64 = "expiresAt",
        consumed_at: Option<i64> = "consumedAt",
    }
}
table_row! {
    /// Row of `"Connection"`.
    pub struct Connection in "Connection" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        provider: String = "provider",
        kind: String = "kind",
        channel: Option<String> = "channel",
        inbox_id: Option<i64> = "inboxId",
        integration_id: Option<i64> = "integrationId",
        source_id: String = "sourceId",
        display_name: String = "displayName",
        status: String = "status",
        status_reason: Option<String> = "statusReason",
        last_error: Option<String> = "lastError",
        auth_expires_at: Option<i64> = "authExpiresAt",
        created_by: Option<i64> = "createdBy",
        connected_at: Option<i64> = "connectedAt",
        disconnected_at: Option<i64> = "disconnectedAt",
        provider_event_at: Option<i64> = "providerEventAt",
        provider_event_kind: Option<String> = "providerEventKind",
    }
}
table_row! {
    /// Row of `"Contact"`.
    pub struct Contact in "Contact" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        avatar: Option<String> = "avatar",
        phone_number: Option<String> = "phoneNumber",
        email: Option<String> = "email",
        email_verified: bool = "emailVerified",
        email_opt_in: bool = "emailOptIn",
        first_name: Option<String> = "firstName",
        last_name: Option<String> = "lastName",
        full_name: Option<String> = "fullName",
        gender: Option<String> = "gender",
        last_read_at: Option<i64> = "lastReadAt",
        r#ref: Option<String> = "ref",
        country: Option<String> = "country",
        state: Option<String> = "state",
        city: Option<String> = "city",
        location: Option<Json> = "location",
        locale: Option<String> = "locale",
        timezone: Option<String> = "timezone",
        subscribed_at: Option<i64> = "subscribedAt",
        broadcast_subscribed_at: Option<i64> = "broadcastSubscribedAt",
        blocked_at: Option<i64> = "blockedAt",
        total_tagged: i64 = "totalTagged",
        total_new_tagged: i64 = "totalNewTagged",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"ContactActiveHourly"`.
    pub struct ContactActiveHourly in "ContactActiveHourly" {
        workspace_id: i64 = "workspaceId",
        contact_id: i64 = "contactId",
        contact_inbox_id: i64 = "contactInboxId",
        hour_bucket: i64 = "hourBucket",
        inbox_id: i64 = "inboxId",
    }
}
table_row! {
    /// Row of `"ContactActiveMonthly"`.
    pub struct ContactActiveMonthly in "ContactActiveMonthly" {
        workspace_id: i64 = "workspaceId",
        contact_id: i64 = "contactId",
        contact_inbox_id: i64 = "contactInboxId",
        period_start: i64 = "periodStart",
        inbox_id: i64 = "inboxId",
        workspace_mac_id: i64 = "workspaceMacId",
    }
}
table_row! {
    /// Row of `"ContactActivity"`.
    pub struct ContactActivity in "ContactActivity" {
        workspace_id: i64 = "workspaceId",
        contact_id: i64 = "contactId",
        activity_type: String = "activityType",
        reference_type: String = "referenceType",
        parent_reference_id: i64 = "parentReferenceId",
        reference_id: i64 = "referenceId",
        first_occurred_at: i64 = "firstOccurredAt",
    }
}
table_row! {
    /// Row of `"ContactCustomField"`.
    pub struct ContactCustomField in "ContactCustomField" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        value: String = "value",
        contact_id: i64 = "contactId",
        custom_field_id: i64 = "customFieldId",
    }
}
table_row! {
    /// Row of `"ContactInbox"`.
    pub struct ContactInbox in "ContactInbox" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        original_contact_id: i64 = "originalContactId",
        contact_id: i64 = "contactId",
        inbox_id: i64 = "inboxId",
        channel: String = "channel",
        source: String = "source",
        source_id: String = "sourceId",
        follows_business: Option<bool> = "followsBusiness",
        business_follows_contact: Option<bool> = "businessFollowsContact",
        account_verified: Option<bool> = "accountVerified",
        follower_count: Option<i64> = "followerCount",
        profile_snapshot_state: Option<String> = "profileSnapshotState",
        profile_snapshot_attempts: Option<i64> = "profileSnapshotAttempts",
        profile_snapshot_next_attempt_at: Option<i64> = "profileSnapshotNextAttemptAt",
        language: Option<String> = "language",
        persona_id: Option<String> = "personaId",
        contact_last_read_at: Option<i64> = "contactLastReadAt",
        first_interaction_at: Option<i64> = "firstInteractionAt",
        last_message_at: Option<i64> = "lastMessageAt",
        last_incoming_message_at: Option<i64> = "lastIncomingMessageAt",
        last_outbound_message_at: Option<i64> = "lastOutboundMessageAt",
        referral: Option<Json> = "referral",
        last_comment_message_id: Option<String> = "lastCommentMessageId",
        last_comment_message_at: Option<i64> = "lastCommentMessageAt",
        last_story_id: Option<String> = "lastStoryId",
        consecutive_failed_reply: i64 = "consecutiveFailedReply",
        last_input_failure: Option<String> = "lastInputFailure",
        last_error_log: Option<String> = "lastErrorLog",
        last_btn_title: Option<String> = "lastBtnTitle",
        last_user_input: Option<String> = "lastUserInput",
        last_user_input_type: Option<String> = "lastUserInputType",
        webchat_parent_url: Option<String> = "webchatParentUrl",
        source_user_id: Option<String> = "sourceUserId",
        source_parent_user_id: Option<String> = "sourceParentUserId",
        source_username: Option<String> = "sourceUsername",
        source_identity_history: Option<Json> = "sourceIdentityHistory",
        thread_control_state: Option<String> = "threadControlState",
        thread_owner_role: Option<String> = "threadOwnerRole",
        thread_control_updated_at: Option<i64> = "threadControlUpdatedAt",
        thread_control_last_event: Option<String> = "threadControlLastEvent",
        thread_owner_app_id: Option<String> = "threadOwnerAppId",
        thread_owner_expires_at: Option<i64> = "threadOwnerExpiresAt",
        thread_previous_owner_app_id: Option<String> = "threadPreviousOwnerAppId",
    }
}
table_row! {
    /// Row of `"ContactInboxPost"`.
    pub struct ContactInboxPost in "ContactInboxPost" {
        workspace_id: i64 = "workspaceId",
        contact_inbox_id: i64 = "contactInboxId",
        post_id: i64 = "postId",
        commented_at: i64 = "commentedAt",
    }
}
table_row! {
    /// Row of `"ContactKeywordMessage"`.
    pub struct ContactKeywordMessage in "ContactKeywordMessage" {
        workspace_id: i64 = "workspaceId",
        contact_id: i64 = "contactId",
        message_id: i64 = "messageId",
        automated_response_id: i64 = "automatedResponseId",
        text: String = "text",
        normalized_text: String = "normalizedText",
        occurred_at: i64 = "occurredAt",
    }
}
table_row! {
    /// Row of `"ContactNote"`.
    pub struct ContactNote in "ContactNote" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        text: String = "text",
        contact_id: i64 = "contactId",
        created_by_id: Option<i64> = "createdById",
    }
}
table_row! {
    /// Row of `"ContactOnBroadcast"`.
    pub struct ContactOnBroadcast in "ContactOnBroadcast" {
        broadcast_id: i64 = "broadcastId",
        contact_id: i64 = "contactId",
        contact_inbox_id: i64 = "contactInboxId",
        conversation_id: i64 = "conversationId",
        sent: bool = "sent",
        seen_at: Option<i64> = "seenAt",
        delivered_at: Option<i64> = "deliveredAt",
        clicked_at: Option<i64> = "clickedAt",
        failed_at: Option<i64> = "failedAt",
        error_content: Option<String> = "errorContent",
        is_read: Option<i64> = "isRead",
    }
}
table_row! {
    /// Row of `"ContactOnSequence"`.
    pub struct ContactOnSequence in "ContactOnSequence" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        enrolled_at: i64 = "enrolledAt",
        completed_at: Option<i64> = "completedAt",
        current_step: i64 = "currentStep",
        status: Option<String> = "status",
        next_run_at: Option<i64> = "nextRunAt",
        last_step_id: Option<i64> = "lastStepId",
        next_step_id: Option<i64> = "nextStepId",
        locked_at: Option<i64> = "lockedAt",
        lock_owner: Option<String> = "lockOwner",
        last_error: Option<String> = "lastError",
        contact_id: i64 = "contactId",
        sequence_id: i64 = "sequenceId",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"ContactOnSmartDelay"`.
    pub struct ContactOnSmartDelay in "ContactOnSmartDelay" {
        id: i64 = "id",
        workspace_id: i64 = "workspaceId",
        flow_id: i64 = "flowId",
        flow_version_id: Option<i64> = "flowVersionId",
        contact_inbox_id: i64 = "contactInboxId",
        conversation_id: i64 = "conversationId",
        node_id: Option<String> = "nodeId",
        step_id: Option<String> = "stepId",
        metadata: Option<Json> = "metadata",
        r#type: String = "type",
        created_at: i64 = "createdAt",
        trigger_at: i64 = "triggerAt",
        status: String = "status",
        appointment_id: Option<i64> = "appointmentId",
    }
}
table_row! {
    /// Row of `"ContactReferralEvent"`.
    pub struct ContactReferralEvent in "ContactReferralEvent" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        contact_id: i64 = "contactId",
        contact_inbox_id: i64 = "contactInboxId",
        channel: String = "channel",
        r#type: String = "type",
        provider: Option<String> = "provider",
        occurred_at: i64 = "occurredAt",
        ad_id: Option<String> = "adId",
        campaign_id: Option<String> = "campaignId",
        ad_group_id: Option<String> = "adGroupId",
        click_id_type: Option<String> = "clickIdType",
        click_id: Option<String> = "clickId",
        r#ref: Option<String> = "ref",
        source: Option<String> = "source",
        source_url: Option<String> = "sourceUrl",
        metadata: Option<Json> = "metadata",
        dedup_key: String = "dedupKey",
    }
}
table_row! {
    /// Row of `"ContactState"`.
    pub struct ContactState in "ContactState" {
        workspace_id: i64 = "workspaceId",
        contact_id: i64 = "contactId",
        last_delivered_at: Option<i64> = "lastDeliveredAt",
    }
}
table_row! {
    /// Row of `"ContactToTag"`.
    pub struct ContactToTag in "ContactToTag" {
        contact_id: i64 = "contactId",
        tag_id: i64 = "tagId",
    }
}
table_row! {
    /// Row of `"ContactToTagChannel"`.
    pub struct ContactToTagChannel in "ContactToTagChannel" {
        tag_id: i64 = "tagId",
        tag_channel_id: i64 = "tagChannelId",
        contact_inbox_id: i64 = "contactInboxId",
        created_at: i64 = "createdAt",
    }
}
table_row! {
    /// Row of `"Conversation"`.
    pub struct Conversation in "Conversation" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        bot_enabled: bool = "botEnabled",
        bot_resume_at: Option<i64> = "botResumeAt",
        archived_at: Option<i64> = "archivedAt",
        additional_attributes: Option<Json> = "additionalAttributes",
        contact_last_read_at: Option<i64> = "contactLastReadAt",
        agent_last_read_at: Option<i64> = "agentLastReadAt",
        ai_context_last_message_id: Option<i64> = "aiContextLastMessageId",
        last_activity_at: Option<i64> = "lastActivityAt",
        followed: bool = "followed",
        assigned_user_id: Option<i64> = "assignedUserId",
        assigned_inbox_team_id: Option<i64> = "assignedInboxTeamId",
        workspace_id: i64 = "workspaceId",
        contact_id: i64 = "contactId",
        source_id: Option<String> = "sourceId",
        last_step: Option<String> = "lastStep",
        current_step: Option<String> = "currentStep",
        admin_replied_at: Option<i64> = "adminRepliedAt",
        contact_replied_at: Option<i64> = "contactRepliedAt",
        awaiting_human_reply_since: Option<i64> = "awaitingHumanReplySince",
        awaiting_human_reply_closed_at: Option<i64> = "awaitingHumanReplyClosedAt",
        assigned_user_at: Option<i64> = "assignedUserAt",
        response_due_at: Option<i64> = "responseDueAt",
        response_sla_breaching_at: Option<i64> = "responseSlaBreachingAt",
        response_sla_target_seconds: Option<i64> = "responseSlaTargetSeconds",
    }
}
table_row! {
    /// Row of `"ConversationParticipant"`.
    pub struct ConversationParticipant in "ConversationParticipant" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        conversation_id: i64 = "conversationId",
        user_id: i64 = "userId",
    }
}
table_row! {
    /// Row of `"Coupon"`.
    pub struct Coupon in "Coupon" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        topic_id: i64 = "topicId",
        code: String = "code",
        issued_contact_id: Option<i64> = "issuedContactId",
        issued_at: Option<i64> = "issuedAt",
        used_at: Option<i64> = "usedAt",
    }
}
table_row! {
    /// Row of `"CouponTopic"`.
    pub struct CouponTopic in "CouponTopic" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        description: Option<String> = "description",
        expires_at: Option<i64> = "expiresAt",
        status: String = "status",
        deleted_at: Option<i64> = "deletedAt",
        has_ever_had_coupon: bool = "hasEverHadCoupon",
        created_by_id: Option<i64> = "createdById",
    }
}
table_row! {
    /// Row of `"CustomDomain"`.
    pub struct CustomDomain in "CustomDomain" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        tenant_id: i64 = "tenantId",
        domain: String = "domain",
        status: String = "status",
        verified_at: Option<i64> = "verifiedAt",
        cf_hostname_id: Option<String> = "cfHostnameId",
        cf_ownership_value: Option<String> = "cfOwnershipValue",
        cf_acme_value: Option<String> = "cfAcmeValue",
    }
}
table_row! {
    /// Row of `"CustomField"`.
    pub struct CustomField in "CustomField" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        r#type: String = "type",
        description: Option<String> = "description",
        folder_id: Option<i64> = "folderId",
        show_in_inbox: bool = "showInInbox",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"DecisionConnection"`.
    pub struct DecisionConnection in "DecisionConnection" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        provider_kind: String = "providerKind",
        endpoint: Option<String> = "endpoint",
        credential: Json = "credential",
        model_catalog: Json = "modelCatalog",
        default_model: Option<String> = "defaultModel",
        status: String = "status",
        last_test_status: Option<String> = "lastTestStatus",
        last_tested_at: Option<i64> = "lastTestedAt",
    }
}
table_row! {
    /// Row of `"DecisionProfile"`.
    pub struct DecisionProfile in "DecisionProfile" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        description: Option<String> = "description",
        status: String = "status",
        connection_id: i64 = "connectionId",
        model: String = "model",
        contract: Json = "contract",
        threshold_config: Option<Json> = "thresholdConfig",
        global_router_enabled: bool = "globalRouterEnabled",
        global_router_config: Option<Json> = "globalRouterConfig",
    }
}
table_row! {
    /// Row of `"DiscountCode"`.
    pub struct DiscountCode in "DiscountCode" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        code: String = "code",
        auto_apply: bool = "autoApply",
        discount_type: String = "discountType",
        discount_value: i64 = "discountValue",
        applies_to: String = "appliesTo",
        category_id: Option<i64> = "categoryId",
        product_ids: Json = "productIds",
        minimum_type: String = "minimumType",
        minimum_value: i64 = "minimumValue",
        usage_limit: Option<i64> = "usageLimit",
        used_count: i64 = "usedCount",
        once_per_customer: bool = "oncePerCustomer",
        eligibility: String = "eligibility",
        customer_filter: Option<Json> = "customerFilter",
        starts_at: i64 = "startsAt",
        ends_at: Option<i64> = "endsAt",
        is_active: bool = "isActive",
        source: String = "source",
        external_id: Option<String> = "externalId",
    }
}
table_row! {
    /// Row of `"DiscountCodeRedemption"`.
    pub struct DiscountCodeRedemption in "DiscountCodeRedemption" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        discount_code_id: i64 = "discountCodeId",
        order_id: i64 = "orderId",
        contact_id: Option<i64> = "contactId",
        amount: i64 = "amount",
    }
}
table_row! {
    /// Row of `"DynamicImage"`.
    pub struct DynamicImage in "DynamicImage" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        workspace_id: i64 = "workspaceId",
        custom_field_id: Option<i64> = "customFieldId",
        data: Json = "data",
        background_url: Option<String> = "backgroundUrl",
        enabled: bool = "enabled",
    }
}
table_row! {
    /// Row of `"EcommerceOpenHours"`.
    pub struct EcommerceOpenHours in "EcommerceOpenHours" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        ranges: Json = "ranges",
    }
}
table_row! {
    /// Row of `"EcommerceOrderCounter"`.
    pub struct EcommerceOrderCounter in "EcommerceOrderCounter" {
        workspace_id: i64 = "workspaceId",
        last_number: i64 = "lastNumber",
    }
}
table_row! {
    /// Row of `"EcommercePaymentProvider"`.
    pub struct EcommercePaymentProvider in "EcommercePaymentProvider" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        provider: String = "provider",
        livemode: bool = "livemode",
        account_label: Option<String> = "accountLabel",
        external_account_id: Option<String> = "externalAccountId",
        platform_credential_id: Option<i64> = "platformCredentialId",
        secret: Option<Json> = "secret",
        webhook_endpoint_id: Option<String> = "webhookEndpointId",
    }
}
table_row! {
    /// Row of `"EcommerceSettings"`.
    pub struct EcommerceSettings in "EcommerceSettings" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        store_source: String = "storeSource",
        ordering_available: bool = "orderingAvailable",
        webshop_product_view: String = "webshopProductView",
        open_hours_id: Option<i64> = "openHoursId",
        search_products: bool = "searchProducts",
        facebook_pixel_id: Option<String> = "facebookPixelId",
        abandoned_cart_delay_minutes: Option<i64> = "abandonedCartDelayMinutes",
        abandoned_cart_flow_id: Option<i64> = "abandonedCartFlowId",
        product_buttons: Json = "productButtons",
        product_image_size: String = "productImageSize",
        allow_rate_orders: bool = "allowRateOrders",
        notify_on_cart_status_change: bool = "notifyOnCartStatusChange",
        payment_methods: Json = "paymentMethods",
        currency: String = "currency",
        currency_position: String = "currencyPosition",
        currency_symbol: Option<String> = "currencySymbol",
        service_fee_percent: i64 = "serviceFeePercent",
        shipping_type: String = "shippingType",
        store_address: Option<String> = "storeAddress",
        shipping_countries: Json = "shippingCountries",
        default_shipping_country: Option<String> = "defaultShippingCountry",
        delivery_cost: i64 = "deliveryCost",
        checkout_mode: String = "checkoutMode",
        checkout_link: Option<String> = "checkoutLink",
        checkout_flow_id: Option<i64> = "checkoutFlowId",
        minimum_order_value: i64 = "minimumOrderValue",
        order_confirmation_mode: String = "orderConfirmationMode",
        order_confirmation_flow_id: Option<i64> = "orderConfirmationFlowId",
        require_name: bool = "requireName",
        require_email: bool = "requireEmail",
        require_phone: bool = "requirePhone",
        require_state: bool = "requireState",
        require_zip: bool = "requireZip",
        external_checkout_method: String = "externalCheckoutMethod",
        external_auto_apply_discounts: bool = "externalAutoApplyDiscounts",
    }
}
table_row! {
    /// Row of `"EmailTopic"`.
    pub struct EmailTopic in "EmailTopic" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        workspace_id: i64 = "workspaceId",
        folder_id: Option<i64> = "folderId",
        sends_total: i64 = "sendsTotal",
        delivereds_total: i64 = "deliveredsTotal",
        seens_total: i64 = "seensTotal",
        clicks_total: i64 = "clicksTotal",
    }
}
table_row! {
    /// Row of `"ErrorLog"`.
    pub struct ErrorLog in "ErrorLog" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        action: String = "action",
        detail: String = "detail",
        http_code: Option<String> = "httpCode",
        workspace_id: i64 = "workspaceId",
        contact_id: Option<i64> = "contactId",
        source_id: Option<String> = "sourceId",
        stack_trace: Option<String> = "stackTrace",
    }
}
table_row! {
    /// Row of `"ExternalWebhook"`.
    pub struct ExternalWebhook in "ExternalWebhook" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        provider: String = "provider",
        event: String = "event",
        url: String = "url",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"FacebookLeadAdsAutomation"`.
    pub struct FacebookLeadAdsAutomation in "FacebookLeadAdsAutomation" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        workspace_id: i64 = "workspaceId",
        page_id: String = "pageId",
        page_name: Option<String> = "pageName",
        form_id: String = "formId",
        form_name: Option<String> = "formName",
        field_mapping: Json = "fieldMapping",
        leads_handled_count: i64 = "leadsHandledCount",
    }
}
table_row! {
    /// Row of `"FacebookLeadAdsLead"`.
    pub struct FacebookLeadAdsLead in "FacebookLeadAdsLead" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        automation_id: i64 = "automationId",
        leadgen_id: String = "leadgenId",
        form_id: Option<String> = "formId",
        form_name: Option<String> = "formName",
        adgroup_id: Option<String> = "adgroupId",
        contact_id: Option<i64> = "contactId",
    }
}
table_row! {
    /// Row of `"FacebookMarketingMessage"`.
    pub struct FacebookMarketingMessage in "FacebookMarketingMessage" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        page_id: String = "pageId",
        ad_account_id: String = "adAccountId",
        currency: String = "currency",
        currency_offset: i64 = "currencyOffset",
        budget_type: String = "budgetType",
        budget_minor_units: i64 = "budgetMinorUnits",
        content: Json = "content",
        target_type: String = "targetType",
        audience_id: Option<i64> = "audienceId",
        campaign_id: String = "campaignId",
        facebook_user_id: Option<String> = "facebookUserId",
        status: String = "status",
        status_reason: Option<String> = "statusReason",
        status_error_code: Option<i64> = "statusErrorCode",
        status_error_message: Option<String> = "statusErrorMessage",
        starts_at: Option<i64> = "startsAt",
        started_at: Option<i64> = "startedAt",
        first_sent_at: Option<i64> = "firstSentAt",
        completed_at: Option<i64> = "completedAt",
        next_day_at: Option<i64> = "nextDayAt",
        meta_ends_at: Option<i64> = "metaEndsAt",
        timezone_name: Option<String> = "timezoneName",
        ad_set_id: Option<String> = "adSetId",
        token_count: Option<i64> = "tokenCount",
        day_target: Option<i64> = "dayTarget",
        day_sent: i64 = "daySent",
        resume_count: i64 = "resumeCount",
    }
}
table_row! {
    /// Row of `"FacebookMarketingMessageAudience"`.
    pub struct FacebookMarketingMessageAudience in "FacebookMarketingMessageAudience" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        custom_audience_id: String = "customAudienceId",
        ad_account_id: String = "adAccountId",
        page_id: String = "pageId",
        name: String = "name",
        description: Option<String> = "description",
        rows_uploaded: i64 = "rowsUploaded",
        ready_at: Option<i64> = "readyAt",
        facebook_user_id: Option<String> = "facebookUserId",
    }
}
table_row! {
    /// Row of `"FacebookMarketingMessageRecipient"`.
    pub struct FacebookMarketingMessageRecipient in "FacebookMarketingMessageRecipient" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        campaign_row_id: i64 = "campaignRowId",
        subscription_token: String = "subscriptionToken",
        psid: Option<String> = "psid",
        mid: Option<String> = "mid",
        tracking_id: Option<String> = "trackingId",
        status: String = "status",
        attempts: i64 = "attempts",
        next_attempt_at: Option<i64> = "nextAttemptAt",
        next_eligible_at: Option<i64> = "nextEligibleAt",
        sent_at: Option<i64> = "sentAt",
        delivered_at: Option<i64> = "deliveredAt",
        failed_at: Option<i64> = "failedAt",
        error_code: Option<i64> = "errorCode",
        error_message: Option<String> = "errorMessage",
        error_source: Option<String> = "errorSource",
    }
}
table_row! {
    /// Row of `"FacebookMarketingMessagesAuth"`.
    pub struct FacebookMarketingMessagesAuth in "FacebookMarketingMessagesAuth" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        auth: Json = "auth",
        token_expires_at: Option<i64> = "tokenExpiresAt",
        status: String = "status",
        facebook_user_id: Option<String> = "facebookUserId",
    }
}
table_row! {
    /// Row of `"File"`.
    pub struct File in "File" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: Option<i64> = "workspaceId",
        user_id: Option<i64> = "userId",
        context_type: String = "contextType",
        sub_type: Option<String> = "subType",
        path: String = "path",
        file_name: String = "fileName",
        mime_type: String = "mimeType",
        file_size: Option<i64> = "fileSize",
        status: String = "status",
        meta: Option<Json> = "meta",
        uploaded_at: Option<i64> = "uploadedAt",
    }
}
table_row! {
    /// Row of `"Flow"`.
    pub struct Flow in "Flow" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        active: bool = "active",
        enable_in_inbox: bool = "enableInInbox",
        current_version_id: Option<i64> = "currentVersionId",
        draft_version_id: Option<i64> = "draftVersionId",
        workspace_id: i64 = "workspaceId",
        folder_id: Option<i64> = "folderId",
    }
}
table_row! {
    /// Row of `"FlowAnalyticsSession"`.
    pub struct FlowAnalyticsSession in "FlowAnalyticsSession" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        flow_id: i64 = "flowId",
        workspace_id: i64 = "workspaceId",
        deleted_at: Option<i64> = "deletedAt",
    }
}
table_row! {
    /// Row of `"FlowNodeStat"`.
    pub struct FlowNodeStat in "FlowNodeStat" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        flow_id: i64 = "flowId",
        analytics_id: i64 = "analyticsId",
        node_id: String = "nodeId",
        button_id: Option<String> = "buttonId",
        contact_id: i64 = "contactId",
        contact_inbox_id: i64 = "contactInboxId",
        event_type: String = "eventType",
        error_content: Option<String> = "errorContent",
        occurred_at: Option<i64> = "occurredAt",
        seen_at: Option<i64> = "seenAt",
        ref_id: Option<String> = "refId",
        ref_type: Option<i64> = "refType",
    }
}
table_row! {
    /// Row of `"FlowRun"`.
    pub struct FlowRun in "FlowRun" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        flow_id: i64 = "flowId",
        flow_version_id: i64 = "flowVersionId",
        conversation_id: Option<i64> = "conversationId",
    }
}
table_row! {
    /// Row of `"FlowVersion"`.
    pub struct FlowVersion in "FlowVersion" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        flow_id: i64 = "flowId",
        nodes: Json = "nodes",
        edges: Json = "edges",
        is_draft: bool = "isDraft",
        is_latest: bool = "isLatest",
        start_node_id: i64 = "startNodeId",
    }
}
table_row! {
    /// Row of `"Folder"`.
    pub struct Folder in "Folder" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        folder_type: String = "folderType",
        parent_id: Option<i64> = "parentId",
        workspace_id: i64 = "workspaceId",
        is_trash: bool = "isTrash",
        paths: Json = "paths",
    }
}
table_row! {
    /// Row of `"GoogleAdsConversionEvent"`.
    pub struct GoogleAdsConversionEvent in "GoogleAdsConversionEvent" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_google_ads_id: Option<i64> = "integrationGoogleAdsId",
        contact_inbox_id: Option<i64> = "contactInboxId",
        referral_event_id: Option<i64> = "referralEventId",
        customer_id: String = "customerId",
        login_customer_id: Option<String> = "loginCustomerId",
        conversion_customer_id: String = "conversionCustomerId",
        conversion_action_id: String = "conversionActionId",
        conversion_action_name: Option<String> = "conversionActionName",
        conversion_action_category: String = "conversionActionCategory",
        lookback_window_days: Option<i64> = "lookbackWindowDays",
        channel: String = "channel",
        source: String = "source",
        scope_id: String = "scopeId",
        click_id_type: String = "clickIdType",
        click_id: String = "clickId",
        google_click_received_at: i64 = "googleClickReceivedAt",
        occurred_at: i64 = "occurredAt",
        value: Option<String> = "value",
        currency: Option<String> = "currency",
        transaction_id: String = "transactionId",
        options: Option<Json> = "options",
        upload_method: String = "uploadMethod",
        status: String = "status",
        attempt: i64 = "attempt",
        claim_token: Option<String> = "claimToken",
        claimed_at: Option<i64> = "claimedAt",
        request_id: Option<String> = "requestId",
        sent_at: Option<i64> = "sentAt",
        error: Option<String> = "error",
        failure_stage: Option<String> = "failureStage",
        processing_status: Option<String> = "processingStatus",
        processing_checked_at: Option<i64> = "processingCheckedAt",
        processing_attempts: i64 = "processingAttempts",
        next_processing_check_at: Option<i64> = "nextProcessingCheckAt",
        processing_detail: Option<Json> = "processingDetail",
    }
}
table_row! {
    /// Row of `"GoogleAdsSettings"`.
    pub struct GoogleAdsSettings in "GoogleAdsSettings" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        settings: Json = "settings",
    }
}
table_row! {
    /// Row of `"IgStoryAutomation"`.
    pub struct IgStoryAutomation in "IgStoryAutomation" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        workspace_id: i64 = "workspaceId",
        folder_id: Option<i64> = "folderId",
        r#type: String = "type",
        trigger: String = "trigger",
        inbox_id: Option<i64> = "inboxId",
        is_active: bool = "isActive",
        start_time: Option<String> = "startTime",
        end_time: Option<String> = "endTime",
        replies_count: i64 = "repliesCount",
        story: Json = "story",
        reply: Json = "reply",
        include_keywords: Json = "includeKeywords",
    }
}
table_row! {
    /// Row of `"Import"`.
    pub struct Import in "Import" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        inbox_id: Option<i64> = "inboxId",
        user_id: Option<i64> = "userId",
        file_id: i64 = "fileId",
        r#type: String = "type",
        format: String = "format",
        status: String = "status",
        meta: Json = "meta",
        total_count: i64 = "totalCount",
        processed_count: i64 = "processedCount",
        success_count: i64 = "successCount",
        failed_count: i64 = "failedCount",
        error_message: Option<String> = "errorMessage",
        error_sample: Json = "errorSample",
        completed_at: Option<i64> = "completedAt",
    }
}
table_row! {
    /// Row of `"InboundWebhook"`.
    pub struct InboundWebhook in "InboundWebhook" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        active: bool = "active",
        token: String = "token",
        inbox_resolution: Option<Json> = "inboxResolution",
        identifiers: Json = "identifiers",
        field_mappings: Json = "fieldMappings",
        actions: Json = "actions",
        sample_json: Option<Json> = "sampleJson",
        request_count: i64 = "requestCount",
    }
}
table_row! {
    /// Row of `"InboundWebhookRequest"`.
    pub struct InboundWebhookRequest in "InboundWebhookRequest" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        inbound_webhook_id: i64 = "inboundWebhookId",
        idempotency_key: Option<String> = "idempotencyKey",
        client_ip: Option<String> = "clientIp",
        user_agent: Option<String> = "userAgent",
        status: String = "status",
        claim_token: Option<String> = "claimToken",
        claimed_at: Option<i64> = "claimedAt",
        attempts: i64 = "attempts",
        skip_reason: Option<String> = "skipReason",
        error_code: Option<String> = "errorCode",
        error_message: Option<String> = "errorMessage",
        inbox_id: Option<i64> = "inboxId",
        contact_id: Option<i64> = "contactId",
        contact_inbox_id: Option<i64> = "contactInboxId",
        payload: Json = "payload",
        field_errors: Option<Json> = "fieldErrors",
        action_results: Option<Json> = "actionResults",
        processed_at: Option<i64> = "processedAt",
    }
}
table_row! {
    /// Row of `"Inbox"`.
    pub struct Inbox in "Inbox" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        channel: String = "channel",
        source_id: String = "sourceId",
        workspace_id: i64 = "workspaceId",
        status: String = "status",
        mark_read_on_outbound: bool = "markReadOnOutbound",
        disconnected_at: Option<i64> = "disconnectedAt",
        disconnect_reason: Option<String> = "disconnectReason",
        thread_control_seen_at: Option<i64> = "threadControlSeenAt",
    }
}
table_row! {
    /// Row of `"InboxContactStat"`.
    pub struct InboxContactStat in "InboxContactStat" {
        inbox_id: i64 = "inboxId",
        total_contacts: i64 = "totalContacts",
        updated_at: i64 = "updatedAt",
    }
}
table_row! {
    /// Row of `"InboxTeam"`.
    pub struct InboxTeam in "InboxTeam" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
    }
}
table_row! {
    /// Row of `"InboxTeamMember"`.
    pub struct InboxTeamMember in "InboxTeamMember" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        inbox_team_id: i64 = "inboxTeamId",
        user_id: i64 = "userId",
    }
}
table_row! {
    /// Row of `"Integration"`.
    pub struct Integration in "Integration" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_type: String = "integrationType",
    }
}
table_row! {
    /// Row of `"IntegrationActiveCampaign"`.
    pub struct IntegrationActiveCampaign in "IntegrationActiveCampaign" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        auth: Json = "auth",
    }
}
table_row! {
    /// Row of `"IntegrationApi"`.
    pub struct IntegrationApi in "IntegrationApi" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        name: String = "name",
        token_hash: String = "tokenHash",
        token_prefix: String = "tokenPrefix",
        callback_url: Option<String> = "callbackUrl",
        enabled: bool = "enabled",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
    }
}
table_row! {
    /// Row of `"IntegrationClaude"`.
    pub struct IntegrationClaude in "IntegrationClaude" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        auto_reply: bool = "autoReply",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        max_output_tokens: i64 = "maxOutputTokens",
        model: String = "model",
        prompt: Option<String> = "prompt",
        temperature: Option<f64> = "temperature",
    }
}
table_row! {
    /// Row of `"IntegrationDeepseek"`.
    pub struct IntegrationDeepseek in "IntegrationDeepseek" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        auto_reply: bool = "autoReply",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        max_output_tokens: i64 = "maxOutputTokens",
        model: String = "model",
        prompt: Option<String> = "prompt",
        temperature: Option<f64> = "temperature",
    }
}
table_row! {
    /// Row of `"IntegrationDrip"`.
    pub struct IntegrationDrip in "IntegrationDrip" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        auth: Json = "auth",
    }
}
table_row! {
    /// Row of `"IntegrationFacebookAds"`.
    pub struct IntegrationFacebookAds in "IntegrationFacebookAds" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        auth: Json = "auth",
        token_expires_at: Option<i64> = "tokenExpiresAt",
        status: String = "status",
    }
}
table_row! {
    /// Row of `"IntegrationGemini"`.
    pub struct IntegrationGemini in "IntegrationGemini" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        auto_reply: bool = "autoReply",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        max_output_tokens: i64 = "maxOutputTokens",
        model: String = "model",
        prompt: Option<String> = "prompt",
        temperature: Option<f64> = "temperature",
    }
}
table_row! {
    /// Row of `"IntegrationGetResponse"`.
    pub struct IntegrationGetResponse in "IntegrationGetResponse" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        auth: Json = "auth",
    }
}
table_row! {
    /// Row of `"IntegrationGoogleAds"`.
    pub struct IntegrationGoogleAds in "IntegrationGoogleAds" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        auth: Json = "auth",
        customer_id: String = "customerId",
        login_customer_id: Option<String> = "loginCustomerId",
        descriptive_name: Option<String> = "descriptiveName",
        currency_code: Option<String> = "currencyCode",
        conversion_customer_id: Option<String> = "conversionCustomerId",
        accepted_customer_data_terms: Option<bool> = "acceptedCustomerDataTerms",
        conversion_actions: Option<Json> = "conversionActions",
        conversion_actions_synced_at: Option<i64> = "conversionActionsSyncedAt",
        setup_error: Option<String> = "setupError",
        setup_error_at: Option<i64> = "setupErrorAt",
    }
}
table_row! {
    /// Row of `"IntegrationGoogleCalendar"`.
    pub struct IntegrationGoogleCalendar in "IntegrationGoogleCalendar" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        auth: Json = "auth",
        provider_calendar_id: String = "providerCalendarId",
        email: Option<String> = "email",
    }
}
table_row! {
    /// Row of `"IntegrationGoogleSheet"`.
    pub struct IntegrationGoogleSheet in "IntegrationGoogleSheet" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        auth: Json = "auth",
    }
}
table_row! {
    /// Row of `"IntegrationInstagram"`.
    pub struct IntegrationInstagram in "IntegrationInstagram" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        user_info: Option<Json> = "userInfo",
        ig_id: String = "igId",
        page_id: String = "pageId",
        name: String = "name",
        username: String = "username",
        coexist_enabled: bool = "coexistEnabled",
        coexist_ai_reads_synced_history: bool = "coexistAiReadsSyncedHistory",
        has_capi_scope: bool = "hasCapiScope",
        capi_scope_checked_at: Option<i64> = "capiScopeCheckedAt",
        dataset_id: Option<String> = "datasetId",
        capi_access_token: Option<Json> = "capiAccessToken",
        capi_disconnected_at: Option<i64> = "capiDisconnectedAt",
        capi_test_event_code: Option<String> = "capiTestEventCode",
        conversation_starters: Json = "conversationStarters",
        persistent_menus: Json = "persistentMenus",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
        r#type: String = "type",
        token_refresh_error: Option<String> = "tokenRefreshError",
        welcome_flow_id: Option<i64> = "welcomeFlowId",
    }
}
table_row! {
    /// Row of `"IntegrationKlaviyo"`.
    pub struct IntegrationKlaviyo in "IntegrationKlaviyo" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        auth: Json = "auth",
    }
}
table_row! {
    /// Row of `"IntegrationMailchimp"`.
    pub struct IntegrationMailchimp in "IntegrationMailchimp" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        auth: Json = "auth",
    }
}
table_row! {
    /// Row of `"IntegrationMailerLite"`.
    pub struct IntegrationMailerLite in "IntegrationMailerLite" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        auth: Json = "auth",
    }
}
table_row! {
    /// Row of `"IntegrationMessenger"`.
    pub struct IntegrationMessenger in "IntegrationMessenger" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        user_info: Option<Json> = "userInfo",
        page_id: String = "pageId",
        name: String = "name",
        conversation_starters: Json = "conversationStarters",
        persistent_menus: Json = "persistentMenus",
        personas: Json = "personas",
        persona_id: Option<String> = "personaId",
        coexist_enabled: bool = "coexistEnabled",
        coexist_ai_reads_synced_history: bool = "coexistAiReadsSyncedHistory",
        has_capi_scope: bool = "hasCapiScope",
        capi_scope_checked_at: Option<i64> = "capiScopeCheckedAt",
        dataset_id: Option<String> = "datasetId",
        capi_access_token: Option<Json> = "capiAccessToken",
        capi_disconnected_at: Option<i64> = "capiDisconnectedAt",
        capi_test_event_code: Option<String> = "capiTestEventCode",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
        sync_tag_enabled_at: Option<i64> = "syncTagEnabledAt",
        token_refresh_error: Option<String> = "tokenRefreshError",
        welcome_flow_id: Option<i64> = "welcomeFlowId",
        handover_resume_flow_id: Option<i64> = "handoverResumeFlowId",
    }
}
table_row! {
    /// Row of `"IntegrationMetaCatalog"`.
    pub struct IntegrationMetaCatalog in "IntegrationMetaCatalog" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        catalog_id: Option<String> = "catalogId",
        catalog_name: Option<String> = "catalogName",
        business_id: Option<String> = "businessId",
        encrypted_auth: Option<Json> = "encryptedAuth",
        auth_mode: String = "authMode",
        token_expires_at: Option<i64> = "tokenExpiresAt",
        status: String = "status",
        deleted_at: Option<i64> = "deletedAt",
        import_status: String = "importStatus",
        import_total_count: i64 = "importTotalCount",
        imported_count: i64 = "importedCount",
        import_failed_count: i64 = "importFailedCount",
        import_error: Option<String> = "importError",
        last_imported_at: Option<i64> = "lastImportedAt",
        currency: String = "currency",
        store_url: Option<String> = "storeUrl",
    }
}
table_row! {
    /// Row of `"IntegrationMoosend"`.
    pub struct IntegrationMoosend in "IntegrationMoosend" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        auth: Json = "auth",
    }
}
table_row! {
    /// Row of `"IntegrationMuse"`.
    pub struct IntegrationMuse in "IntegrationMuse" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        auto_reply: bool = "autoReply",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        max_output_tokens: i64 = "maxOutputTokens",
        model: String = "model",
        prompt: Option<String> = "prompt",
        temperature: Option<f64> = "temperature",
    }
}
table_row! {
    /// Row of `"IntegrationOpenai"`.
    pub struct IntegrationOpenai in "IntegrationOpenai" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        auto_reply: bool = "autoReply",
        auto_reply_voice: bool = "autoReplyVoice",
        voice: Option<String> = "voice",
        prompt: Option<String> = "prompt",
        model: String = "model",
        temperature: Option<f64> = "temperature",
        max_output_tokens: i64 = "maxOutputTokens",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        ai_agent_id: Option<i64> = "aiAgentId",
    }
}
table_row! {
    /// Row of `"IntegrationOpenaiCompatible"`.
    pub struct IntegrationOpenaiCompatible in "IntegrationOpenaiCompatible" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Option<Json> = "auth",
        auto_reply: bool = "autoReply",
        base_url: String = "baseURL",
        default_model: String = "defaultModel",
        enabled: bool = "enabled",
        integration_id: i64 = "integrationId",
        name: String = "name",
        preset: String = "preset",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"IntegrationOpenrouter"`.
    pub struct IntegrationOpenrouter in "IntegrationOpenrouter" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        auto_reply: bool = "autoReply",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        max_output_tokens: i64 = "maxOutputTokens",
        model: String = "model",
        prompt: Option<String> = "prompt",
        temperature: Option<f64> = "temperature",
    }
}
table_row! {
    /// Row of `"IntegrationOutlookCalendar"`.
    pub struct IntegrationOutlookCalendar in "IntegrationOutlookCalendar" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        auth: Json = "auth",
        provider_calendar_id: String = "providerCalendarId",
        email: Option<String> = "email",
    }
}
table_row! {
    /// Row of `"IntegrationSendGrid"`.
    pub struct IntegrationSendGrid in "IntegrationSendGrid" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_id: i64 = "integrationId",
        auth: Json = "auth",
    }
}
table_row! {
    /// Row of `"IntegrationShopify"`.
    pub struct IntegrationShopify in "IntegrationShopify" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        shop_domain: String = "shopDomain",
        shop_name: Option<String> = "shopName",
        currency: String = "currency",
        auth_kind: String = "authKind",
        encrypted_auth: Json = "encryptedAuth",
        webhook_subscription_ids: Json = "webhookSubscriptionIds",
        status: String = "status",
        status_reason: Option<String> = "statusReason",
        sync_status: String = "syncStatus",
        sync_error: Option<String> = "syncError",
        last_synced_at: Option<i64> = "lastSyncedAt",
        product_count: i64 = "productCount",
        order_count: i64 = "orderCount",
    }
}
table_row! {
    /// Row of `"IntegrationSmtp"`.
    pub struct IntegrationSmtp in "IntegrationSmtp" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        name: String = "name",
        from_address: String = "fromAddress",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
    }
}
table_row! {
    /// Row of `"IntegrationTelegram"`.
    pub struct IntegrationTelegram in "IntegrationTelegram" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        bot_id: String = "botId",
        name: String = "name",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
    }
}
table_row! {
    /// Row of `"IntegrationThreads"`.
    pub struct IntegrationThreads in "IntegrationThreads" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        threads_user_id: String = "threadsUserId",
        username: String = "username",
        name: String = "name",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
        token_refresh_error: Option<String> = "tokenRefreshError",
    }
}
table_row! {
    /// Row of `"IntegrationTiktok"`.
    pub struct IntegrationTiktok in "IntegrationTiktok" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        open_id: String = "openId",
        name: String = "name",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
        token_refresh_error: Option<String> = "tokenRefreshError",
    }
}
table_row! {
    /// Row of `"IntegrationWebchat"`.
    pub struct IntegrationWebchat in "IntegrationWebchat" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        name: String = "name",
        enable: bool = "enable",
        authorized_domains: Json = "authorizedDomains",
        conversation_starters: Json = "conversationStarters",
        persistent_menus: Json = "persistentMenus",
        brand_color: String = "brandColor",
        hide_header: bool = "hideHeader",
        show_logo: bool = "showLogo",
        hide_message_input: bool = "hideMessageInput",
        custom_css: Option<String> = "customCss",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
        welcome_flow_id: Option<i64> = "welcomeFlowId",
    }
}
table_row! {
    /// Row of `"IntegrationWhatsapp"`.
    pub struct IntegrationWhatsapp in "IntegrationWhatsapp" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        phone_number_id: String = "phoneNumberId",
        waba_id: String = "wabaId",
        business_id: String = "businessId",
        name: String = "name",
        display_phone_number: String = "displayPhoneNumber",
        coexist_enabled: bool = "coexistEnabled",
        calling_enabled: Option<bool> = "callingEnabled",
        inbound_calls_enabled: bool = "inboundCallsEnabled",
        call_hours: Option<Json> = "callHours",
        call_recording_enabled: bool = "callRecordingEnabled",
        call_recording_retention_days: i64 = "callRecordingRetentionDays",
        call_transcription_enabled: bool = "callTranscriptionEnabled",
        call_recording_mode: String = "callRecordingMode",
        call_transcription_mode: String = "callTranscriptionMode",
        call_announcement_language: Option<String> = "callAnnouncementLanguage",
        call_recording_purpose: Option<String> = "callRecordingPurpose",
        pix_redirect_enabled: bool = "pixRedirectEnabled",
        coexist_ai_reads_synced_history: bool = "coexistAiReadsSyncedHistory",
        is_coexist: bool = "isCoexist",
        platform_type: String = "platformType",
        history_declined: bool = "historyDeclined",
        has_capi_scope: bool = "hasCapiScope",
        capi_scope_checked_at: Option<i64> = "capiScopeCheckedAt",
        dataset_id: Option<String> = "datasetId",
        capi_access_token: Option<Json> = "capiAccessToken",
        capi_disconnected_at: Option<i64> = "capiDisconnectedAt",
        capi_test_event_code: Option<String> = "capiTestEventCode",
        registration_status: String = "registrationStatus",
        registration_error: Option<Json> = "registrationError",
        verification_code_requested_at: Option<i64> = "verificationCodeRequestedAt",
        token_refresh_error: Option<String> = "tokenRefreshError",
        routing_roles: Option<Json> = "routingRoles",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
        handover_resume_flow_id: Option<i64> = "handoverResumeFlowId",
    }
}
table_row! {
    /// Row of `"IntegrationWoocommerce"`.
    pub struct IntegrationWoocommerce in "IntegrationWoocommerce" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        site_url: String = "siteUrl",
        store_name: Option<String> = "storeName",
        currency: String = "currency",
        encrypted_auth: Json = "encryptedAuth",
        webhook_ids: Json = "webhookIds",
        status: String = "status",
        status_reason: Option<String> = "statusReason",
        sync_status: String = "syncStatus",
        sync_error: Option<String> = "syncError",
        last_synced_at: Option<i64> = "lastSyncedAt",
        product_count: i64 = "productCount",
        order_count: i64 = "orderCount",
    }
}
table_row! {
    /// Row of `"IntegrationZalo"`.
    pub struct IntegrationZalo in "IntegrationZalo" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        auth: Json = "auth",
        oa_id: String = "oaId",
        name: String = "name",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
        sync_tag_enabled_at: Option<i64> = "syncTagEnabledAt",
        token_refresh_error: Option<String> = "tokenRefreshError",
        fallback_flow_id: Option<i64> = "fallbackFlowId",
    }
}
table_row! {
    /// Row of `"Invitation"`.
    pub struct Invitation in "Invitation" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        code: String = "code",
        permissions: Json = "permissions",
        expires_at: i64 = "expiresAt",
        workspace_id: Option<i64> = "workspaceId",
        invited_by: i64 = "invitedBy",
    }
}
table_row! {
    /// Row of `"Jwk"`.
    pub struct Jwk in "Jwk" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        public_key: String = "publicKey",
        private_key: String = "privateKey",
        alg: Option<String> = "alg",
        crv: Option<String> = "crv",
        expires_at: Option<i64> = "expiresAt",
    }
}
table_row! {
    /// Row of `"Leaderboard"`.
    pub struct Leaderboard in "Leaderboard" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        description: String = "description",
        cover_image_url: String = "coverImageUrl",
        starts_at: i64 = "startsAt",
        ends_at: i64 = "endsAt",
        status: String = "status",
        platform: String = "platform",
        inbox_id: Option<i64> = "inboxId",
        minigame_id: Option<i64> = "minigameId",
        scope: Json = "scope",
        rules: Json = "rules",
        followers_only: bool = "followersOnly",
        tag_ids: Json = "tagIds",
        reset_time: String = "resetTime",
        cooldown_seconds: i64 = "cooldownSeconds",
        referral_welcome_flow_id: Option<i64> = "referralWelcomeFlowId",
        reward_settings: Json = "rewardSettings",
        rewards_triggered_at: Option<i64> = "rewardsTriggeredAt",
        reward_recipients: Option<Json> = "rewardRecipients",
        finished_at: Option<i64> = "finishedAt",
        deleted_at: Option<i64> = "deletedAt",
    }
}
table_row! {
    /// Row of `"LeaderboardAuditLog"`.
    pub struct LeaderboardAuditLog in "LeaderboardAuditLog" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        leaderboard_id: i64 = "leaderboardId",
        workspace_id: i64 = "workspaceId",
        contact_id: Option<i64> = "contactId",
        action: String = "action",
        delta: i64 = "delta",
        actor_user_id: Option<i64> = "actorUserId",
        reason: String = "reason",
    }
}
table_row! {
    /// Row of `"LeaderboardParticipant"`.
    pub struct LeaderboardParticipant in "LeaderboardParticipant" {
        leaderboard_id: i64 = "leaderboardId",
        contact_id: i64 = "contactId",
        workspace_id: i64 = "workspaceId",
        score: i64 = "score",
        is_banned: bool = "isBanned",
        referrer_contact_id: Option<i64> = "referrerContactId",
        scored_at: i64 = "scoredAt",
        daily_bucket: Option<String> = "dailyBucket",
        daily_counts: Json = "dailyCounts",
        breakdown: Json = "breakdown",
        last_activity_at: i64 = "lastActivityAt",
        created_at: i64 = "createdAt",
    }
}
table_row! {
    /// Row of `"LeaderboardScoreEvent"`.
    pub struct LeaderboardScoreEvent in "LeaderboardScoreEvent" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        leaderboard_id: i64 = "leaderboardId",
        workspace_id: i64 = "workspaceId",
        contact_id: i64 = "contactId",
        delta: i64 = "delta",
        source: String = "source",
        rule: Option<String> = "rule",
        source_ref: Option<String> = "sourceRef",
    }
}
table_row! {
    /// Row of `"License"`.
    pub struct License in "License" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        license_id: String = "licenseId",
        token: String = "token",
        expires_at: i64 = "expiresAt",
    }
}
table_row! {
    /// Row of `"MagicLink"`.
    pub struct MagicLink in "MagicLink" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        url: String = "url",
    }
}
table_row! {
    /// Row of `"MagicLinkStat"`.
    pub struct MagicLinkStat in "MagicLinkStat" {
        workspace_id: i64 = "workspaceId",
        link_id: i64 = "linkId",
        contact_id: i64 = "contactId",
        contact_inbox_id: i64 = "contactInboxId",
        occurred_at: i64 = "occurredAt",
        created_at: i64 = "createdAt",
    }
}
table_row! {
    /// Row of `"MediaLibraryFile"`.
    pub struct MediaLibraryFile in "MediaLibraryFile" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        path: String = "path",
        mime_type: String = "mimeType",
        size: i64 = "size",
        is_favourite: bool = "isFavourite",
        last_accessed_at: Option<i64> = "lastAccessedAt",
        workspace_id: i64 = "workspaceId",
        folder_id: Option<i64> = "folderId",
    }
}
table_row! {
    /// Row of `"MediaLibraryFolder"`.
    pub struct MediaLibraryFolder in "MediaLibraryFolder" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"Message"`.
    pub struct Message in "Message" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        conversation_id: i64 = "conversationId",
        contact_inbox_id: i64 = "contactInboxId",
        workspace_id: i64 = "workspaceId",
        text: Option<String> = "text",
        content_attributes: Option<Json> = "contentAttributes",
        message_type: String = "messageType",
        content_type: String = "contentType",
        sender_type: String = "senderType",
        sender_id: Option<i64> = "senderId",
        source_id: Option<String> = "sourceId",
        deleted_at: Option<i64> = "deletedAt",
        r#type: String = "type",
        parent_id: Option<String> = "parentId",
        attributes: Option<Json> = "attributes",
        send_error: Option<String> = "sendError",
    }
}
table_row! {
    /// Row of `"MessageCleanup"`.
    pub struct MessageCleanup in "MessageCleanup" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        contact_id: i64 = "contactId",
        contact_inbox_id: i64 = "contactInboxId",
        inbox_id: i64 = "inboxId",
        source_id: String = "sourceId",
        conversation_ids: Json = "conversationIds",
        since_time: Option<i64> = "sinceTime",
        deleted_at: i64 = "deletedAt",
        status: String = "status",
        attempts: i64 = "attempts",
        last_error: Option<String> = "lastError",
        processed_at: Option<i64> = "processedAt",
    }
}
table_row! {
    /// Row of `"MessagingAdOperation"`.
    pub struct MessagingAdOperation in "MessagingAdOperation" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        channel: String = "channel",
        integration_whatsapp_id: Option<i64> = "integrationWhatsappId",
        integration_messenger_id: Option<i64> = "integrationMessengerId",
        integration_instagram_id: Option<i64> = "integrationInstagramId",
        ad_account_id: String = "adAccountId",
        name: String = "name",
        create_state: String = "createState",
        publish_state: String = "publishState",
        meta_campaign_id: Option<String> = "metaCampaignId",
        meta_ad_set_id: Option<String> = "metaAdSetId",
        meta_ad_creative_id: Option<String> = "metaAdCreativeId",
        meta_ad_id: Option<String> = "metaAdId",
        input: Json = "input",
        last_error: Option<String> = "lastError",
        cleanup_error: Option<String> = "cleanupError",
        created_by: Option<i64> = "createdBy",
    }
}
table_row! {
    /// Row of `"MessagingAdsConnection"`.
    pub struct MessagingAdsConnection in "MessagingAdsConnection" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        channel: String = "channel",
        integration_whatsapp_id: Option<i64> = "integrationWhatsappId",
        integration_messenger_id: Option<i64> = "integrationMessengerId",
        integration_instagram_id: Option<i64> = "integrationInstagramId",
        auth: Json = "auth",
        status: String = "status",
    }
}
table_row! {
    /// Row of `"MessengerMessageTemplate"`.
    pub struct MessengerMessageTemplate in "MessengerMessageTemplate" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        integration_messenger_id: i64 = "integrationMessengerId",
        source_id: String = "sourceId",
        language: String = "language",
        category: String = "category",
        status: String = "status",
        parameter_format: String = "parameterFormat",
        components: Json = "components",
    }
}
table_row! {
    /// Row of `"MessengerWelcomeMessageFlow"`.
    pub struct MessengerWelcomeMessageFlow in "MessengerWelcomeMessageFlow" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        flow_id: i64 = "flowId",
        integration_messenger_id: i64 = "integrationMessengerId",
        page_id: String = "pageId",
        meta_flow_id: String = "metaFlowId",
        flow_version_id: Option<i64> = "flowVersionId",
        status: String = "status",
    }
}
table_row! {
    /// Row of `"MetaCapiEvent"`.
    pub struct MetaCapiEvent in "MetaCapiEvent" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        channel: String = "channel",
        integration_id: i64 = "integrationId",
        contact_inbox_id: i64 = "contactInboxId",
        event_name: String = "eventName",
        currency: Option<String> = "currency",
        content_category: Option<String> = "contentCategory",
        content_name: Option<String> = "contentName",
        value: Option<String> = "value",
        action_source: String = "actionSource",
        content_type: Option<String> = "contentType",
        content_ids: Option<Json> = "contentIds",
        source: String = "source",
        source_key: String = "sourceKey",
        occurred_at: i64 = "occurredAt",
        capi_status: String = "capiStatus",
        capi_sent_at: Option<i64> = "capiSentAt",
        capi_error: Option<String> = "capiError",
    }
}
table_row! {
    /// Row of `"MetaCatalogItem"`.
    pub struct MetaCatalogItem in "MetaCatalogItem" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        integration_meta_catalog_id: i64 = "integrationMetaCatalogId",
        product_id: i64 = "productId",
        catalog_id: String = "catalogId",
        direction: String = "direction",
        retailer_id: String = "retailerId",
        last_synced_fingerprint: Option<String> = "lastSyncedFingerprint",
        last_synced_at: Option<i64> = "lastSyncedAt",
    }
}
table_row! {
    /// Row of `"MetaCatalogSyncRun"`.
    pub struct MetaCatalogSyncRun in "MetaCatalogSyncRun" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        integration_meta_catalog_id: i64 = "integrationMetaCatalogId",
        status: String = "status",
        direction: String = "direction",
        catalog_id: Option<String> = "catalogId",
        scope: String = "scope",
        category_id: Option<i64> = "categoryId",
        selected_product_ids: Json = "selectedProductIds",
        handles: Json = "handles",
        submission_lease_id: Option<String> = "submissionLeaseId",
        total_count: i64 = "totalCount",
        succeeded_count: i64 = "succeededCount",
        failed_count: i64 = "failedCount",
        skipped_count: i64 = "skippedCount",
        item_errors: Json = "itemErrors",
        skipped_items: Json = "skippedItems",
        poll_attempt: i64 = "pollAttempt",
        error: Option<String> = "error",
        started_at: Option<i64> = "startedAt",
        finished_at: Option<i64> = "finishedAt",
    }
}
table_row! {
    /// Row of `"MiniApp"`.
    pub struct MiniApp in "MiniApp" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        enabled: bool = "enabled",
        definition: Json = "definition",
        flow_json: Json = "flowJson",
        submissions_count: i64 = "submissionsCount",
    }
}
table_row! {
    /// Row of `"MiniAppPublication"`.
    pub struct MiniAppPublication in "MiniAppPublication" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        mini_app_id: i64 = "miniAppId",
        integration_whatsapp_id: i64 = "integrationWhatsappId",
        whatsapp_flow_id: Option<i64> = "whatsappFlowId",
        source_id: String = "sourceId",
        status: String = "status",
        validation_errors: Json = "validationErrors",
        published_at: Option<i64> = "publishedAt",
    }
}
table_row! {
    /// Row of `"MiniAppSubmission"`.
    pub struct MiniAppSubmission in "MiniAppSubmission" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        mini_app_id: i64 = "miniAppId",
        contact_id: Option<i64> = "contactId",
        source: String = "source",
        answers: Json = "answers",
    }
}
table_row! {
    /// Row of `"MiniAppUpload"`.
    pub struct MiniAppUpload in "MiniAppUpload" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        mini_app_id: i64 = "miniAppId",
        contact_id: Option<i64> = "contactId",
        submission_id: Option<i64> = "submissionId",
        input_name: String = "inputName",
        upload_token: String = "uploadToken",
        path: String = "path",
        file_name: String = "fileName",
        mime_type: String = "mimeType",
        size: i64 = "size",
        status: String = "status",
    }
}
table_row! {
    /// Row of `"Minigame"`.
    pub struct Minigame in "Minigame" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        r#type: String = "type",
        enabled: bool = "enabled",
        general_settings: Json = "generalSettings",
        appearance: Json = "appearance",
        player_settings: Json = "playerSettings",
        prize_settings: Json = "prizeSettings",
        winning_message_settings: Json = "winningMessageSettings",
        non_winning_message_settings: Json = "nonWinningMessageSettings",
        plays_count: i64 = "playsCount",
        participants_count: i64 = "participantsCount",
        winners_count: i64 = "winnersCount",
        shares_count: i64 = "sharesCount",
    }
}
table_row! {
    /// Row of `"MinigameContact"`.
    pub struct MinigameContact in "MinigameContact" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        minigame_id: i64 = "minigameId",
        contact_id: i64 = "contactId",
        opened_at: i64 = "openedAt",
        played: i64 = "played",
        remaining: i64 = "remaining",
        shares_count: i64 = "sharesCount",
        referrer_contact_id: Option<i64> = "referrerContactId",
        contact_inbox_id: Option<i64> = "contactInboxId",
    }
}
table_row! {
    /// Row of `"MinigamePlay"`.
    pub struct MinigamePlay in "MinigamePlay" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        minigame_id: i64 = "minigameId",
        contact_id: i64 = "contactId",
        is_winning: bool = "isWinning",
        prize_id: Option<String> = "prizeId",
        prize_name: Option<String> = "prizeName",
    }
}
table_row! {
    /// Row of `"MobileRefreshToken"`.
    pub struct MobileRefreshToken in "MobileRefreshToken" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        user_id: i64 = "userId",
        device_id: String = "deviceId",
        family_id: String = "familyId",
        token_hash: String = "tokenHash",
        expires_at: i64 = "expiresAt",
        rotated_at: Option<i64> = "rotatedAt",
        revoked: Option<i64> = "revoked",
    }
}
table_row! {
    /// Row of `"OAuthAccessToken"`.
    pub struct OAuthAccessToken in "OAuthAccessToken" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        token: String = "token",
        client_id: String = "clientId",
        session_id: Option<i64> = "sessionId",
        user_id: Option<i64> = "userId",
        reference_id: Option<String> = "referenceId",
        authorization_code_id: Option<String> = "authorizationCodeId",
        resources: Option<Json> = "resources",
        requested_user_info_claims: Option<Json> = "requestedUserInfoClaims",
        refresh_id: Option<i64> = "refreshId",
        expires_at: i64 = "expiresAt",
        revoked: Option<i64> = "revoked",
        confirmation: Option<Json> = "confirmation",
        scopes: Json = "scopes",
    }
}
table_row! {
    /// Row of `"OAuthClient"`.
    pub struct OAuthClient in "OAuthClient" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        client_id: String = "clientId",
        client_secret: Option<String> = "clientSecret",
        client_discovery_id: Option<String> = "clientDiscoveryId",
        disabled: bool = "disabled",
        skip_consent: Option<bool> = "skipConsent",
        enable_end_session: Option<bool> = "enableEndSession",
        subject_type: Option<String> = "subjectType",
        scopes: Option<Json> = "scopes",
        client_credentials_scopes: Json = "clientCredentialsScopes",
        user_id: Option<i64> = "userId",
        name: Option<String> = "name",
        uri: Option<String> = "uri",
        icon: Option<String> = "icon",
        contacts: Option<Json> = "contacts",
        tos: Option<String> = "tos",
        policy: Option<String> = "policy",
        software_id: Option<String> = "softwareId",
        software_version: Option<String> = "softwareVersion",
        software_statement: Option<String> = "softwareStatement",
        redirect_uris: Json = "redirectUris",
        post_logout_redirect_uris: Option<Json> = "postLogoutRedirectUris",
        backchannel_logout_uri: Option<String> = "backchannelLogoutUri",
        backchannel_logout_session_required: Option<bool> = "backchannelLogoutSessionRequired",
        token_endpoint_auth_method: Option<String> = "tokenEndpointAuthMethod",
        application_type: Option<String> = "applicationType",
        jwks: Option<String> = "jwks",
        jwks_uri: Option<String> = "jwksUri",
        grant_types: Option<Json> = "grantTypes",
        response_types: Option<Json> = "responseTypes",
        require_pkce: Option<bool> = "requirePKCE",
        dpop_bound_access_tokens: bool = "dpopBoundAccessTokens",
        reference_id: Option<String> = "referenceId",
        metadata: Option<Json> = "metadata",
    }
}
table_row! {
    /// Row of `"OAuthClientAssertion"`.
    pub struct OAuthClientAssertion in "OAuthClientAssertion" {
        id: String = "id",
        expires_at: i64 = "expiresAt",
    }
}
table_row! {
    /// Row of `"OAuthClientResource"`.
    pub struct OAuthClientResource in "OAuthClientResource" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        client_id: String = "clientId",
        resource_id: String = "resourceId",
        metadata: Option<Json> = "metadata",
    }
}
table_row! {
    /// Row of `"OAuthConsent"`.
    pub struct OAuthConsent in "OAuthConsent" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        client_id: String = "clientId",
        user_id: Option<i64> = "userId",
        reference_id: Option<String> = "referenceId",
        resources: Option<Json> = "resources",
        requested_user_info_claims: Option<Json> = "requestedUserInfoClaims",
        scopes: Json = "scopes",
        workspace_id: Option<i64> = "workspaceId",
    }
}
table_row! {
    /// Row of `"OAuthRefreshToken"`.
    pub struct OAuthRefreshToken in "OAuthRefreshToken" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        token: String = "token",
        client_id: String = "clientId",
        session_id: Option<i64> = "sessionId",
        user_id: i64 = "userId",
        reference_id: Option<String> = "referenceId",
        authorization_code_id: Option<String> = "authorizationCodeId",
        resources: Option<Json> = "resources",
        requested_user_info_claims: Option<Json> = "requestedUserInfoClaims",
        expires_at: i64 = "expiresAt",
        revoked: Option<i64> = "revoked",
        rotated_at: Option<i64> = "rotatedAt",
        rotation_replay_response: Option<String> = "rotationReplayResponse",
        rotation_replay_expires_at: Option<i64> = "rotationReplayExpiresAt",
        auth_time: Option<i64> = "authTime",
        confirmation: Option<Json> = "confirmation",
        scopes: Json = "scopes",
    }
}
table_row! {
    /// Row of `"OAuthResource"`.
    pub struct OAuthResource in "OAuthResource" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        identifier: String = "identifier",
        name: String = "name",
        access_token_ttl: Option<i64> = "accessTokenTtl",
        refresh_token_ttl: Option<i64> = "refreshTokenTtl",
        signing_algorithm: Option<String> = "signingAlgorithm",
        signing_key_id: Option<String> = "signingKeyId",
        allowed_scopes: Option<Json> = "allowedScopes",
        custom_claims: Option<Json> = "customClaims",
        dpop_bound_access_tokens_required: bool = "dpopBoundAccessTokensRequired",
        disabled: bool = "disabled",
        policy_version: i64 = "policyVersion",
        metadata: Option<Json> = "metadata",
    }
}
table_row! {
    /// Row of `"Order"`.
    pub struct Order in "Order" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        number: i64 = "number",
        contact_id: Option<i64> = "contactId",
        contact_inbox_id: Option<i64> = "contactInboxId",
        inbox_id: Option<i64> = "inboxId",
        conversation_id: Option<i64> = "conversationId",
        channel: Option<String> = "channel",
        source: String = "source",
        status: String = "status",
        payment_method: String = "paymentMethod",
        payment_status: String = "paymentStatus",
        payment_expires_at: Option<i64> = "paymentExpiresAt",
        paid_at: Option<i64> = "paidAt",
        currency: String = "currency",
        subtotal: i64 = "subtotal",
        discount_total: i64 = "discountTotal",
        shipping_total: i64 = "shippingTotal",
        service_fee_total: i64 = "serviceFeeTotal",
        tax_total: i64 = "taxTotal",
        coupon_discount: i64 = "couponDiscount",
        total: i64 = "total",
        discount_code_id: Option<i64> = "discountCodeId",
        discount_code: Option<String> = "discountCode",
        customer_name: Option<String> = "customerName",
        customer_email: Option<String> = "customerEmail",
        customer_phone: Option<String> = "customerPhone",
        shipping_type: Option<String> = "shippingType",
        shipping_address: Option<Json> = "shippingAddress",
        note: Option<String> = "note",
        rating: Option<i64> = "rating",
        rating_comment: Option<String> = "ratingComment",
        accepted_at: Option<i64> = "acceptedAt",
        shipped_at: Option<i64> = "shippedAt",
        concluded_at: Option<i64> = "concludedAt",
        cancelled_at: Option<i64> = "cancelledAt",
        cancel_reason: Option<String> = "cancelReason",
        external_id: Option<String> = "externalId",
        external_name: Option<String> = "externalName",
        fulfillment_status: Option<String> = "fulfillmentStatus",
        tracking_company: Option<String> = "trackingCompany",
        tracking_number: Option<String> = "trackingNumber",
        tracking_url: Option<String> = "trackingUrl",
    }
}
table_row! {
    /// Row of `"OrderItem"`.
    pub struct OrderItem in "OrderItem" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        order_id: i64 = "orderId",
        workspace_id: i64 = "workspaceId",
        product_id: Option<i64> = "productId",
        variant_id: Option<i64> = "variantId",
        vendor_id: Option<i64> = "vendorId",
        category_id: Option<i64> = "categoryId",
        name: String = "name",
        variant_label: Option<String> = "variantLabel",
        combination: Option<Json> = "combination",
        sku: Option<String> = "sku",
        retailer_id: Option<String> = "retailerId",
        image_url: Option<String> = "imageUrl",
        vendor_name: Option<String> = "vendorName",
        unit_price: i64 = "unitPrice",
        quantity: i64 = "quantity",
        stock_taken: i64 = "stockTaken",
        discount_percent: i64 = "discountPercent",
        tax_percent: i64 = "taxPercent",
        discount_total: i64 = "discountTotal",
        tax_total: i64 = "taxTotal",
        line_total: i64 = "lineTotal",
    }
}
table_row! {
    /// Row of `"OrderPayment"`.
    pub struct OrderPayment in "OrderPayment" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        order_id: i64 = "orderId",
        workspace_id: i64 = "workspaceId",
        provider: String = "provider",
        provider_reference: String = "providerReference",
        transaction_id: Option<String> = "transactionId",
        refund_id: Option<String> = "refundId",
        amount: i64 = "amount",
        currency: String = "currency",
        status: String = "status",
        livemode: bool = "livemode",
        checkout_url: String = "checkoutUrl",
        expires_at: i64 = "expiresAt",
        paid_at: Option<i64> = "paidAt",
        refunded_at: Option<i64> = "refundedAt",
    }
}
table_row! {
    /// Row of `"OrderStatusHistory"`.
    pub struct OrderStatusHistory in "OrderStatusHistory" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        order_id: i64 = "orderId",
        workspace_id: i64 = "workspaceId",
        from_status: Option<String> = "fromStatus",
        to_status: String = "toStatus",
        actor_type: String = "actorType",
        actor_user_id: Option<i64> = "actorUserId",
        note: Option<String> = "note",
    }
}
table_row! {
    /// Row of `"Pipeline"`.
    pub struct Pipeline in "Pipeline" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        r#type: String = "type",
        reference_name: String = "referenceName",
        currency: Option<String> = "currency",
        settings: Json = "settings",
        created_by_id: Option<i64> = "createdById",
    }
}
table_row! {
    /// Row of `"PipelineField"`.
    pub struct PipelineField in "PipelineField" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        pipeline_id: i64 = "pipelineId",
        name: String = "name",
        reference_name: String = "referenceName",
        r#type: String = "type",
        required: bool = "required",
        position: i64 = "position",
    }
}
table_row! {
    /// Row of `"PipelineItem"`.
    pub struct PipelineItem in "PipelineItem" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        pipeline_id: i64 = "pipelineId",
        stage_id: i64 = "stageId",
        contact_id: i64 = "contactId",
        number: i64 = "number",
        title: String = "title",
        description: Option<String> = "description",
        value: Option<i64> = "value",
        status: String = "status",
        priority: String = "priority",
        due_date: Option<i64> = "dueDate",
        field_values: Json = "fieldValues",
        created_by_id: Option<i64> = "createdById",
        closed_by_id: Option<i64> = "closedById",
        stage_entered_at: i64 = "stageEnteredAt",
        status_changed_at: Option<i64> = "statusChangedAt",
        closed_at: Option<i64> = "closedAt",
        last_comment_at: Option<i64> = "lastCommentAt",
        idempotency_key: Option<String> = "idempotencyKey",
    }
}
table_row! {
    /// Row of `"PipelineItemActivity"`.
    pub struct PipelineItemActivity in "PipelineItemActivity" {
        id: i64 = "id",
        workspace_id: i64 = "workspaceId",
        pipeline_id: i64 = "pipelineId",
        item_id: i64 = "itemId",
        item_number: i64 = "itemNumber",
        item_title: String = "itemTitle",
        contact_id: i64 = "contactId",
        kind: String = "kind",
        actor_kind: String = "actorKind",
        actor_user_id: Option<i64> = "actorUserId",
        actor_source_id: Option<String> = "actorSourceId",
        from_stage_id: Option<i64> = "fromStageId",
        to_stage_id: Option<i64> = "toStageId",
        from_status: Option<String> = "fromStatus",
        to_status: Option<String> = "toStatus",
        from_value: Option<i64> = "fromValue",
        to_value: Option<i64> = "toValue",
        from_priority: Option<String> = "fromPriority",
        to_priority: Option<String> = "toPriority",
        from_due_date: Option<i64> = "fromDueDate",
        to_due_date: Option<i64> = "toDueDate",
        from_title: Option<String> = "fromTitle",
        to_title: Option<String> = "toTitle",
        from_pipeline_id: Option<i64> = "fromPipelineId",
        version: i64 = "version",
        occurred_at: i64 = "occurredAt",
    }
}
table_row! {
    /// Row of `"PipelineItemAssignee"`.
    pub struct PipelineItemAssignee in "PipelineItemAssignee" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        item_id: i64 = "itemId",
        workspace_member_id: Option<i64> = "workspaceMemberId",
        inbox_team_id: Option<i64> = "inboxTeamId",
    }
}
table_row! {
    /// Row of `"PipelineItemAttachment"`.
    pub struct PipelineItemAttachment in "PipelineItemAttachment" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        item_id: i64 = "itemId",
        media_library_file_id: i64 = "mediaLibraryFileId",
    }
}
table_row! {
    /// Row of `"PipelineItemComment"`.
    pub struct PipelineItemComment in "PipelineItemComment" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        item_id: i64 = "itemId",
        text: String = "text",
        created_by_id: Option<i64> = "createdById",
        idempotency_key: Option<String> = "idempotencyKey",
    }
}
table_row! {
    /// Row of `"PipelineItemCounter"`.
    pub struct PipelineItemCounter in "PipelineItemCounter" {
        workspace_id: i64 = "workspaceId",
        last_number: i64 = "lastNumber",
    }
}
table_row! {
    /// Row of `"PipelineMember"`.
    pub struct PipelineMember in "PipelineMember" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        pipeline_id: i64 = "pipelineId",
        workspace_member_id: i64 = "workspaceMemberId",
        can_manage_settings: bool = "canManageSettings",
        can_view: bool = "canView",
        can_edit: bool = "canEdit",
        can_delete: bool = "canDelete",
    }
}
table_row! {
    /// Row of `"PipelineStage"`.
    pub struct PipelineStage in "PipelineStage" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        pipeline_id: i64 = "pipelineId",
        name: String = "name",
        description: Option<String> = "description",
        text_color: Option<String> = "textColor",
        background_color: Option<String> = "backgroundColor",
        position: i64 = "position",
        bulk_job_id: Option<String> = "bulkJobId",
    }
}
table_row! {
    /// Row of `"PlaceCategory"`.
    pub struct PlaceCategory in "PlaceCategory" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        rank: i64 = "rank",
    }
}
table_row! {
    /// Row of `"PlaceLocation"`.
    pub struct PlaceLocation in "PlaceLocation" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        latitude: f64 = "latitude",
        longitude: f64 = "longitude",
        category_id: Option<i64> = "categoryId",
    }
}
table_row! {
    /// Row of `"PlatformAdmin"`.
    pub struct PlatformAdmin in "PlatformAdmin" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        user_id: i64 = "userId",
        role: String = "role",
    }
}
table_row! {
    /// Row of `"PlatformCredential"`.
    pub struct PlatformCredential in "PlatformCredential" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        user_id: Option<i64> = "userId",
        r#type: String = "type",
        public_config: Json = "publicConfig",
        value: Json = "value",
        livemode: bool = "livemode",
        use_platform_credential: bool = "usePlatformCredential",
        is_verified: bool = "isVerified",
        verified_at: Option<i64> = "verifiedAt",
        last_used_at: Option<i64> = "lastUsedAt",
    }
}
table_row! {
    /// Row of `"Poll"`.
    pub struct Poll in "Poll" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        image_url: String = "imageUrl",
        starts_at: i64 = "startsAt",
        ends_at: i64 = "endsAt",
        after_vote_flow_id: Option<i64> = "afterVoteFlowId",
    }
}
table_row! {
    /// Row of `"PollCandidate"`.
    pub struct PollCandidate in "PollCandidate" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        poll_id: i64 = "pollId",
        position: i64 = "position",
        name: String = "name",
        description: Option<String> = "description",
        image_url: Option<String> = "imageUrl",
    }
}
table_row! {
    /// Row of `"PollVote"`.
    pub struct PollVote in "PollVote" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        poll_id: i64 = "pollId",
        candidate_id: i64 = "candidateId",
        contact_id: i64 = "contactId",
        contact_inbox_id: Option<i64> = "contactInboxId",
    }
}
table_row! {
    /// Row of `"Product"`.
    pub struct Product in "Product" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        short_description: Option<String> = "shortDescription",
        long_description: Option<String> = "longDescription",
        price: f64 = "price",
        taxes: f64 = "taxes",
        discount: f64 = "discount",
        currency: String = "currency",
        product_url: Option<String> = "productUrl",
        sku: Option<String> = "sku",
        inventory_policy: String = "inventoryPolicy",
        inventory_quantity: i64 = "inventoryQuantity",
        allow_out_of_stock_purchase: bool = "allowOutOfStockPurchase",
        images: Json = "images",
        tags: Json = "tags",
        vendor_id: Option<i64> = "vendorId",
        rank: i64 = "rank",
        category_id: Option<i64> = "categoryId",
        subcategory_id: Option<i64> = "subcategoryId",
        status: String = "status",
        is_searchable: bool = "isSearchable",
        allow_special_request: bool = "allowSpecialRequest",
        is_addon_only: bool = "isAddonOnly",
        source: String = "source",
        external_id: Option<String> = "externalId",
        external_handle: Option<String> = "externalHandle",
        external_variant_id: Option<String> = "externalVariantId",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"ProductAddon"`.
    pub struct ProductAddon in "ProductAddon" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        product_id: i64 = "productId",
        name: String = "name",
        max_selections: i64 = "maxSelections",
        addon_product_ids: Json = "addonProductIds",
    }
}
table_row! {
    /// Row of `"ProductCategory"`.
    pub struct ProductCategory in "ProductCategory" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        parent_id: Option<i64> = "parentId",
        name: String = "name",
        image_url: Option<String> = "imageUrl",
        is_visible: bool = "isVisible",
        rank: i64 = "rank",
        source: String = "source",
    }
}
table_row! {
    /// Row of `"ProductVariant"`.
    pub struct ProductVariant in "ProductVariant" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        product_id: i64 = "productId",
        combination: Json = "combination",
        price: f64 = "price",
        is_enabled: bool = "isEnabled",
        sku: Option<String> = "sku",
        inventory_quantity: Option<i64> = "inventoryQuantity",
        external_id: Option<String> = "externalId",
        external_inventory_item_id: Option<String> = "externalInventoryItemId",
    }
}
table_row! {
    /// Row of `"ProductVariantOption"`.
    pub struct ProductVariantOption in "ProductVariantOption" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        product_id: i64 = "productId",
        name: String = "name",
        values: Json = "values",
        position: i64 = "position",
    }
}
table_row! {
    /// Row of `"Questionnaire"`.
    pub struct Questionnaire in "Questionnaire" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        enable_score: bool = "enableScore",
        enable_retry_messages: bool = "enableRetryMessages",
        enable_custom_field_mapping: bool = "enableCustomFieldMapping",
        deleted_at: Option<i64> = "deletedAt",
        trigger_flow_id: Option<i64> = "triggerFlowId",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"QuestionnaireAnswer"`.
    pub struct QuestionnaireAnswer in "QuestionnaireAnswer" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        submission_id: i64 = "submissionId",
        question_id: i64 = "questionId",
        question_id_snapshot: String = "questionIdSnapshot",
        question_title_snapshot: String = "questionTitleSnapshot",
        question_type_snapshot: String = "questionTypeSnapshot",
        label_snapshot: String = "labelSnapshot",
        value: Option<Json> = "value",
        points_earned: Option<i64> = "pointsEarned",
        attempt_count: i64 = "attemptCount",
        answered_at: i64 = "answeredAt",
    }
}
table_row! {
    /// Row of `"QuestionnaireQuestion"`.
    pub struct QuestionnaireQuestion in "QuestionnaireQuestion" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        questionnaire_id: i64 = "questionnaireId",
        title: String = "title",
        r#type: String = "type",
        active: bool = "active",
        image: Option<Json> = "image",
        order_no: i64 = "orderNo",
        point: i64 = "point",
        retry_message: Option<String> = "retryMessage",
        custom_field_id: Option<i64> = "customFieldId",
        system_field_key: Option<String> = "systemFieldKey",
        config: Option<Json> = "config",
        deleted_at: Option<i64> = "deletedAt",
    }
}
table_row! {
    /// Row of `"QuestionnaireSubmission"`.
    pub struct QuestionnaireSubmission in "QuestionnaireSubmission" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        questionnaire_id: i64 = "questionnaireId",
        contact_id: i64 = "contactId",
        conversation_id: Option<i64> = "conversationId",
        status: String = "status",
        total_points: Option<i64> = "totalPoints",
        current_question_id: Option<i64> = "currentQuestionId",
        current_question_sent_at: Option<i64> = "currentQuestionSentAt",
        last_answered_message_id: Option<i64> = "lastAnsweredMessageId",
        started_at: i64 = "startedAt",
        completed_at: Option<i64> = "completedAt",
        cancelled_at: Option<i64> = "cancelledAt",
    }
}
table_row! {
    /// Row of `"RefLinkStat"`.
    pub struct RefLinkStat in "RefLinkStat" {
        workspace_id: i64 = "workspaceId",
        link_id: i64 = "linkId",
        contact_id: i64 = "contactId",
        contact_inbox_id: i64 = "contactInboxId",
        occurred_at: i64 = "occurredAt",
        created_at: i64 = "createdAt",
    }
}
table_row! {
    /// Row of `"Reflink"`.
    pub struct Reflink in "Reflink" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        r#type: String = "type",
        flow_id: i64 = "flowId",
        workspace_id: i64 = "workspaceId",
        custom_field_id: Option<i64> = "customFieldId",
        qr_styles: Option<Json> = "qrStyles",
        widget_authorized_domains: Json = "widgetAuthorizedDomains",
        widget_hidden_inbox_ids: Json = "widgetHiddenInboxIds",
        widget_logo_file_id: Option<i64> = "widgetLogoFileId",
        widget_brand_name: Option<String> = "widgetBrandName",
        widget_brand_url: Option<String> = "widgetBrandUrl",
        widget_logo_background_color: Option<String> = "widgetLogoBackgroundColor",
    }
}
table_row! {
    /// Row of `"ResourceDeletionLog"`.
    pub struct ResourceDeletionLog in "ResourceDeletionLog" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        resource_type: String = "resourceType",
        resource_id: String = "resourceId",
        resource_subtype: Option<String> = "resourceSubtype",
        resource_name: Option<String> = "resourceName",
        status: String = "status",
        requested_by_user_id: Option<i64> = "requestedByUserId",
        request_source: Option<String> = "requestSource",
        ip_address: Option<String> = "ipAddress",
        user_agent: Option<String> = "userAgent",
        requested_at: i64 = "requestedAt",
        completed_at: Option<i64> = "completedAt",
        canceled_at: Option<i64> = "canceledAt",
    }
}
table_row! {
    /// Row of `"ResponseSlaPolicy"`.
    pub struct ResponseSlaPolicy in "ResponseSlaPolicy" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        enabled: bool = "enabled",
        first_response_target_seconds: i64 = "firstResponseTargetSeconds",
        next_response_target_seconds: Option<i64> = "nextResponseTargetSeconds",
        breaching_threshold_percent: i64 = "breachingThresholdPercent",
    }
}
table_row! {
    /// Row of `"SavedReply"`.
    pub struct SavedReply in "SavedReply" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        shortcut: String = "shortcut",
        text: String = "text",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"SavedReplyImage"`.
    pub struct SavedReplyImage in "SavedReplyImage" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        saved_reply_id: i64 = "savedReplyId",
        media_library_file_id: i64 = "mediaLibraryFileId",
        workspace_id: i64 = "workspaceId",
        position: i64 = "position",
    }
}
table_row! {
    /// Row of `"Sequence"`.
    pub struct Sequence in "Sequence" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        folder_id: Option<i64> = "folderId",
        active: bool = "active",
        subscribers: i64 = "subscribers",
        messages: i64 = "messages",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"SequenceDispatch"`.
    pub struct SequenceDispatch in "SequenceDispatch" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        run_at_ms: i64 = "runAtMs",
        bucket: i64 = "bucket",
        status: String = "status",
        idempotency_key: String = "idempotencyKey",
        attempt: i64 = "attempt",
        last_error: Option<String> = "lastError",
        locked_at: Option<i64> = "lockedAt",
        lock_owner: Option<String> = "lockOwner",
        completed_at: Option<i64> = "completedAt",
        delivered_at: Option<i64> = "deliveredAt",
        seen_at: Option<i64> = "seenAt",
        clicked_at: Option<i64> = "clickedAt",
        failed_at: Option<i64> = "failedAt",
        error_content: Option<String> = "errorContent",
        workspace_id: i64 = "workspaceId",
        sequence_id: i64 = "sequenceId",
        contact_id: i64 = "contactId",
        contact_inbox_id: i64 = "contactInboxId",
        step_id: i64 = "stepId",
        enrollment_id: i64 = "enrollmentId",
        is_read: Option<i64> = "isRead",
    }
}
table_row! {
    /// Row of `"SequenceStep"`.
    pub struct SequenceStep in "SequenceStep" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        order: i64 = "order",
        delay_days: i64 = "delayDays",
        delay_minutes: i64 = "delayMinutes",
        delay_unit: Option<String> = "delayUnit",
        specific_date_time: Option<i64> = "specificDateTime",
        is_active: bool = "isActive",
        anytime: bool = "anytime",
        send_time_start: Option<String> = "sendTimeStart",
        send_time_end: Option<String> = "sendTimeEnd",
        send_days: Option<String> = "sendDays",
        flow_id: Option<i64> = "flowId",
        sequence_id: i64 = "sequenceId",
    }
}
table_row! {
    /// Row of `"Session"`.
    pub struct Session in "Session" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        expires_at: i64 = "expiresAt",
        token: String = "token",
        ip_address: Option<String> = "ipAddress",
        user_agent: Option<String> = "userAgent",
        user_id: i64 = "userId",
    }
}
table_row! {
    /// Row of `"Spreadsheet"`.
    pub struct Spreadsheet in "Spreadsheet" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        url: String = "url",
        spreadsheet_id: String = "spreadsheetId",
    }
}
table_row! {
    /// Row of `"SystemField"`.
    pub struct SystemField in "SystemField" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        r#type: String = "type",
        payload: Json = "payload",
    }
}
table_row! {
    /// Row of `"Tag"`.
    pub struct Tag in "Tag" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        deleted_at: Option<i64> = "deletedAt",
        folder_id: Option<i64> = "folderId",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"TagChannel"`.
    pub struct TagChannel in "TagChannel" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        tag_id: i64 = "tagId",
        channel_type: String = "channelType",
        integration_id: i64 = "integrationId",
        external_label_id: String = "externalLabelId",
    }
}
table_row! {
    /// Row of `"TelegramChat"`.
    pub struct TelegramChat in "TelegramChat" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
        chat_id: String = "chatId",
        title: Option<String> = "title",
        r#type: String = "type",
        bot_status: String = "botStatus",
    }
}
table_row! {
    /// Row of `"Template"`.
    pub struct Template in "Template" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        tenant_id: i64 = "tenantId",
        name: String = "name",
        description: Option<String> = "description",
        image_url: Option<String> = "imageUrl",
        publisher_name: Option<String> = "publisherName",
        youtube_video_id: Option<String> = "youtubeVideoId",
        test_link: Option<String> = "testLink",
        selection: Json = "selection",
        payload: Json = "payload",
        category_counts: Json = "categoryCounts",
        format_version: i64 = "formatVersion",
        share_token: String = "shareToken",
        share_enabled: bool = "shareEnabled",
        share_expires_at: Option<i64> = "shareExpiresAt",
        default_permissions: Json = "defaultPermissions",
        create_install_folder: bool = "createInstallFolder",
        default_auto_update: bool = "defaultAutoUpdate",
        install_count: i64 = "installCount",
        version: i64 = "version",
        payload_hash: Option<String> = "payloadHash",
        created_by: Option<i64> = "createdBy",
        deleted_at: Option<i64> = "deletedAt",
    }
}
table_row! {
    /// Row of `"TemplateInstallation"`.
    pub struct TemplateInstallation in "TemplateInstallation" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        template_id: Option<i64> = "templateId",
        template_name: String = "templateName",
        source_workspace_id: i64 = "sourceWorkspaceId",
        format_version: i64 = "formatVersion",
        status: String = "status",
        permissions: Json = "permissions",
        warnings: Json = "warnings",
        warning_count: i64 = "warningCount",
        error_message: Option<String> = "errorMessage",
        resource_count: i64 = "resourceCount",
        install_folder_id: Option<i64> = "installFolderId",
        auto_update: bool = "autoUpdate",
        source_updated_at: Option<i64> = "sourceUpdatedAt",
        installed_by: Option<i64> = "installedBy",
        installed_by_kind: String = "installedByKind",
        install_source: String = "installSource",
        completed_at: Option<i64> = "completedAt",
        staging_generation: i64 = "stagingGeneration",
        staged_cleaned_generation: i64 = "stagedCleanedGeneration",
        staged_cleanup_claimed_at: Option<i64> = "stagedCleanupClaimedAt",
        pending_publish_effects: Option<Json> = "pendingPublishEffects",
        claim_token: Option<String> = "claimToken",
        claimed_at: Option<i64> = "claimedAt",
        attempts: i64 = "attempts",
        error_code: Option<String> = "errorCode",
        source_template_id: Option<i64> = "sourceTemplateId",
        successful_install_events: i64 = "successfulInstallEvents",
        source_deleted_at: Option<i64> = "sourceDeletedAt",
        inbox_bindings: Option<Json> = "inboxBindings",
        publisher_ref: String = "publisherRef",
        publisher_uninstall_grant: i64 = "publisherUninstallGrant",
        uninstalled_at: Option<i64> = "uninstalledAt",
        uninstalled_by_kind: Option<String> = "uninstalledByKind",
        uninstall_counts: Option<Json> = "uninstallCounts",
        uninstall_state: String = "uninstallState",
        uninstall_requested_by_kind: Option<String> = "uninstallRequestedByKind",
        uninstall_requested_by: Option<i64> = "uninstallRequestedBy",
        uninstall_requested_at: Option<i64> = "uninstallRequestedAt",
        uninstall_attempt: i64 = "uninstallAttempt",
        uninstall_claim_token: Option<String> = "uninstallClaimToken",
        uninstall_claimed_at: Option<i64> = "uninstallClaimedAt",
        uninstall_error_code: Option<String> = "uninstallErrorCode",
        uninstall_operation_id: Option<i64> = "uninstallOperationId",
        source_version: i64 = "sourceVersion",
        last_synced_at: Option<i64> = "lastSyncedAt",
    }
}
table_row! {
    /// Row of `"TemplateInstallationSync"`.
    pub struct TemplateInstallationSync in "TemplateInstallationSync" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        installation_id: i64 = "installationId",
        status: String = "status",
        trigger: String = "trigger",
        requested_version: i64 = "requestedVersion",
        idempotency_key: Option<String> = "idempotencyKey",
        request_fingerprint: Option<String> = "requestFingerprint",
        requested_by: Option<i64> = "requestedBy",
        requested_by_kind: String = "requestedByKind",
        claim_token: Option<String> = "claimToken",
        claimed_at: Option<i64> = "claimedAt",
        attempts: i64 = "attempts",
        error_code: Option<String> = "errorCode",
        from_version: Option<i64> = "fromVersion",
        applied_version: Option<i64> = "appliedVersion",
        applied_hash: Option<String> = "appliedHash",
        counts: Option<Json> = "counts",
        warnings: Json = "warnings",
        warning_summary: Option<Json> = "warningSummary",
        started_at: Option<i64> = "startedAt",
        finished_at: Option<i64> = "finishedAt",
    }
}
table_row! {
    /// Row of `"TemplateInstalledResource"`.
    pub struct TemplateInstalledResource in "TemplateInstalledResource" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        installation_id: i64 = "installationId",
        workspace_id: i64 = "workspaceId",
        category: String = "category",
        resource_kind: String = "resourceKind",
        resource_id: i64 = "resourceId",
        source_resource_id: String = "sourceResourceId",
        was_existing: bool = "wasExisting",
        allow_edit: bool = "allowEdit",
        allow_delete: bool = "allowDelete",
        detached_at: Option<i64> = "detachedAt",
        synced_hash: Option<String> = "syncedHash",
        synced_version: Option<i64> = "syncedVersion",
        uninstall_outcome: Option<String> = "uninstallOutcome",
        uninstall_effects: Option<Json> = "uninstallEffects",
        uninstall_effects_attempts: i64 = "uninstallEffectsAttempts",
        uninstall_frozen_from: Option<Json> = "uninstallFrozenFrom",
        uninstall_drain_started_at: Option<i64> = "uninstallDrainStartedAt",
    }
}
table_row! {
    /// Row of `"TemplateResourceBackup"`.
    pub struct TemplateResourceBackup in "TemplateResourceBackup" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        installation_id: i64 = "installationId",
        sync_id: i64 = "syncId",
        installed_resource_id: i64 = "installedResourceId",
        resource_kind: String = "resourceKind",
        resource_id: i64 = "resourceId",
        format_version: i64 = "formatVersion",
        content: Json = "content",
        byte_size: i64 = "byteSize",
    }
}
table_row! {
    /// Row of `"TemplateUninstallOperation"`.
    pub struct TemplateUninstallOperation in "TemplateUninstallOperation" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        template_id: Option<i64> = "templateId",
        publisher_ref: String = "publisherRef",
        requested_by_kind: String = "requestedByKind",
        requested_by: Option<i64> = "requestedBy",
        state: String = "state",
        counts: Option<Json> = "counts",
        audit_log_id: String = "auditLogId",
        notice_delivered_at: Option<i64> = "noticeDeliveredAt",
        finished_at: Option<i64> = "finishedAt",
    }
}
table_row! {
    /// Row of `"TemplateVersion"`.
    pub struct TemplateVersion in "TemplateVersion" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        template_id: i64 = "templateId",
        workspace_id: i64 = "workspaceId",
        version: i64 = "version",
        payload: Json = "payload",
        payload_hash: String = "payloadHash",
        format_version: i64 = "formatVersion",
        changes: Option<Json> = "changes",
    }
}
table_row! {
    /// Row of `"Tenant"`.
    pub struct Tenant in "Tenant" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        owner_id: Option<i64> = "ownerId",
        status: String = "status",
        disabled_reason: Option<String> = "disabledReason",
        brand_name: Option<String> = "brandName",
        logo_light_path: Option<String> = "logoLightPath",
        logo_dark_path: Option<String> = "logoDarkPath",
        favicon_path: Option<String> = "faviconPath",
        custom_css: Option<String> = "customCss",
        custom_js: Option<String> = "customJs",
        theme: Option<String> = "theme",
        storage_url: Option<String> = "storageUrl",
        policy_url: Option<String> = "policyUrl",
        terms_of_service_url: Option<String> = "termsOfServiceUrl",
        signup_email_template: Option<Json> = "signupEmailTemplate",
        forgot_password_email_template: Option<Json> = "forgotPasswordEmailTemplate",
        magic_link_email_template: Option<Json> = "magicLinkEmailTemplate",
        account_credentials_email_template: Option<Json> = "accountCredentialsEmailTemplate",
        hidden_channels: Option<Json> = "hiddenChannels",
    }
}
table_row! {
    /// Row of `"TenantDefaultTemplate"`.
    pub struct TenantDefaultTemplate in "TenantDefaultTemplate" {
        tenant_id: i64 = "tenantId",
        template_id: i64 = "templateId",
        key_hash: String = "keyHash",
        encrypted_key: Json = "encryptedKey",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
    }
}
table_row! {
    /// Row of `"TenantDefaultTemplateEvent"`.
    pub struct TenantDefaultTemplateEvent in "TenantDefaultTemplateEvent" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        tenant_id: i64 = "tenantId",
        action: String = "action",
        template_id: i64 = "templateId",
        actor_user_id: Option<i64> = "actorUserId",
    }
}
table_row! {
    /// Row of `"TenantHelpItem"`.
    pub struct TenantHelpItem in "TenantHelpItem" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        tenant_id: i64 = "tenantId",
        name: String = "name",
        url: String = "url",
        icon: Option<String> = "icon",
        position: i64 = "position",
    }
}
table_row! {
    /// Row of `"Trigger"`.
    pub struct Trigger in "Trigger" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        active: bool = "active",
        folder_id: Option<i64> = "folderId",
        actions: Json = "actions",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"TriggerContactHistory"`.
    pub struct TriggerContactHistory in "TriggerContactHistory" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        trigger_id: i64 = "triggerId",
        contact_id: i64 = "contactId",
        workspace_id: i64 = "workspaceId",
        first_entered_at: i64 = "firstEnteredAt",
    }
}
table_row! {
    /// Row of `"TriggerExecution"`.
    pub struct TriggerExecution in "TriggerExecution" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        executed_at: i64 = "executedAt",
        trigger_id: i64 = "triggerId",
        contact_id: i64 = "contactId",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"TriggerStat"`.
    pub struct TriggerStat in "TriggerStat" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        trigger_id: i64 = "triggerId",
        workspace_id: i64 = "workspaceId",
        date: i64 = "date",
        total_contacts: i64 = "totalContacts",
        success_count: i64 = "successCount",
        failure_count: i64 = "failureCount",
        total_executions: i64 = "totalExecutions",
    }
}
table_row! {
    /// Row of `"User"`.
    pub struct User in "User" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: Option<String> = "name",
        email: String = "email",
        email_verified: bool = "emailVerified",
        image: Option<String> = "image",
        is_anonymous: bool = "isAnonymous",
        must_change_password: bool = "mustChangePassword",
        tenant_id: i64 = "tenantId",
    }
}
table_row! {
    /// Row of `"UserDeviceToken"`.
    pub struct UserDeviceToken in "UserDeviceToken" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        user_id: i64 = "userId",
        workspace_id: Option<i64> = "workspaceId",
        platform: String = "platform",
        token: String = "token",
        last_seen_at: i64 = "lastSeenAt",
    }
}
table_row! {
    /// Row of `"UserPersistentMenu"`.
    pub struct UserPersistentMenu in "UserPersistentMenu" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        menus: Json = "menus",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"UserQuota"`.
    pub struct UserQuota in "UserQuota" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        user_id: i64 = "userId",
        contacts_limit: Option<i64> = "contactsLimit",
        contacts_used: i64 = "contactsUsed",
        workspaces_limit: Option<i64> = "workspacesLimit",
        workspaces_used: i64 = "workspacesUsed",
        channels_limit: Option<i64> = "channelsLimit",
        channels_used: i64 = "channelsUsed",
        team_members_limit: Option<i64> = "teamMembersLimit",
        team_members_used: i64 = "teamMembersUsed",
        mac_limit: Option<i64> = "macLimit",
        mac_used: i64 = "macUsed",
        bot_messages_limit: Option<i64> = "botMessagesLimit",
        bot_messages_used: i64 = "botMessagesUsed",
        monthly_bot_messages_limit: Option<i64> = "monthlyBotMessagesLimit",
        monthly_bot_messages_used: i64 = "monthlyBotMessagesUsed",
        monthly_bot_messages_period_start: Option<i64> = "monthlyBotMessagesPeriodStart",
        bot_messages_top_up_granted: i64 = "botMessagesTopUpGranted",
        white_label: bool = "whiteLabel",
        sso_saml: bool = "ssoSaml",
        saas_mode: bool = "saasMode",
        custom_app_credentials: bool = "customAppCredentials",
        plan_name: Option<String> = "planName",
        plan_status: Option<String> = "planStatus",
        selected_trial_plan_id: Option<i64> = "selectedTrialPlanId",
        period_start: Option<i64> = "periodStart",
        period_end: Option<i64> = "periodEnd",
        channels_torn_down_at: Option<i64> = "channelsTornDownAt",
        synced_at: i64 = "syncedAt",
    }
}
table_row! {
    /// Row of `"Vendor"`.
    pub struct Vendor in "Vendor" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        short_description: Option<String> = "shortDescription",
        phone: Option<String> = "phone",
        store_address: Option<String> = "storeAddress",
        image_url: Option<String> = "imageUrl",
        is_active: bool = "isActive",
        discount_percent: i64 = "discountPercent",
        minimum_order_value: i64 = "minimumOrderValue",
        open_hours_id: Option<i64> = "openHoursId",
        category_id: Option<i64> = "categoryId",
        subcategory_id: Option<i64> = "subcategoryId",
        rank: i64 = "rank",
        source: String = "source",
    }
}
table_row! {
    /// Row of `"Verification"`.
    pub struct Verification in "Verification" {
        id: String = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        identifier: String = "identifier",
        value: String = "value",
        expires_at: i64 = "expiresAt",
    }
}
table_row! {
    /// Row of `"Webhook"`.
    pub struct Webhook in "Webhook" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        active: bool = "active",
        folder_id: Option<i64> = "folderId",
        url: String = "url",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"WebhookExecution"`.
    pub struct WebhookExecution in "WebhookExecution" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        executed_at: i64 = "executedAt",
        webhook_id: i64 = "webhookId",
        contact_id: i64 = "contactId",
        workspace_id: i64 = "workspaceId",
    }
}
table_row! {
    /// Row of `"WhatsappBusinessAccount"`.
    pub struct WhatsappBusinessAccount in "WhatsappBusinessAccount" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        waba_id: String = "wabaId",
        business_id: String = "businessId",
        credential: Json = "credential",
        granted_scopes: Json = "grantedScopes",
        scope_checked_at: Option<i64> = "scopeCheckedAt",
        provisioned_at: Option<i64> = "provisionedAt",
        credit_line_id: Option<String> = "creditLineId",
        revision: i64 = "revision",
    }
}
table_row! {
    /// Row of `"WhatsappCall"`.
    pub struct WhatsappCall in "WhatsappCall" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        wacid: Option<String> = "wacid",
        attempt_id: Option<String> = "attemptId",
        direction: String = "direction",
        status: String = "status",
        outcome: Option<String> = "outcome",
        started_at: Option<i64> = "startedAt",
        ended_at: Option<i64> = "endedAt",
        duration_seconds: Option<i64> = "durationSeconds",
        message_id: Option<i64> = "messageId",
        last_error: Option<String> = "lastError",
        answered_by_user_id: Option<i64> = "answeredByUserId",
        initiated_by_user_id: Option<i64> = "initiatedByUserId",
        recording_requested: Option<bool> = "recordingRequested",
        recording_failure_reason: Option<String> = "recordingFailureReason",
        recording_path: Option<String> = "recordingPath",
        recorded_at: Option<i64> = "recordedAt",
        transcript: Option<String> = "transcript",
        transcribed_at: Option<i64> = "transcribedAt",
        transcript_segments: Option<Json> = "transcriptSegments",
        ai_summary: Option<Json> = "aiSummary",
        ai_summarized_at: Option<i64> = "aiSummarizedAt",
        ai_summary_provider: Option<String> = "aiSummaryProvider",
        workspace_id: i64 = "workspaceId",
        inbox_id: i64 = "inboxId",
        contact_inbox_id: i64 = "contactInboxId",
        conversation_id: i64 = "conversationId",
    }
}
table_row! {
    /// Row of `"WhatsappCallPermission"`.
    pub struct WhatsappCallPermission in "WhatsappCallPermission" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        response: String = "response",
        is_permanent: bool = "isPermanent",
        expires_at: Option<i64> = "expiresAt",
        responded_at: i64 = "respondedAt",
        workspace_id: i64 = "workspaceId",
        contact_inbox_id: i64 = "contactInboxId",
    }
}
table_row! {
    /// Row of `"WhatsappCoexistStaging"`.
    pub struct WhatsappCoexistStaging in "WhatsappCoexistStaging" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        phone_number_id: String = "phoneNumberId",
        payload: Json = "payload",
        payload_hash: String = "payloadHash",
        processed_at: Option<i64> = "processedAt",
        parse_failed_at: Option<i64> = "parseFailedAt",
    }
}
table_row! {
    /// Row of `"WhatsappFlow"`.
    pub struct WhatsappFlow in "WhatsappFlow" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        integration_whatsapp_id: i64 = "integrationWhatsappId",
        source_id: String = "sourceId",
        status: String = "status",
        categories: Json = "categories",
        validation_errors: Json = "validationErrors",
        completed_count: i64 = "completedCount",
        screens: Json = "screens",
    }
}
table_row! {
    /// Row of `"WhatsappMessageTemplate"`.
    pub struct WhatsappMessageTemplate in "WhatsappMessageTemplate" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        integration_whatsapp_id: i64 = "integrationWhatsappId",
        source_id: String = "sourceId",
        language: String = "language",
        category: String = "category",
        status: String = "status",
        components: Json = "components",
    }
}
table_row! {
    /// Row of `"WhatsappSignupSession"`.
    pub struct WhatsappSignupSession in "WhatsappSignupSession" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        user_id: i64 = "userId",
        owner_id: i64 = "ownerId",
        workspace_id: Option<i64> = "workspaceId",
        waba_id: String = "wabaId",
        business_id: String = "businessId",
        encrypted_access_token: Json = "encryptedAccessToken",
        api_version: String = "apiVersion",
        candidate_phone_number_ids: Json = "candidatePhoneNumberIds",
        claimed_phone_number_ids: Json = "claimedPhoneNumberIds",
        expires_at: i64 = "expiresAt",
        consumed_at: Option<i64> = "consumedAt",
    }
}
table_row! {
    /// Row of `"WorkingHours"`.
    pub struct WorkingHours in "WorkingHours" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
    }
}
table_row! {
    /// Row of `"WorkingHoursDateOverride"`.
    pub struct WorkingHoursDateOverride in "WorkingHoursDateOverride" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        working_hours_id: Option<i64> = "workingHoursId",
        date: String = "date",
        name: Option<String> = "name",
        ranges: Json = "ranges",
    }
}
table_row! {
    /// Row of `"WorkingHoursInterval"`.
    pub struct WorkingHoursInterval in "WorkingHoursInterval" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        working_hours_id: i64 = "workingHoursId",
        weekday: i64 = "weekday",
        start_minute: i64 = "startMinute",
        end_minute: i64 = "endMinute",
    }
}
table_row! {
    /// Row of `"WorkingHoursMember"`.
    pub struct WorkingHoursMember in "WorkingHoursMember" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        working_hours_id: i64 = "workingHoursId",
        workspace_id: i64 = "workspaceId",
        user_id: i64 = "userId",
    }
}
table_row! {
    /// Row of `"Workspace"`.
    pub struct Workspace in "Workspace" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        name: String = "name",
        default_reply: Option<String> = "defaultReply",
        default_reply_frequency: String = "defaultReplyFrequency",
        target_country: Option<String> = "targetCountry",
        language: String = "language",
        timezone: String = "timezone",
        brand_color: String = "brandColor",
        development_mode: bool = "developmentMode",
        smart_response_delay_seconds: Option<i64> = "smartResponseDelaySeconds",
        is_active: bool = "isActive",
        start_time: Option<String> = "startTime",
        end_time: Option<String> = "endTime",
        logo: Option<String> = "logo",
        scheduled_deletion_at: Option<i64> = "scheduledDeletionAt",
        deletion_requested_by_id: Option<i64> = "deletionRequestedById",
        deletion_requested_at: Option<i64> = "deletionRequestedAt",
        purge_started_at: Option<i64> = "purgeStartedAt",
        support_access_until: Option<i64> = "supportAccessUntil",
        capi_limited_data_use: bool = "capiLimitedDataUse",
        token: Option<String> = "token",
        owner_id: i64 = "ownerId",
        tenant_id: i64 = "tenantId",
    }
}
table_row! {
    /// Row of `"WorkspaceApiToken"`.
    pub struct WorkspaceApiToken in "WorkspaceApiToken" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        name: String = "name",
        permission: String = "permission",
        token_hash: String = "tokenHash",
        token_prefix: Option<String> = "tokenPrefix",
        is_default: bool = "isDefault",
        encrypted_token: Option<Json> = "encryptedToken",
        scopes: Option<Json> = "scopes",
    }
}
table_row! {
    /// Row of `"WorkspaceBotMessageUsage"`.
    pub struct WorkspaceBotMessageUsage in "WorkspaceBotMessageUsage" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        owner_id: i64 = "ownerId",
        tenant_id: i64 = "tenantId",
        period_start: i64 = "periodStart",
        period_end: i64 = "periodEnd",
        count: i64 = "count",
    }
}
table_row! {
    /// Row of `"WorkspaceDeletionRecord"`.
    pub struct WorkspaceDeletionRecord in "WorkspaceDeletionRecord" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        workspace_name: String = "workspaceName",
        owner_id: i64 = "ownerId",
        owner_email: Option<String> = "ownerEmail",
        tenant_id: i64 = "tenantId",
        deletion_requested_by_id: Option<i64> = "deletionRequestedById",
        deletion_requested_by_email: Option<String> = "deletionRequestedByEmail",
        deletion_requested_at: Option<i64> = "deletionRequestedAt",
        scheduled_deletion_at: Option<i64> = "scheduledDeletionAt",
        purged_at: Option<i64> = "purgedAt",
        usage_snapshot: Json = "usageSnapshot",
        details: Json = "details",
    }
}
table_row! {
    /// Row of `"WorkspaceMac"`.
    pub struct WorkspaceMac in "WorkspaceMac" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        owner_id: Option<i64> = "ownerId",
        tenant_id: Option<i64> = "tenantId",
        period_start: i64 = "periodStart",
        period_end: i64 = "periodEnd",
        mac_count: i64 = "macCount",
    }
}
table_row! {
    /// Row of `"WorkspaceMember"`.
    pub struct WorkspaceMember in "WorkspaceMember" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        user_id: i64 = "userId",
        role: String = "role",
        notification_channels: Json = "notificationChannels",
        notification_types: Json = "notificationTypes",
        permissions: Json = "permissions",
        online_since: Option<i64> = "onlineSince",
    }
}
table_row! {
    /// Row of `"WorkspaceUsage"`.
    pub struct WorkspaceUsage in "WorkspaceUsage" {
        id: i64 = "id",
        created_at: i64 = "createdAt",
        updated_at: i64 = "updatedAt",
        workspace_id: i64 = "workspaceId",
        contacts_used: i64 = "contactsUsed",
        channels_used: i64 = "channelsUsed",
        team_members_used: i64 = "teamMembersUsed",
        bot_messages_used: i64 = "botMessagesUsed",
        mac_used: i64 = "macUsed",
        synced_at: i64 = "syncedAt",
    }
}
