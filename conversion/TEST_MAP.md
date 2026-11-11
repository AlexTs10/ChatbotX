# Test map

Filled in by P0.2 (list), P0.3 (`baseline-fail`), and P0.5 (class).

Classes: **A** black-box parity (`conversion/parity/`), **B** frontend (TypeScript, unchanged), **C** backend unit (ported 1:1 to Rust `#[test]`, same name in snake_case).
Status values: `todo`, `ported`, `passing`, `baseline-fail`, `dropped`, `disputed` (needs user approval).

| Original test file | Class | New test location | Status | Feature; note |
|--------------------|-------|-------------------|--------|---------------|
| `apps/builder/__tests__/__snapshots__/plain-text-tiptap-editor-dom.test.tsx.snap` | B | - | todo | Variables (contact/bot/system variable interpolation); builder jsdom |
| `apps/builder/__tests__/__snapshots__/public-spec-mcp.test.ts.snap` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/__snapshots__/public-spec-operations.test.ts.snap` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/accept-invitation-action.test.ts` | C | tbd | todo | Workspace members, roles and invitations; builder server code |
| `apps/builder/__tests__/action-prompt-popover.test.tsx` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/__tests__/activity-reference-value-field.test.tsx` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/__tests__/ad-account-filter.test.tsx` | B | - | todo | Facebook Ads integration; builder jsdom |
| `apps/builder/__tests__/add-action-allowed-types.test.tsx` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/__tests__/add-channel-button.test.tsx` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/add-condition-order.test.tsx` | B | - | todo | Ecommerce: orders and payment providers; builder jsdom |
| `apps/builder/__tests__/add-contact-custom-field.action.test.ts` | C | tbd | todo | Custom fields; builder server code |
| `apps/builder/__tests__/admin-workspaces-queries.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/ads-account-filter.test.tsx` | B | - | todo | Ads campaigns (Meta messaging ads); builder jsdom |
| `apps/builder/__tests__/ads-account-switcher.test.tsx` | B | - | todo | Ads campaigns (Meta messaging ads); builder jsdom |
| `apps/builder/__tests__/ads-analytics-export-route.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking; builder server code |
| `apps/builder/__tests__/ads-analytics-view-retarget.test.tsx` | B | - | todo | Ads analytics and CTWA tracking; builder jsdom |
| `apps/builder/__tests__/ads-analytics-view-revenue-delivery.test.tsx` | B | - | todo | Ads analytics and CTWA tracking; builder jsdom |
| `apps/builder/__tests__/ads-api.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/ads-campaign-creative-media-schema.test.ts` | B | - | todo | Ads campaigns (Meta messaging ads); builder jsdom |
| `apps/builder/__tests__/ads-campaign-insights-api.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/ads-campaign-messenger-pages-schema.test.ts` | B | - | todo | Messenger channel; builder jsdom |
| `apps/builder/__tests__/ads-campaign-public-api.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/ads-campaign-public-extras.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/ads-campaign-wizard.test.ts` | B | - | todo | Ads campaigns (Meta messaging ads); builder jsdom |
| `apps/builder/__tests__/ads-conversion-rule-actions.test.ts` | C | tbd | todo | Google Ads integration; builder server code |
| `apps/builder/__tests__/ads-creative-image-upload.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/ads-dashboard-channel-page.test.tsx` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/ads-dashboard-channels.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/ads-dashboard-redirect-page.test.ts` | B | - | todo | Ads campaigns (Meta messaging ads); builder node-env, client logic |
| `apps/builder/__tests__/ads-date-key.test.ts` | B | - | todo | Ads analytics and CTWA tracking; builder jsdom |
| `apps/builder/__tests__/ads-performance-chart.test.tsx` | B | - | todo | Ads analytics and CTWA tracking; builder jsdom |
| `apps/builder/__tests__/ads-public-api.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/ads-public-scope.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/ads-referral-stats.test.tsx` | B | - | todo | Ads analytics and CTWA tracking; builder jsdom |
| `apps/builder/__tests__/ads-select-account.test.ts` | B | - | todo | Ads campaigns (Meta messaging ads); builder jsdom |
| `apps/builder/__tests__/ai-actions-field.test.tsx` | B | - | todo | AI flow steps (text, image, speech, extraction); builder jsdom |
| `apps/builder/__tests__/ai-agent-action-options.test.ts` | B | - | todo | AI agents (tools, MCP client, functions); builder jsdom |
| `apps/builder/__tests__/ai-agent-action-schema.test.ts` | B | - | todo | AI agents (tools, MCP client, functions); builder jsdom |
| `apps/builder/__tests__/ai-agent-openai-compatible-models.test.ts` | B | - | todo | OpenAI-compatible integration; builder jsdom |
| `apps/builder/__tests__/ai-agents-public-api.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions); builder server code |
| `apps/builder/__tests__/ai-files-public-api.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files); builder server code |
| `apps/builder/__tests__/ai-files-reindex-api.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files); builder server code |
| `apps/builder/__tests__/ai-files-resource-schema.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files); builder server code |
| `apps/builder/__tests__/ai-files-search-api.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files); builder server code |
| `apps/builder/__tests__/ai-functions-public-api.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions); builder server code |
| `apps/builder/__tests__/ai-handover-apply-to-all.test.tsx` | B | - | todo | AI handover (human takeover); builder jsdom |
| `apps/builder/__tests__/ai-handover-bulk-actions.test.ts` | C | tbd | todo | AI handover (human takeover); builder server code |
| `apps/builder/__tests__/ai-handover-bulk-schema.test.ts` | B | - | todo | AI handover (human takeover); builder jsdom |
| `apps/builder/__tests__/ai-handover-history-dialog.test.tsx` | B | - | todo | AI handover (human takeover); builder jsdom |
| `apps/builder/__tests__/ai-handover-public-api.test.ts` | C | tbd | todo | AI handover (human takeover); builder server code |
| `apps/builder/__tests__/ai-handover-settings-card.test.tsx` | B | - | todo | AI handover (human takeover); builder jsdom |
| `apps/builder/__tests__/ai-handover-settings-request-schema.test.ts` | B | - | todo | AI handover (human takeover); builder node-env, client logic |
| `apps/builder/__tests__/ai-mcp-server-bot-field-variable.test.ts` | B | - | todo | AI agents (tools, MCP client, functions); builder jsdom |
| `apps/builder/__tests__/ai-mcp-servers-public-api.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions); builder server code |
| `apps/builder/__tests__/ai-mcp-servers-public-resource.test.ts` | B | - | todo | AI agents (tools, MCP client, functions); builder jsdom |
| `apps/builder/__tests__/ai-segment-image-step.test.tsx` | B | - | todo | AI flow steps (text, image, speech, extraction); builder jsdom |
| `apps/builder/__tests__/analytics-link-stats-schema.test.ts` | B | - | todo | Analytics dashboards; builder node-env, client logic |
| `apps/builder/__tests__/analytics-nav.test.tsx` | B | - | todo | Analytics dashboards; builder jsdom |
| `apps/builder/__tests__/analytics-pages-google-nav.test.tsx` | C | tbd | todo | Analytics dashboards; builder server code |
| `apps/builder/__tests__/analytics-public-api.test.ts` | C | tbd | todo | Analytics dashboards; builder server code |
| `apps/builder/__tests__/analytics-public-scope.test.ts` | C | tbd | todo | Analytics dashboards; builder server code |
| `apps/builder/__tests__/api-idempotency-middleware.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/api-idempotency.test.ts` | B | - | todo | Workspace API tokens and public API; builder node-env, client logic |
| `apps/builder/__tests__/api-parity-manifest.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/api-rate-limit.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/app-tab.test.tsx` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/__tests__/appointment-calendars-private-availability.test.ts` | C | tbd | todo | Appointments and booking; builder server code |
| `apps/builder/__tests__/appointment-calendars-public-api.test.ts` | C | tbd | todo | Appointments and booking; builder server code |
| `apps/builder/__tests__/appointment-external-calendars-public-api.test.ts` | C | tbd | todo | Google Calendar integration; builder server code |
| `apps/builder/__tests__/appointment-management-actions.test.ts` | C | tbd | todo | Appointments and booking; builder server code |
| `apps/builder/__tests__/appointment-management-inbox-actions.test.ts` | C | tbd | todo | Appointments and booking; builder server code |
| `apps/builder/__tests__/appointment-reminders-public-api.test.ts` | C | tbd | todo | Appointments and booking; builder server code |
| `apps/builder/__tests__/appointments-public-api.test.ts` | C | tbd | todo | Appointments and booking; builder server code |
| `apps/builder/__tests__/appointments-public-scope.test.ts` | C | tbd | todo | Appointments and booking; builder server code |
| `apps/builder/__tests__/attachment-url.test.ts` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/audit-logs-query.test.ts` | C | tbd | todo | Audit logs; builder server code |
| `apps/builder/__tests__/auth-oauth-error.test.ts` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/auth-page-options.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links); builder server code |
| `apps/builder/__tests__/auth-redirect.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links); builder server code |
| `apps/builder/__tests__/auth-route.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links); builder server code |
| `apps/builder/__tests__/authorize-workspace-access.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/automated-response-type-scope.test.ts` | C | tbd | todo | Automated responses and keywords; builder server code |
| `apps/builder/__tests__/automation-public-scope.test.ts` | C | tbd | todo | Automated responses and keywords; builder server code |
| `apps/builder/__tests__/booking-webview-pages.test.tsx` | C | tbd | todo | Appointments and booking; builder server code |
| `apps/builder/__tests__/bot-block-conditions-schema.test.ts` | B | - | todo | Bot fields; builder node-env, client logic |
| `apps/builder/__tests__/bot-fields-public-api.test.ts` | C | tbd | todo | Bot fields; builder server code |
| `apps/builder/__tests__/bot-fields-public-id-schema.test.ts` | C | tbd | todo | Bot fields; builder server code |
| `apps/builder/__tests__/bot-simulator-link.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/__tests__/bot-simulator-public-api.test.ts` | C | tbd | todo | Flow builder and execution; builder server code |
| `apps/builder/__tests__/bounded-seen-set.test.ts` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/__tests__/broadcast-audience-preview-dialog-send-limit.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-audience-preview-request.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-confirm-dialog.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-detail-dialog-recurrence.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-detail-dialog.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-draft-actions.test.ts` | C | tbd | todo | Broadcasts (incl. recurring); builder server code |
| `apps/builder/__tests__/broadcast-estimated-contacts-display.test.ts` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/__tests__/broadcast-flow-targets.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-flow-type-selector.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-plan-limit-dialog.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-plan-limit-orpc-contract.test.ts` | C | tbd | todo | Broadcasts (incl. recurring); builder server code |
| `apps/builder/__tests__/broadcast-plan-limit-outcome.test.ts` | C | tbd | todo | Broadcasts (incl. recurring); builder server code |
| `apps/builder/__tests__/broadcast-public-resource-send-limit.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-public-template-params.test.ts` | C | tbd | todo | Template marketplace (install/share/sync); builder server code |
| `apps/builder/__tests__/broadcast-recurrence-request.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-row-actions.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-run-upcoming.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-runs-view.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-schedule-type-options.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-send-limit-fields.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-send-limit-lib.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-series-summary.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-status-actions.test.ts` | C | tbd | todo | Broadcasts (incl. recurring); builder server code |
| `apps/builder/__tests__/broadcast-status-badge.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-status-config.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-status-panel.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-target-card.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcast-template-targets.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcasts-calendar-grid.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcasts-calendar.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcasts-empty-state-predicate.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcasts-public-api.test.ts` | C | tbd | todo | Broadcasts (incl. recurring); builder server code |
| `apps/builder/__tests__/broadcasts-public-scope.test.ts` | C | tbd | todo | Broadcasts (incl. recurring); builder server code |
| `apps/builder/__tests__/broadcasts-search-params.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcasts-search-parsers-server-safe.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcasts-table-toolbar-actions.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/broadcasts-toolbar.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/build-conversation-where-cursor.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/build-conversation-where-sla.test.ts` | C | tbd | todo | Response SLA; builder server code |
| `apps/builder/__tests__/build-conversation-where-unread.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/build-redirect-search.test.ts` | B | - | todo | Builder app shell and shared components; builder node-env, client logic |
| `apps/builder/__tests__/bulk-tag-stats-contacts-action-behavior.test.ts` | C | tbd | todo | Tags and labels; builder server code |
| `apps/builder/__tests__/bulk-tag-stats-contacts-action.test.ts` | B | - | todo | Tags and labels; builder node-env, client logic |
| `apps/builder/__tests__/calling-action-client-gates.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/calls-filter-bar-selects.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/calls-page-access-gate.test.tsx` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/calls-page-client.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/calls-page-components.test.tsx` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/calls-page-invalid-filter-params.test.tsx` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/calls-table.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/capabilities-public-api.test.ts` | C | tbd | todo | Capabilities discovery; builder server code |
| `apps/builder/__tests__/capi-ads-pages.test.tsx` | C | tbd | todo | Meta Conversions API (CAPI); builder server code |
| `apps/builder/__tests__/capi-channel-tabs.test.tsx` | B | - | todo | Meta Conversions API (CAPI); builder jsdom |
| `apps/builder/__tests__/capi-connect-button.test.tsx` | B | - | todo | Meta Conversions API (CAPI); builder jsdom |
| `apps/builder/__tests__/capi-connect-card.test.tsx` | B | - | todo | Meta Conversions API (CAPI); builder jsdom |
| `apps/builder/__tests__/capi-connect-result.test.ts` | B | - | todo | Meta Conversions API (CAPI); builder node-env, client logic |
| `apps/builder/__tests__/capi-connection-tab.test.tsx` | B | - | todo | Meta Conversions API (CAPI); builder jsdom |
| `apps/builder/__tests__/capi-operations.test.ts` | C | tbd | todo | Meta Conversions API (CAPI); builder server code |
| `apps/builder/__tests__/capi-status.test.ts` | B | - | todo | Meta Conversions API (CAPI); builder jsdom |
| `apps/builder/__tests__/capi-tab-chain.test.tsx` | B | - | todo | Meta Conversions API (CAPI); builder jsdom |
| `apps/builder/__tests__/capi-test-event-actions.test.ts` | C | tbd | todo | Meta Conversions API (CAPI); builder server code |
| `apps/builder/__tests__/capi-test-event-card.test.tsx` | B | - | todo | Meta Conversions API (CAPI); builder jsdom |
| `apps/builder/__tests__/capi-whatsapp-connect-actions.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/capi-whatsapp-oauth-connect.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/category-resource-list-selection.test.tsx` | B | - | todo | Ecommerce: products, categories, vendors; builder jsdom |
| `apps/builder/__tests__/category-tree.test.ts` | B | - | todo | Ecommerce: products, categories, vendors; builder jsdom |
| `apps/builder/__tests__/change-password-schema.test.ts` | B | - | todo | Authentication (sign-in, social, mobile, magic links); builder jsdom |
| `apps/builder/__tests__/channel-connect-credential-consistency.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/channel-integrations-public-api.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/channel-post-options-api.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/channel-reconnect-actions.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/channel-reconnect-handlers.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/channel-route-guards.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/channel-settings-patch.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/channel-settings-public-api.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/channels-and-integrations-public-scope.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/channels-create-messenger-handoff.test.ts` | C | tbd | todo | AI handover (human takeover); builder server code |
| `apps/builder/__tests__/channels-create-messenger-reuse.test.ts` | C | tbd | todo | Messenger channel; builder server code |
| `apps/builder/__tests__/channels-create-platform-owner.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/channels-create-visibility-guard.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/channels-instagram-routes.test.ts` | C | tbd | todo | Instagram channel; builder server code |
| `apps/builder/__tests__/channels-public-scope.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/chat-layout-mobile.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/chat-realtime.test.tsx` | B | - | todo | Realtime (WebSocket gateway); builder jsdom |
| `apps/builder/__tests__/chat-store-provider.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/chat-store-update-contact.test.ts` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/__tests__/chat-store.test.ts` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/client-cookies.test.ts` | B | - | todo | Authentication (sign-in, social, mobile, magic links); builder jsdom |
| `apps/builder/__tests__/client-imports-no-backend-packages.test.ts` | C | tbd | todo | Builder app shell and shared components; builder server code |
| `apps/builder/__tests__/client-imports-no-queries.test.ts` | B | - | todo | Builder app shell and shared components; builder node-env, client logic |
| `apps/builder/__tests__/coexist-client.test.ts` | B | - | todo | Coexist (existing-app history sync); builder jsdom |
| `apps/builder/__tests__/coexist-messenger-api.test.ts` | C | tbd | todo | Coexist (existing-app history sync); builder server code |
| `apps/builder/__tests__/coexist-row-trailing.test.tsx` | B | - | todo | Coexist (existing-app history sync); builder jsdom |
| `apps/builder/__tests__/coexist-run.test.ts` | B | - | todo | Coexist (existing-app history sync); builder node-env, client logic |
| `apps/builder/__tests__/coexist-step.test.tsx` | B | - | todo | Coexist (existing-app history sync); builder jsdom |
| `apps/builder/__tests__/coexist-whatsapp-api.test.ts` | C | tbd | todo | Coexist (existing-app history sync); builder server code |
| `apps/builder/__tests__/comment-automation-analytics-public-api.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/comment-automations-public-api.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/comment-moderation-logs.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/comment-private-reply-gate.test.ts` | B | - | todo | Comment automation (FB/IG/Threads/TikTok); builder jsdom |
| `apps/builder/__tests__/connect-action-outcomes.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/connect-capi-actions.test.ts` | C | tbd | todo | Meta Conversions API (CAPI); builder server code |
| `apps/builder/__tests__/connect-channel-registry.test.ts` | B | - | todo | Channel connection and OAuth connect flow; builder node-env, client logic |
| `apps/builder/__tests__/connect-client.test.ts` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/connect-error-code.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/connect-flow-return-url.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/connect-flow.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/connect-many-dialog.a11y.test.tsx` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/connect-many-dialog.footer-steps.test.tsx` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/connect-many-dialog.rows.test.tsx` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/connect-picker-card-width.test.tsx` | B | - | todo | Channel connection and OAuth connect flow; builder node-env, client logic |
| `apps/builder/__tests__/connect-pool-ceiling.test.ts` | B | - | todo | Channel connection and OAuth connect flow; builder node-env, client logic |
| `apps/builder/__tests__/connect-routes.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/connect-row-status.test.tsx` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/__tests__/connect-selection-form.test.tsx` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/connect-session-message.test.ts` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/connection-status-message.test.ts` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/connections-private-api.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/connections-public-api.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/connections-public-permission.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/connections-public-spec-flat.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/contact-activity-filter-schema.test.ts` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/__tests__/contact-ad-referrals-list.test.tsx` | B | - | todo | Ads analytics and CTWA tracking; builder jsdom |
| `apps/builder/__tests__/contact-avatar-rendering.test.tsx` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/__tests__/contact-custom-field-manage.test.tsx` | B | - | todo | Custom fields; builder jsdom |
| `apps/builder/__tests__/contact-custom-field-schema.test.ts` | B | - | todo | Custom fields; builder jsdom |
| `apps/builder/__tests__/contact-detail-call-button.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/contact-detail-custom-fields-reset.test.tsx` | B | - | todo | Custom fields; builder jsdom |
| `apps/builder/__tests__/contact-export-files.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows); builder server code |
| `apps/builder/__tests__/contact-filter-condition-draft.test.ts` | C | tbd | todo | Contact filters and segments; builder server code |
| `apps/builder/__tests__/contact-filter-config.test.ts` | C | tbd | todo | Contact filters and segments; builder server code |
| `apps/builder/__tests__/contact-filter-pipeline-item.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/contact-filter-query-schema.test.ts` | B | - | todo | Contact filters and segments; builder node-env, client logic |
| `apps/builder/__tests__/contact-filter-query-state.test.tsx` | B | - | todo | Contact filters and segments; builder jsdom |
| `apps/builder/__tests__/contact-filter-schema.test.ts` | C | tbd | todo | Contact filters and segments; builder server code |
| `apps/builder/__tests__/contact-filter-summary.test.tsx` | B | - | todo | Contact filters and segments; builder jsdom |
| `apps/builder/__tests__/contact-filter-value-inputs.test.tsx` | B | - | todo | Contact filters and segments; builder jsdom |
| `apps/builder/__tests__/contact-inbox-panel.test.tsx` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/__tests__/contact-inbox-resource.test.ts` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/__tests__/contact-scan-form.test.tsx` | B | - | todo | Contact scan; builder jsdom |
| `apps/builder/__tests__/contact-scan-histories-page.test.tsx` | C | tbd | todo | Contact scan; builder server code |
| `apps/builder/__tests__/contact-scan-history-table.test.tsx` | B | - | todo | Contact scan; builder jsdom |
| `apps/builder/__tests__/contact-scan-public-api.test.ts` | C | tbd | todo | Contact scan; builder server code |
| `apps/builder/__tests__/contact-scan-status-api.test.ts` | C | tbd | todo | Contact scan; builder server code |
| `apps/builder/__tests__/contact-search-combobox.test.tsx` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/__tests__/contact-subresource-authz.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/contact-thread-control-section.test.tsx` | B | - | todo | Thread control (handover protocol); builder jsdom |
| `apps/builder/__tests__/contacts-ad-referrals-public-api.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking; builder server code |
| `apps/builder/__tests__/contacts-authenticated-api-get-contact.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/contacts-avatar-resolution.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/contacts-bulk-public-api.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/contacts-crud-public-api.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/contacts-export-files-public-api.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows); builder server code |
| `apps/builder/__tests__/contacts-export-public-api.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows); builder server code |
| `apps/builder/__tests__/contacts-filter-fields-public-api.test.ts` | C | tbd | todo | Contact filters and segments; builder server code |
| `apps/builder/__tests__/contacts-import-public-api.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows); builder server code |
| `apps/builder/__tests__/contacts-import-template-route.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows); builder server code |
| `apps/builder/__tests__/contacts-import-template.test.ts` | B | - | todo | Template marketplace (install/share/sync); builder node-env, client logic |
| `apps/builder/__tests__/contacts-inboxes-public-api.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/contacts-last-read.test.ts` | B | - | todo | Contacts (profiles, notes, activity); builder node-env, client logic |
| `apps/builder/__tests__/contacts-list-action.test.tsx` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/__tests__/contacts-messages-public-api.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/contacts-notes-public-api.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/contacts-permissions.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/contacts-public-filter-strict.test.ts` | B | - | todo | Contact filters and segments; builder node-env, client logic |
| `apps/builder/__tests__/contacts-public-scope.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/contacts-refresh-profile-public-api.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/contacts-route-guards.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/contacts-sequences-public-api.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/contacts-table-api.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/contacts-table-query-state.test.tsx` | B | - | todo | Contacts (profiles, notes, activity); builder node-env, client logic |
| `apps/builder/__tests__/contacts-tags-and-fields-public-api.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/conversation-action-unread.test.tsx` | B | - | todo | Flow steps: contact and conversation actions; builder jsdom |
| `apps/builder/__tests__/conversation-contact-inbox-resource.test.ts` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/__tests__/conversation-item.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/conversation-list.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/conversation-routing-card.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/conversations-authenticated-api-block-gate.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/conversations-public-api.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/conversations-public-scope.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/conversations-thread-control-public-api.test.ts` | C | tbd | todo | Thread control (handover protocol); builder server code |
| `apps/builder/__tests__/conversion-events-account-reset.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/conversion-events-rule-types.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/count-contact-inboxes.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/coupons-public-api.test.ts` | C | tbd | todo | Coupons and discount codes; builder server code |
| `apps/builder/__tests__/coupons-public-scope.test.ts` | C | tbd | todo | Coupons and discount codes; builder server code |
| `apps/builder/__tests__/create-admin-workspace-action.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/create-ai-file-action.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files); builder server code |
| `apps/builder/__tests__/create-api.action.test.ts` | C | tbd | todo | API channel integration (custom channels); builder server code |
| `apps/builder/__tests__/create-automated-response-action.test.ts` | C | tbd | todo | Automated responses and keywords; builder server code |
| `apps/builder/__tests__/create-broadcast-defaults-send-limit.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/create-broadcast-flow-filter.test.ts` | C | tbd | todo | Broadcasts (incl. recurring); builder server code |
| `apps/builder/__tests__/create-broadcast-form-plan-limit.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/create-broadcast-mpm-validation.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/create-broadcast-request-draft.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/create-broadcast-send-limit-request.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/create-broadcast.action.test.ts` | C | tbd | todo | Broadcasts (incl. recurring); builder server code |
| `apps/builder/__tests__/create-contact-action.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/create-contact-request-whatsapp-identity.test.ts` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/__tests__/create-first-workspace.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/create-flow-action.test.ts` | C | tbd | todo | Flow builder and execution; builder server code |
| `apps/builder/__tests__/create-flow-request-schema.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/__tests__/create-message-action.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/create-message-schema.test.ts` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/create-sequence.action.test.ts` | C | tbd | todo | Sequences; builder server code |
| `apps/builder/__tests__/create-template-dialog.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/create-trigger-action.test.ts` | C | tbd | todo | Triggers; builder server code |
| `apps/builder/__tests__/create-webchat-message-action.test.ts` | C | tbd | todo | Webchat channel; builder server code |
| `apps/builder/__tests__/create-webhook-action.test.ts` | C | tbd | todo | Outbound webhooks; builder server code |
| `apps/builder/__tests__/create-workspace-token-action.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/custom-field-value-input.test.tsx` | B | - | todo | Custom fields; builder jsdom |
| `apps/builder/__tests__/date-time-picker-form-value-conversion.test.ts` | B | - | todo | Custom fields; builder jsdom |
| `apps/builder/__tests__/datetime-picker-page.test.tsx` | C | tbd | todo | Custom fields; builder server code |
| `apps/builder/__tests__/delete-ai-agent-action.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions); builder server code |
| `apps/builder/__tests__/delete-broadcast-dialog.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/delete-contact-action.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/delete-me-data-action.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/delete-message-action.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/delete-mini-apps-whatsapp-flows.test.ts` | C | tbd | todo | Mini-apps; builder server code |
| `apps/builder/__tests__/delete-sequence-step.action.test.ts` | C | tbd | todo | Sequences; builder server code |
| `apps/builder/__tests__/delete-sequence.action.test.ts` | C | tbd | todo | Sequences; builder server code |
| `apps/builder/__tests__/delete-tag-action.test.ts` | C | tbd | todo | Tags and labels; builder server code |
| `apps/builder/__tests__/delete-webhooks-action.test.ts` | C | tbd | todo | Outbound webhooks; builder server code |
| `apps/builder/__tests__/delete-workspace-token-action.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/disconnect-meta-actions.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/disconnect-whatsapp-action.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/discount-codes-public-api.test.ts` | C | tbd | todo | Coupons and discount codes; builder server code |
| `apps/builder/__tests__/download-flow-export.test.ts` | B | - | todo | Imports and exports (contacts, products, flows); builder jsdom |
| `apps/builder/__tests__/dynamic-images-public-api.test.ts` | C | tbd | todo | Dynamic images; builder server code |
| `apps/builder/__tests__/ecommerce-analytics-public-api.test.ts` | C | tbd | todo | Ecommerce: settings, open hours, analytics; builder server code |
| `apps/builder/__tests__/ecommerce-open-hours-public-api.test.ts` | C | tbd | todo | Ecommerce: settings, open hours, analytics; builder server code |
| `apps/builder/__tests__/ecommerce-public-scope.test.ts` | C | tbd | todo | Ecommerce: settings, open hours, analytics; builder server code |
| `apps/builder/__tests__/ecommerce-settings-legacy-keys.test.ts` | B | - | todo | Ecommerce: settings, open hours, analytics; builder jsdom |
| `apps/builder/__tests__/edge-instrumentation-graph.test.ts` | C | tbd | todo | Builder app shell and shared components; builder server code |
| `apps/builder/__tests__/edit-message-action.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/email-topic-click-route.test.ts` | C | tbd | todo | Email topics and tracking; builder server code |
| `apps/builder/__tests__/email-topic-open-route.test.ts` | C | tbd | todo | Email topics and tracking; builder server code |
| `apps/builder/__tests__/email-topics-columns.test.ts` | B | - | todo | Email topics and tracking; builder node-env, client logic |
| `apps/builder/__tests__/email-topics-public-api.test.ts` | C | tbd | todo | Email topics and tracking; builder server code |
| `apps/builder/__tests__/email-topics-public-scope.test.ts` | C | tbd | todo | Email topics and tracking; builder server code |
| `apps/builder/__tests__/email-topics-public-sort.test.ts` | B | - | todo | Email topics and tracking; builder node-env, client logic |
| `apps/builder/__tests__/ensure-live-comments-subscription.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/enterprise-feature-gates.test.ts` | C | tbd | todo | Multi-tenancy (white-label tenants); builder server code |
| `apps/builder/__tests__/error-log-google-ads-detail.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/error-logs-public-scope.test.ts` | C | tbd | todo | Error logs; builder server code |
| `apps/builder/__tests__/event-label.test.ts` | B | - | todo | Inbox and live chat (conversations, messages); builder node-env, client logic |
| `apps/builder/__tests__/export-contacts-action.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows); builder server code |
| `apps/builder/__tests__/export-poll.test.ts` | B | - | todo | Polls; builder jsdom |
| `apps/builder/__tests__/facebook-ads-ad-accounts-section.test.tsx` | B | - | todo | Facebook Ads integration; builder jsdom |
| `apps/builder/__tests__/facebook-lead-ad-automation-create-lib.test.ts` | C | tbd | todo | Facebook Lead Ads automation; builder server code |
| `apps/builder/__tests__/facebook-lead-ad-automation-public-api.test.ts` | C | tbd | todo | Facebook Lead Ads automation; builder server code |
| `apps/builder/__tests__/facebook-sso-reuse.test.ts` | C | tbd | todo | Facebook Ads integration; builder server code |
| `apps/builder/__tests__/fb-comments-facebook-posts-api.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/fb-comments-public-api.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/fetch-all-pages.test.ts` | B | - | todo | Builder app shell and shared components; builder node-env, client logic |
| `apps/builder/__tests__/filter-value-labels.test.ts` | B | - | todo | Contact filters and segments; builder jsdom |
| `apps/builder/__tests__/find-contact-inbox-by-channel.test.ts` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/__tests__/find-qr-code-query.test.ts` | C | tbd | todo | Public links (open-link, QR codes, reflinks); builder server code |
| `apps/builder/__tests__/fix-whatsapp-calls-subscription-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/flow-button-edge-read-only.test.tsx` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/__tests__/flow-export-route.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows); builder server code |
| `apps/builder/__tests__/flow-node-toolbar-read-only.test.tsx` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/__tests__/flow-publish-schema.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/__tests__/flow-validation-message.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/__tests__/flow-wrapper-read-only.test.tsx` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/__tests__/flows-private-messenger-ads-json.test.ts` | C | tbd | todo | Messenger channel; builder server code |
| `apps/builder/__tests__/flows-public-api.test.ts` | C | tbd | todo | Flow builder and execution; builder server code |
| `apps/builder/__tests__/flows-public-scope.test.ts` | C | tbd | todo | Flow builder and execution; builder server code |
| `apps/builder/__tests__/folders-public-api.test.ts` | C | tbd | todo | Flow builder and execution; builder server code |
| `apps/builder/__tests__/format-call-duration.test.ts` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/format-error-content.test.ts` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/__tests__/full-bleed.test.tsx` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/__tests__/generate-call-ai-summary-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/generate-call-summary.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/get-call-recording-url-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/get-call-summary-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/get-call-transcript-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/get-contact.query.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/get-inbox-initial-state.test.ts` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/get-outbound-voip-answer-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/get-pending-incoming-voip-call-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/get-post-details-query.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/get-user-data-menu-default.test.ts` | B | - | todo | Get user data webview; builder jsdom |
| `apps/builder/__tests__/get-user-data-webview-calendar-grid.test.ts` | B | - | todo | Get user data webview; builder jsdom |
| `apps/builder/__tests__/google-ads-account-picker.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-actions.test.ts` | C | tbd | todo | Google Ads integration; builder server code |
| `apps/builder/__tests__/google-ads-api.test.ts` | C | tbd | todo | Google Ads integration; builder server code |
| `apps/builder/__tests__/google-ads-connect-error-alert.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-connect-url.test.ts` | C | tbd | todo | Google Ads integration; builder server code |
| `apps/builder/__tests__/google-ads-connection-row.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-consent-section.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-consent-view.test.ts` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-conversion-actions-section.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-conversion-fields.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-conversion-options.test.ts` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-conversion-summary.test.ts` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-customer-matching.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-dashboard-entry.test.ts` | C | tbd | todo | Google Ads integration; builder server code |
| `apps/builder/__tests__/google-ads-enum-labels.test.ts` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-events-history.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-load-settings-page.test.ts` | C | tbd | todo | Google Ads integration; builder server code |
| `apps/builder/__tests__/google-ads-load-settings-session.test.ts` | C | tbd | todo | Google Ads integration; builder server code |
| `apps/builder/__tests__/google-ads-notice-alert.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-public-api.test.ts` | C | tbd | todo | Google Ads integration; builder server code |
| `apps/builder/__tests__/google-ads-select-field.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-settings-actions.test.ts` | C | tbd | todo | Google Ads integration; builder server code |
| `apps/builder/__tests__/google-ads-settings-view.test.ts` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-settings.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-stats-error.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-stats-filters-hydration.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-stats-page.test.tsx` | C | tbd | todo | Google Ads integration; builder server code |
| `apps/builder/__tests__/google-ads-stats-preset.test.ts` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-stats-view.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-status-maps.test.ts` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-template-field.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-to-event-resource.test.ts` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-ads-validate-dialog.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/__tests__/google-settings-actions.test.ts` | C | tbd | todo | Google Calendar integration; builder server code |
| `apps/builder/__tests__/google-sheets-disconnect-action.test.ts` | C | tbd | todo | Google Sheets integration; builder server code |
| `apps/builder/__tests__/guest-rate-limit-key.test.ts` | B | - | todo | Webchat channel; builder node-env, client logic |
| `apps/builder/__tests__/guest-realtime-token-route.test.ts` | C | tbd | todo | Realtime (WebSocket gateway); builder server code |
| `apps/builder/__tests__/html-to-plain-text.test.ts` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/__tests__/i18n-messages.test.ts` | B | - | todo | i18n (UI translations); builder node-env, client logic |
| `apps/builder/__tests__/i18n-request-config.test.ts` | C | tbd | todo | i18n (UI translations); builder server code |
| `apps/builder/__tests__/i18n-source-keys.test.ts` | B | - | todo | i18n (UI translations); builder node-env, client logic |
| `apps/builder/__tests__/i18n-zh-cn-parity.test.ts` | B | - | todo | i18n (UI translations); builder node-env, client logic |
| `apps/builder/__tests__/idempotency-read-exemption.test.ts` | B | - | todo | Workspace API tokens and public API; builder node-env, client logic |
| `apps/builder/__tests__/ig-comment-form-capability-gating.test.tsx` | B | - | todo | Comment automation (FB/IG/Threads/TikTok); builder jsdom |
| `apps/builder/__tests__/ig-comments-instagram-media.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/ig-comments-public-api.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/ig-stories-public-api.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/import-contacts-action.test.ts` | B | - | todo | Imports and exports (contacts, products, flows); builder node-env, client logic |
| `apps/builder/__tests__/import-upload-url-public-api.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows); builder server code |
| `apps/builder/__tests__/inbound-webhook-actions.test.ts` | C | tbd | todo | Inbound webhooks; builder server code |
| `apps/builder/__tests__/inbound-webhook-analytics-page.test.tsx` | C | tbd | todo | Inbound webhooks; builder server code |
| `apps/builder/__tests__/inbound-webhook-contact-lookup-section.test.tsx` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/__tests__/inbound-webhook-endpoint-url.test.ts` | B | - | todo | Inbound webhooks; builder jsdom |
| `apps/builder/__tests__/inbound-webhook-form.test.ts` | B | - | todo | Inbound webhooks; builder jsdom |
| `apps/builder/__tests__/inbound-webhook-json-picker.test.ts` | B | - | todo | Inbound webhooks; builder jsdom |
| `apps/builder/__tests__/inbound-webhook-route.test.ts` | C | tbd | todo | Inbound webhooks; builder server code |
| `apps/builder/__tests__/inbound-webhook-schema.test.ts` | B | - | todo | Inbound webhooks; builder jsdom |
| `apps/builder/__tests__/inbound-webhook-secret-url-input.test.tsx` | B | - | todo | Inbound webhooks; builder jsdom |
| `apps/builder/__tests__/inbound-webhooks-public-api.test.ts` | C | tbd | todo | Inbound webhooks; builder server code |
| `apps/builder/__tests__/inbox-card-list-linkless-channels.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/inbox-mark-read-on-outbound-switch.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/inbox-route-guards.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/inbox-select-card-error-flash.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/inbox-teams-public-api.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/inboxes-public-api.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/instagram-accounts.test.tsx` | B | - | todo | Instagram channel; builder jsdom |
| `apps/builder/__tests__/instagram-facebook-select-page.test.tsx` | C | tbd | todo | Instagram channel; builder server code |
| `apps/builder/__tests__/instagram-facebook-settings-actions.test.ts` | C | tbd | todo | Instagram channel; builder server code |
| `apps/builder/__tests__/instagram-select-account-action.test.ts` | C | tbd | todo | Instagram channel; builder server code |
| `apps/builder/__tests__/instagram-select-account-facebook-action.test.ts` | C | tbd | todo | Instagram channel; builder server code |
| `apps/builder/__tests__/instagram-select-page.test.tsx` | C | tbd | todo | Instagram channel; builder server code |
| `apps/builder/__tests__/instagram-settings-actions.test.ts` | C | tbd | todo | Instagram channel; builder server code |
| `apps/builder/__tests__/install-source-badge.test.tsx` | B | - | todo | Ads analytics and CTWA tracking; builder jsdom |
| `apps/builder/__tests__/integration-actions-create-disconnect.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/integration-api-token-hash.test.ts` | B | - | todo | API channel integration (custom channels); builder jsdom |
| `apps/builder/__tests__/integration-credential-connect-actions.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/integration-klaviyo-api.test.ts` | B | - | todo | Klaviyo integration; builder jsdom |
| `apps/builder/__tests__/integration-llm-connect-actions.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions); builder server code |
| `apps/builder/__tests__/integration-mailer-lite-api.test.ts` | B | - | todo | MailerLite integration; builder jsdom |
| `apps/builder/__tests__/integration-moosend-api.test.ts` | B | - | todo | Moosend integration; builder jsdom |
| `apps/builder/__tests__/integration-sendgrid-api.test.ts` | C | tbd | todo | SendGrid integration; builder server code |
| `apps/builder/__tests__/integration-tiktok-connect.action.test.ts` | C | tbd | todo | TikTok channel; builder server code |
| `apps/builder/__tests__/integration-tiktok-queries.test.ts` | C | tbd | todo | TikTok channel; builder server code |
| `apps/builder/__tests__/integration-webhook-real-dispatch.test.ts` | C | tbd | todo | Outbound webhooks; builder server code |
| `apps/builder/__tests__/integration-webhook-routing.test.ts` | C | tbd | todo | Outbound webhooks; builder server code |
| `apps/builder/__tests__/integration-webhook-threads.test.ts` | C | tbd | todo | Threads channel; builder server code |
| `apps/builder/__tests__/integrations-public-api.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/integrations-public-scope.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/invitation-start-route.test.ts` | C | tbd | todo | Workspace members, roles and invitations; builder server code |
| `apps/builder/__tests__/keywords-public-api.test.ts` | C | tbd | todo | Automated responses and keywords; builder server code |
| `apps/builder/__tests__/lang-selector.test.tsx` | B | - | todo | i18n (UI translations); builder jsdom |
| `apps/builder/__tests__/last-message-window.test.ts` | B | - | todo | Automated responses and keywords; builder jsdom |
| `apps/builder/__tests__/leaderboards-actions.test.ts` | C | tbd | todo | Leaderboards; builder server code |
| `apps/builder/__tests__/leaderboards-admin-ui.test.tsx` | B | - | todo | Leaderboards; builder jsdom |
| `apps/builder/__tests__/list-broadcasts-for-calendar.test.ts` | C | tbd | todo | Broadcasts (incl. recurring); builder server code |
| `apps/builder/__tests__/list-broadcasts-query.test.ts` | C | tbd | todo | Broadcasts (incl. recurring); builder server code |
| `apps/builder/__tests__/list-call-filter-options-query.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/list-call-summary-providers-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/list-contact-appointments-api.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/list-contact-fields-iso.test.ts` | C | tbd | todo | Custom fields; builder server code |
| `apps/builder/__tests__/list-conversations-ad-referral.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking; builder server code |
| `apps/builder/__tests__/list-conversations-query.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/list-my-workspaces-api.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/list-questionnaires-for-flow-api.test.ts` | C | tbd | todo | Questionnaires; builder server code |
| `apps/builder/__tests__/list-selectable-resources.test.ts` | C | tbd | todo | Template marketplace (install/share/sync); builder server code |
| `apps/builder/__tests__/list-whatsapp-calls-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/list-whatsapp-calls-query.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/load-servable-workspace.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/magic-link-route.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links); builder server code |
| `apps/builder/__tests__/magic-links-public-api.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links); builder server code |
| `apps/builder/__tests__/main-page-default-workspace.test.tsx` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/mark-read-on-outbound-field.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/marketing-messages-api.test.ts` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-audience-actions.test.ts` | C | tbd | todo | Facebook Marketing Messages; builder server code |
| `apps/builder/__tests__/marketing-messages-audience-status.test.ts` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-audience-upload.test.ts` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-button-action-editor.test.tsx` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-callback-state.test.ts` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-campaign-payload.test.ts` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-content-editor.test.tsx` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-content-schema.test.ts` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-create-action.test.ts` | C | tbd | todo | Facebook Marketing Messages; builder server code |
| `apps/builder/__tests__/marketing-messages-currency.test.ts` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-delete-action.test.ts` | C | tbd | todo | Facebook Marketing Messages; builder server code |
| `apps/builder/__tests__/marketing-messages-delete-expiry.test.ts` | C | tbd | todo | Facebook Marketing Messages; builder server code |
| `apps/builder/__tests__/marketing-messages-error-labels.test.ts` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-grant.test.ts` | C | tbd | todo | Facebook Marketing Messages; builder server code |
| `apps/builder/__tests__/marketing-messages-item-list-field.test.tsx` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-media-attachment.test.ts` | C | tbd | todo | Facebook Marketing Messages; builder server code |
| `apps/builder/__tests__/marketing-messages-public-api.test.ts` | C | tbd | todo | Facebook Marketing Messages; builder server code |
| `apps/builder/__tests__/marketing-messages-retry-warning.test.ts` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-statistics-format.test.ts` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-statistics-range.test.ts` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-statistics.test.ts` | C | tbd | todo | Facebook Marketing Messages; builder server code |
| `apps/builder/__tests__/marketing-messages-tab.test.ts` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-tos.test.ts` | B | - | todo | Facebook Marketing Messages; builder jsdom |
| `apps/builder/__tests__/marketing-messages-update-action.test.ts` | C | tbd | todo | Facebook Marketing Messages; builder server code |
| `apps/builder/__tests__/mcp-oauth-connect-snippets.test.ts` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/mcp-oauth-grants-table.test.tsx` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/mcp-oauth-revoke-action.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/me-extension-download-route.test.ts` | C | tbd | todo | Builder app shell and shared components; builder server code |
| `apps/builder/__tests__/media-library-actions.test.ts` | C | tbd | todo | Media library and file storage; builder server code |
| `apps/builder/__tests__/media-library-public-api.test.ts` | C | tbd | todo | Media library and file storage; builder server code |
| `apps/builder/__tests__/media-proxy-route.test.ts` | C | tbd | todo | Media library and file storage; builder server code |
| `apps/builder/__tests__/media-public-scope.test.ts` | C | tbd | todo | Media library and file storage; builder server code |
| `apps/builder/__tests__/message-input-client-id.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/message-input-thread-control.test.tsx` | B | - | todo | Thread control (handover protocol); builder jsdom |
| `apps/builder/__tests__/message-item-attachments.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/message-item-avatar.test.tsx` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/__tests__/message-item-call-phone-number.test.tsx` | B | - | todo | Flow steps: messages and buttons; builder jsdom |
| `apps/builder/__tests__/message-item-flow-response.test.tsx` | B | - | todo | WhatsApp Flows (forms); builder jsdom |
| `apps/builder/__tests__/message-item-inbound-commerce.test.tsx` | B | - | todo | Ecommerce: products, categories, vendors; builder jsdom |
| `apps/builder/__tests__/message-item-payment-method.test.tsx` | B | - | todo | Ecommerce: orders and payment providers; builder jsdom |
| `apps/builder/__tests__/message-item-send-error.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/message-item-timestamp.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/message-item-view-catalog.test.tsx` | B | - | todo | Flow steps: messages and buttons; builder jsdom |
| `apps/builder/__tests__/message-list.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/message-queries.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/message-thread-pane-scroll.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/messages-public-api.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/messaging-ads-connect-action-referer.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/messaging-ads-connection-state.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/messaging-ads-integration-filter.test.tsx` | B | - | todo | Ads campaigns (Meta messaging ads); builder jsdom |
| `apps/builder/__tests__/messaging-ads-oauth-referer-roundtrip.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/messaging-ads-scopes.test.ts` | B | - | todo | Ads campaigns (Meta messaging ads); builder jsdom |
| `apps/builder/__tests__/messaging-ads-tool-active-integration-ids.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/messaging-ads-tool-channel-page.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/messaging-ads-tool-channels.test.ts` | B | - | todo | Ads campaigns (Meta messaging ads); builder jsdom |
| `apps/builder/__tests__/messaging-ads-tool-empty-state.test.ts` | B | - | todo | Ads campaigns (Meta messaging ads); builder jsdom |
| `apps/builder/__tests__/messaging-ads-tool-integration-select.test.ts` | B | - | todo | Ads campaigns (Meta messaging ads); builder jsdom |
| `apps/builder/__tests__/messaging-ads-tool-integrations.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads); builder server code |
| `apps/builder/__tests__/messaging-ads-tool-path.test.ts` | B | - | todo | Ads campaigns (Meta messaging ads); builder jsdom |
| `apps/builder/__tests__/messaging-ads-tool-tabs.test.tsx` | B | - | todo | Ads campaigns (Meta messaging ads); builder jsdom |
| `apps/builder/__tests__/messenger-build-template-components.test.ts` | B | - | todo | Messenger channel; builder node-env, client logic |
| `apps/builder/__tests__/messenger-clone-template-components.test.ts` | C | tbd | todo | Messenger channel; builder server code |
| `apps/builder/__tests__/messenger-pages.test.tsx` | B | - | todo | Messenger channel; builder jsdom |
| `apps/builder/__tests__/messenger-select-page-action.test.ts` | C | tbd | todo | Messenger channel; builder server code |
| `apps/builder/__tests__/messenger-select-page.test.tsx` | C | tbd | todo | Messenger channel; builder server code |
| `apps/builder/__tests__/messenger-settings-actions.test.ts` | C | tbd | todo | Messenger channel; builder server code |
| `apps/builder/__tests__/messenger-template-mini-app-button.test.ts` | C | tbd | todo | Mini-apps; builder server code |
| `apps/builder/__tests__/messenger-template-preview.test.tsx` | B | - | todo | Messenger channel; builder jsdom |
| `apps/builder/__tests__/messenger-templates-public-api.test.ts` | C | tbd | todo | Template marketplace (install/share/sync); builder server code |
| `apps/builder/__tests__/meta-capi-callback-wiring.test.ts` | B | - | todo | Meta Conversions API (CAPI); builder jsdom |
| `apps/builder/__tests__/meta-capi-event-dialog.test.tsx` | B | - | todo | Meta Conversions API (CAPI); builder jsdom |
| `apps/builder/__tests__/meta-capi-oauth-callback.test.ts` | C | tbd | todo | Meta Conversions API (CAPI); builder server code |
| `apps/builder/__tests__/meta-catalog-operations.test.ts` | C | tbd | todo | Meta Catalog; builder server code |
| `apps/builder/__tests__/mini-app-contact-variables.test.ts` | C | tbd | todo | Mini-apps; builder server code |
| `apps/builder/__tests__/mini-app-editor-store.test.ts` | B | - | todo | Mini-apps; builder jsdom |
| `apps/builder/__tests__/mini-app-flow-response-labels.test.ts` | C | tbd | todo | Mini-apps; builder server code |
| `apps/builder/__tests__/mini-app-simple-markdown.test.tsx` | B | - | todo | Mini-apps; builder jsdom |
| `apps/builder/__tests__/mini-app-upload-route.test.ts` | C | tbd | todo | Mini-apps; builder server code |
| `apps/builder/__tests__/mini-apps-public-api.test.ts` | C | tbd | todo | Mini-apps; builder server code |
| `apps/builder/__tests__/minigame-name-conflict-action.test.ts` | C | tbd | todo | Minigames; builder server code |
| `apps/builder/__tests__/minigame-play-url.test.ts` | B | - | todo | Minigames; builder jsdom |
| `apps/builder/__tests__/minigame-player-settings-reset-policy.test.ts` | B | - | todo | Minigames; builder node-env, client logic |
| `apps/builder/__tests__/minigame-share-url.test.ts` | C | tbd | todo | Minigames; builder server code |
| `apps/builder/__tests__/minigames-public-api.test.ts` | C | tbd | todo | Minigames; builder server code |
| `apps/builder/__tests__/minigames-public-scope.test.ts` | C | tbd | todo | Minigames; builder server code |
| `apps/builder/__tests__/missed-comments-scanners.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/mobile-contacts-member-scope.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/mobile-conversations-member-scope.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/mobile-jwt-middleware.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links); builder server code |
| `apps/builder/__tests__/mobile-me-api.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links); builder server code |
| `apps/builder/__tests__/mobile-member-allowlist.test.ts` | C | tbd | todo | Workspace members, roles and invitations; builder server code |
| `apps/builder/__tests__/mobile-password-grant-rate-limit.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links); builder server code |
| `apps/builder/__tests__/mobile-user-auth-middleware.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links); builder server code |
| `apps/builder/__tests__/move-broadcast-to-draft-dialog.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/nav-main-mobile.test.tsx` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/__tests__/navigation-pending.test.tsx` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/__tests__/no-legacy-callapi-routes.test.ts` | C | tbd | todo | Builder app shell and shared components; builder server code |
| `apps/builder/__tests__/oauth-broker.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/oauth-callback-connect-session.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/oauth-consent-action.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/oauth-consent-form.test.tsx` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/oauth-error-alert-flash.test.tsx` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/oauth-protected-resource-route.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/oauth-reconnect-callback.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/oauth-referer.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/oauth-sign-in-route.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/oauth-state-decode.test.ts` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/oauth-state-nonce.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/on-user-created.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/open-link-button.test.tsx` | B | - | todo | Public links (open-link, QR codes, reflinks); builder jsdom |
| `apps/builder/__tests__/open-link-labels.test.ts` | B | - | todo | Public links (open-link, QR codes, reflinks); builder jsdom |
| `apps/builder/__tests__/open-link-page.test.tsx` | C | tbd | todo | Public links (open-link, QR codes, reflinks); builder server code |
| `apps/builder/__tests__/openai-compatible-action-schema.test.ts` | B | - | todo | OpenAI-compatible integration; builder jsdom |
| `apps/builder/__tests__/openai-compatible-actions.test.ts` | C | tbd | todo | OpenAI-compatible integration; builder server code |
| `apps/builder/__tests__/openai-compatible-model-options.test.ts` | B | - | todo | OpenAI-compatible integration; builder jsdom |
| `apps/builder/__tests__/openai-compatible-provider-verify.test.ts` | C | tbd | todo | OpenAI-compatible integration; builder server code |
| `apps/builder/__tests__/optional-field-marker.test.tsx` | C | tbd | todo | Builder app shell and shared components; builder server code |
| `apps/builder/__tests__/orders-public-api.test.ts` | C | tbd | todo | Ecommerce: orders and payment providers; builder server code |
| `apps/builder/__tests__/orpc-client-error.test.ts` | C | tbd | todo | Error logs; builder server code |
| `apps/builder/__tests__/orpc-default-error-code.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/orpc-error-mapping.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/orpc-handlers.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/orpc-unbatched-paths.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/outbound-call-mode-query-keys.test.ts` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/page-automated-responses-route-guards.test.ts` | C | tbd | todo | Automated responses and keywords; builder server code |
| `apps/builder/__tests__/payment-request-param-field.test.tsx` | B | - | todo | WhatsApp message templates; builder jsdom |
| `apps/builder/__tests__/payment-settings-card.test.tsx` | B | - | todo | Ecommerce: orders and payment providers; builder jsdom |
| `apps/builder/__tests__/pending-reads.test.ts` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/pipeline-activity-actor.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-activity-change.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-add-item-details-form.test.tsx` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-add-item-draft.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-add-item-form.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-admin-pie-model.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-analytics-range.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-append-stage-page.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-applicable-filter.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-board-filter-param.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-can-view.test.ts` | C | tbd | todo | Pipelines (CRM deals); builder server code |
| `apps/builder/__tests__/pipeline-contact-display-name.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-contact-panel-refresh.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-field-date.test.ts` | B | - | todo | Pipelines (CRM deals); builder node-env, client logic |
| `apps/builder/__tests__/pipeline-item-condition.test.tsx` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-list-filter.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-move-item-in-board.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-move-targets.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-mutation-invalidation.test.tsx` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-public-activity.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipeline-realtime-refetch.test.tsx` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipelines-actions-parity.test.ts` | C | tbd | todo | Pipelines (CRM deals); builder server code |
| `apps/builder/__tests__/pipelines-activity-filter.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/pipelines-media-scope.test.ts` | C | tbd | todo | Pipelines (CRM deals); builder server code |
| `apps/builder/__tests__/pipelines-public-api.test.ts` | C | tbd | todo | Pipelines (CRM deals); builder server code |
| `apps/builder/__tests__/pipelines-public-scope.test.ts` | C | tbd | todo | Pipelines (CRM deals); builder server code |
| `apps/builder/__tests__/pipelines-realtime-event.test.ts` | B | - | todo | Pipelines (CRM deals); builder node-env, client logic |
| `apps/builder/__tests__/places-near-me-categories-ui.test.tsx` | B | - | todo | Places near me; builder jsdom |
| `apps/builder/__tests__/places-near-me-category-options.test.ts` | C | tbd | todo | Places near me; builder server code |
| `apps/builder/__tests__/places-near-me-locations-ui.test.tsx` | B | - | todo | Places near me; builder jsdom |
| `apps/builder/__tests__/places-near-me-public-api.test.ts` | C | tbd | todo | Places near me; builder server code |
| `apps/builder/__tests__/places-near-me-public-schema.test.ts` | B | - | todo | Places near me; builder jsdom |
| `apps/builder/__tests__/plain-text-tiptap-editor-dom.test.tsx` | B | - | todo | Variables (contact/bot/system variable interpolation); builder jsdom |
| `apps/builder/__tests__/plain-text-tiptap-editor.test.ts` | B | - | todo | Variables (contact/bot/system variable interpolation); builder jsdom |
| `apps/builder/__tests__/platform-admins-queries.test.ts` | C | tbd | todo | Platform admin; builder server code |
| `apps/builder/__tests__/platform-audit-logs-queries.test.ts` | C | tbd | todo | Platform admin; builder server code |
| `apps/builder/__tests__/platform-credential-owner.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/platform-credentials-scope.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/platform-default-template-actions.test.ts` | C | tbd | todo | Template marketplace (install/share/sync); builder server code |
| `apps/builder/__tests__/platform-default-template-settings.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/poll-form-schema.test.ts` | B | - | todo | Polls; builder jsdom |
| `apps/builder/__tests__/poll-queries.test.ts` | C | tbd | todo | Polls; builder server code |
| `apps/builder/__tests__/polls-public-api.test.ts` | C | tbd | todo | Polls; builder server code |
| `apps/builder/__tests__/post-preview-image.test.ts` | B | - | todo | Comment automation (FB/IG/Threads/TikTok); builder jsdom |
| `apps/builder/__tests__/presigned-upload-route.test.ts` | C | tbd | todo | Media library and file storage; builder server code |
| `apps/builder/__tests__/preview-channels-gating.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/process-missed-comments.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/product-categories-public-api.test.ts` | C | tbd | todo | Ecommerce: products, categories, vendors; builder server code |
| `apps/builder/__tests__/product-categories-public-scope.test.ts` | C | tbd | todo | Ecommerce: products, categories, vendors; builder server code |
| `apps/builder/__tests__/product-message-orpc-contract.test.ts` | C | tbd | todo | Ecommerce: products, categories, vendors; builder server code |
| `apps/builder/__tests__/product-vendors-public-api.test.ts` | C | tbd | todo | Ecommerce: products, categories, vendors; builder server code |
| `apps/builder/__tests__/products-public-api.test.ts` | C | tbd | todo | Ecommerce: products, categories, vendors; builder server code |
| `apps/builder/__tests__/products-public-scope.test.ts` | C | tbd | todo | Ecommerce: products, categories, vendors; builder server code |
| `apps/builder/__tests__/prompt-variable-options-bot-fields.test.ts` | B | - | todo | AI agents (tools, MCP client, functions); builder jsdom |
| `apps/builder/__tests__/prompt-variable-options-pipelines.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/__tests__/provider-account-route-scope.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/provider-origin.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/proxy-public-routes.test.ts` | B | - | todo | Workspace API tokens and public API; builder jsdom |
| `apps/builder/__tests__/public-api-followup-schemas.test.ts` | B | - | todo | Workspace API tokens and public API; builder node-env, client logic |
| `apps/builder/__tests__/public-api-list-pagination.test.ts` | B | - | todo | Workspace API tokens and public API; builder jsdom |
| `apps/builder/__tests__/public-field-operation-schema.test.ts` | B | - | todo | Custom fields; builder jsdom |
| `apps/builder/__tests__/public-list-queries-no-session.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links); builder server code |
| `apps/builder/__tests__/public-router-boundary.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/public-scheduled-deletion-routes.test.tsx` | C | tbd | todo | Scheduler and scheduled jobs; builder server code |
| `apps/builder/__tests__/public-spec-json-route.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/public-spec-mcp.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/public-spec-operations.test.ts` | C | tbd | baseline-fail | Workspace API tokens and public API; builder server code; P0.3: beforeAll hook timeout 120 s (imports whole public router; Dell under memory pressure) — recheck in P0.7 |
| `apps/builder/__tests__/public-spec-options.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/public-template-card.test.tsx` | C | tbd | todo | Template marketplace (install/share/sync); builder server code |
| `apps/builder/__tests__/publish-flow-action.test.ts` | C | tbd | todo | Flow builder and execution; builder server code |
| `apps/builder/__tests__/publish-flow-channel-validation.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/__tests__/qr-codes-public-api.test.ts` | C | tbd | todo | Public links (open-link, QR codes, reflinks); builder server code |
| `apps/builder/__tests__/questionnaire-applicant-name-cell.test.tsx` | B | - | todo | Questionnaires; builder jsdom |
| `apps/builder/__tests__/questionnaire-retry-message-defaults.test.ts` | B | - | todo | Questionnaires; builder node-env, client logic |
| `apps/builder/__tests__/questionnaires-form-schema.test.ts` | B | - | todo | Questionnaires; builder jsdom |
| `apps/builder/__tests__/questionnaires-public-api.test.ts` | C | tbd | todo | Questionnaires; builder server code |
| `apps/builder/__tests__/quick-replies-popover-images.test.tsx` | B | - | todo | Flow steps: messages and buttons; builder jsdom |
| `apps/builder/__tests__/quick-replies-popover.test.tsx` | B | - | todo | Flow steps: messages and buttons; builder jsdom |
| `apps/builder/__tests__/quota-metrics-trial.test.ts` | C | tbd | todo | Licensing, billing and quotas; builder server code |
| `apps/builder/__tests__/read-conversation.action.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/realtime-connect-token-api.test.ts` | C | tbd | todo | Realtime (WebSocket gateway); builder server code |
| `apps/builder/__tests__/realtime-warn-limiter.test.ts` | B | - | todo | Realtime (WebSocket gateway); builder jsdom |
| `apps/builder/__tests__/recurrence-defaults.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/recurrence-rule-fields.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/recurrence-summary.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/recurrence-weekday-options.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/reflink-widget-route.test.ts` | C | tbd | todo | Leaderboards; builder server code |
| `apps/builder/__tests__/reflinks-public-api.test.ts` | C | tbd | todo | Leaderboards; builder server code |
| `apps/builder/__tests__/refresh-all-channel-tokens-action.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/refresh-contact-profile-action.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/remove-contact-sequence.action.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/request-resolver.test.tsx` | C | tbd | todo | Flow builder and execution; builder server code |
| `apps/builder/__tests__/require-contact-scan-page-access.test.ts` | C | tbd | todo | Contact scan; builder server code |
| `apps/builder/__tests__/require-unrestricted-contacts-scope.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/require-workspace-permission.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/resend-broadcast-dialog.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/resend-broadcast.action.test.ts` | C | tbd | todo | Broadcasts (incl. recurring); builder server code |
| `apps/builder/__tests__/reserved-custom-fields.test.ts` | B | - | todo | Custom fields; builder jsdom |
| `apps/builder/__tests__/reset-bot-fields.action.test.ts` | C | tbd | todo | Bot fields; builder server code |
| `apps/builder/__tests__/resolve-already-connected-providers.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/resolve-connect-session.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/resolve-contact-avatars.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/resolve-integration-capi-connection.test.ts` | B | - | todo | Meta Conversions API (CAPI); builder jsdom |
| `apps/builder/__tests__/resolve-last-message-preview.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/resolve-provider.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/resolve-workspace-realtime-gates.test.ts` | B | - | todo | Realtime (WebSocket gateway); builder jsdom |
| `apps/builder/__tests__/response-sla-public-api.test.ts` | C | tbd | todo | Response SLA; builder server code |
| `apps/builder/__tests__/resume-broadcast-dialog.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/routing-roles-section.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/row-status.test.ts` | B | - | todo | Builder app shell and shared components; builder node-env, client logic |
| `apps/builder/__tests__/rpc-auth-boundary.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links); builder server code |
| `apps/builder/__tests__/rpc-router-auth-surface.test.ts` | B | - | todo | Authentication (sign-in, social, mobile, magic links); builder jsdom |
| `apps/builder/__tests__/safe-action-error-logging.test.ts` | C | tbd | todo | Builder app shell and shared components; builder server code |
| `apps/builder/__tests__/safe-callback-url.test.ts` | B | - | todo | Builder app shell and shared components; builder node-env, client logic |
| `apps/builder/__tests__/same-site-request.test.ts` | C | tbd | todo | Builder app shell and shared components; builder server code |
| `apps/builder/__tests__/save-ai-handover-settings-action.test.ts` | C | tbd | todo | AI handover (human takeover); builder server code |
| `apps/builder/__tests__/saved-replies-public-api.test.ts` | C | tbd | todo | Saved replies; builder server code |
| `apps/builder/__tests__/schedule-broadcast-dialog.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/schedule-contact-scan-action.test.ts` | C | tbd | todo | Contact scan; builder server code |
| `apps/builder/__tests__/schedule-workspace-deletion-action.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/select-ad-badge.test.ts` | B | - | todo | Ads analytics and CTWA tracking; builder jsdom |
| `apps/builder/__tests__/select-all.test.ts` | B | - | todo | Builder app shell and shared components; builder node-env, client logic |
| `apps/builder/__tests__/select-facebook-accounts.test.tsx` | B | - | todo | Facebook Ads integration; builder jsdom |
| `apps/builder/__tests__/sequence-delay.test.ts` | B | - | todo | Sequences; builder jsdom |
| `apps/builder/__tests__/sequence-detail-resource.test.ts` | B | - | todo | Sequences; builder node-env, client logic |
| `apps/builder/__tests__/sequence-folders-public-api.test.ts` | C | tbd | todo | Sequences; builder server code |
| `apps/builder/__tests__/sequences-list-step-contacts-api.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/sequences-public-api.test.ts` | C | tbd | todo | Sequences; builder server code |
| `apps/builder/__tests__/sequences-public-scope.test.ts` | C | tbd | todo | Sequences; builder server code |
| `apps/builder/__tests__/set-custom-field-boolean-value.test.tsx` | B | - | todo | Custom fields; builder jsdom |
| `apps/builder/__tests__/settings-channels-add-button-gating.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/settings-channels-visibility.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/__tests__/settings-tab-active-state.test.ts` | C | tbd | todo | Builder app shell and shared components; builder server code |
| `apps/builder/__tests__/shop-product-discount.test.ts` | B | - | todo | Coupons and discount codes; builder jsdom |
| `apps/builder/__tests__/shopify-public-api.test.ts` | C | tbd | todo | Shopify integration; builder server code |
| `apps/builder/__tests__/short-time-ago.test.ts` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/should-run-webchat-challenge.test.ts` | B | - | todo | Webchat channel; builder jsdom |
| `apps/builder/__tests__/should-send-on-enter.test.ts` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/smart-delay-node-stats.test.ts` | B | - | todo | Smart delay; builder jsdom |
| `apps/builder/__tests__/smtp-actions-thin.test.ts` | C | tbd | todo | SMTP email integration; builder server code |
| `apps/builder/__tests__/smtp-integrations-public-api.test.ts` | C | tbd | todo | SMTP email integration; builder server code |
| `apps/builder/__tests__/social-auth-instances.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links); builder server code |
| `apps/builder/__tests__/social-login-community.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links); builder server code |
| `apps/builder/__tests__/spreadsheets-public-api.test.ts` | C | tbd | todo | Google Sheets integration; builder server code |
| `apps/builder/__tests__/spreadsheets-url-validation.test.ts` | B | - | todo | Google Sheets integration; builder jsdom |
| `apps/builder/__tests__/start-meta-capi-oauth.test.ts` | C | tbd | todo | Meta Conversions API (CAPI); builder server code |
| `apps/builder/__tests__/start-product-import.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows); builder server code |
| `apps/builder/__tests__/status-text-contrast.test.ts` | B | - | todo | Builder app shell and shared components; builder node-env, client logic |
| `apps/builder/__tests__/stop-broadcast-dialog.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/stripe-settings-actions.test.ts` | C | tbd | todo | Ecommerce: orders and payment providers; builder server code |
| `apps/builder/__tests__/submit-availability-range-action.test.ts` | C | tbd | todo | Appointments and booking; builder server code |
| `apps/builder/__tests__/submit-booking-action.test.ts` | C | tbd | todo | Appointments and booking; builder server code |
| `apps/builder/__tests__/submit-date-time-action.test.ts` | C | tbd | todo | Appointments and booking; builder server code |
| `apps/builder/__tests__/submit-mini-app-action.test.ts` | C | tbd | todo | Mini-apps; builder server code |
| `apps/builder/__tests__/support-access-actions.test.ts` | C | tbd | todo | Workspace support access; builder server code |
| `apps/builder/__tests__/support-access-safe-action.test.ts` | C | tbd | todo | Workspace support access; builder server code |
| `apps/builder/__tests__/support-chat-script.test.tsx` | B | - | todo | Workspace support access; builder node-env, client logic |
| `apps/builder/__tests__/sync-thread-owner-action.test.ts` | C | tbd | todo | Thread control (handover protocol); builder server code |
| `apps/builder/__tests__/tags-custom-fields-public-api.test.ts` | C | tbd | todo | Custom fields; builder server code |
| `apps/builder/__tests__/tailwind-sources.test.ts` | B | - | todo | Builder app shell and shared components; builder node-env, client logic |
| `apps/builder/__tests__/template-cancel-uninstall-toasts.test.ts` | B | - | todo | Template marketplace (install/share/sync); builder node-env, client logic |
| `apps/builder/__tests__/template-editor-feedback.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-install-intent-guard.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-install-progress-refresher.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-install-section.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-installation-auto-update-action.test.ts` | C | tbd | todo | Template marketplace (install/share/sync); builder server code |
| `apps/builder/__tests__/template-installation-warnings.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-installer-action-client.test.ts` | C | tbd | todo | Template marketplace (install/share/sync); builder server code |
| `apps/builder/__tests__/template-installer-issues-dialog.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-installers-dialog.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-invalidate-after-change.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-lock-badge-undeletable.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-not-found-guard.test.ts` | C | tbd | todo | Template marketplace (install/share/sync); builder server code |
| `apps/builder/__tests__/template-params-form-payment-request.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-preview-payment-request.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-resource-href.test.ts` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-safe-http-url.test.ts` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-selection-lock.test.ts` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-share-credentials-and-source.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-share-expiry.test.ts` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-share-refresh.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-sync-history-counts.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-tabs-permissions.test.ts` | C | tbd | todo | Template marketplace (install/share/sync); builder server code |
| `apps/builder/__tests__/template-uninstall-state-label.test.ts` | C | tbd | todo | Template marketplace (install/share/sync); builder server code |
| `apps/builder/__tests__/template-use-category-selection.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-warning-label.test.ts` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/template-workspace-picker.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/templates-public-hardening.test.ts` | C | tbd | todo | Template marketplace (install/share/sync); builder server code |
| `apps/builder/__tests__/templates-public-install-idempotency.test.ts` | C | tbd | todo | Template marketplace (install/share/sync); builder server code |
| `apps/builder/__tests__/templates-public-read-permissions.test.ts` | C | tbd | todo | Template marketplace (install/share/sync); builder server code |
| `apps/builder/__tests__/templates-public-scope.test.ts` | C | tbd | todo | Template marketplace (install/share/sync); builder server code |
| `apps/builder/__tests__/templates-table-installs.test.tsx` | B | - | todo | Template marketplace (install/share/sync); builder jsdom |
| `apps/builder/__tests__/test-external-request-action.test.ts` | C | tbd | todo | Outbound webhooks; builder server code |
| `apps/builder/__tests__/test-query-helpers.test.ts` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/__tests__/theme-fonts.test.ts` | B | - | todo | Builder app shell and shared components; builder node-env, client logic |
| `apps/builder/__tests__/thread-control-action.test.ts` | C | tbd | todo | Thread control (handover protocol); builder server code |
| `apps/builder/__tests__/thread-control-content.test.ts` | B | - | todo | Thread control (handover protocol); builder jsdom |
| `apps/builder/__tests__/thread-control-locked-composer.test.tsx` | B | - | todo | Thread control (handover protocol); builder jsdom |
| `apps/builder/__tests__/thread-control-pass-entry-points.test.tsx` | B | - | todo | Thread control (handover protocol); builder jsdom |
| `apps/builder/__tests__/thread-control-pass-partner-dialog.test.tsx` | B | - | todo | Thread control (handover protocol); builder jsdom |
| `apps/builder/__tests__/thread-control-release-button.test.tsx` | B | - | todo | Thread control (handover protocol); builder jsdom |
| `apps/builder/__tests__/thread-control-request-schema.test.ts` | C | tbd | todo | Thread control (handover protocol); builder server code |
| `apps/builder/__tests__/thread-control-view-multi-channel.test.ts` | B | - | todo | Thread control (handover protocol); builder jsdom |
| `apps/builder/__tests__/thread-control-view.test.ts` | B | - | todo | Thread control (handover protocol); builder jsdom |
| `apps/builder/__tests__/threads-comment-form-capability.test.tsx` | B | - | todo | Comment automation (FB/IG/Threads/TikTok); builder jsdom |
| `apps/builder/__tests__/threads-comment-form-schema.test.ts` | B | - | todo | Comment automation (FB/IG/Threads/TikTok); builder jsdom |
| `apps/builder/__tests__/threads-comments-threads-posts.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/threads-settings-actions.test.ts` | C | tbd | todo | Threads channel; builder server code |
| `apps/builder/__tests__/threads-tiktok-comment-read-adapters.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/threads-tiktok-comments-public-api.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok); builder server code |
| `apps/builder/__tests__/threads-webhook-url.test.ts` | B | - | todo | Threads channel; builder jsdom |
| `apps/builder/__tests__/tiktok-settings-actions.test.ts` | C | tbd | todo | TikTok channel; builder server code |
| `apps/builder/__tests__/timezone-sync.test.tsx` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/__tests__/token-introspection-public-api.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/token-scope.test.ts` | B | - | todo | Workspace API tokens and public API; builder node-env, client logic |
| `apps/builder/__tests__/tools-list-config.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/__tests__/tools-list-visibility.test.tsx` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/__tests__/trigger-actions-public-api-schema.test.ts` | C | tbd | todo | Triggers; builder server code |
| `apps/builder/__tests__/trigger-pipeline-action-schema.test.ts` | C | tbd | todo | Pipelines (CRM deals); builder server code |
| `apps/builder/__tests__/trigger-send-google-ads-conversion-schema.test.ts` | C | tbd | todo | Google Ads integration; builder server code |
| `apps/builder/__tests__/trigger-send-meta-capi-event-schema.test.ts` | C | tbd | todo | Meta Conversions API (CAPI); builder server code |
| `apps/builder/__tests__/trigger-set-custom-field-schema.test.ts` | C | tbd | todo | Custom fields; builder server code |
| `apps/builder/__tests__/trigger-start-external-step-schema.test.ts` | C | tbd | todo | Triggers; builder server code |
| `apps/builder/__tests__/triggers-public-api.test.ts` | C | tbd | todo | Triggers; builder server code |
| `apps/builder/__tests__/update-contact-field.action.test.ts` | C | tbd | todo | Custom fields; builder server code |
| `apps/builder/__tests__/update-contact-sequence.action.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/__tests__/update-conversation-assignee.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/update-handover-resume-flow-action.test.ts` | C | tbd | todo | Thread control (handover protocol); builder server code |
| `apps/builder/__tests__/update-instagram-action.test.ts` | C | tbd | todo | Instagram channel; builder server code |
| `apps/builder/__tests__/update-mark-read-on-outbound.action.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/__tests__/update-messenger-action.test.ts` | C | tbd | todo | Messenger channel; builder server code |
| `apps/builder/__tests__/update-messenger-handover-resume-flow-action.test.ts` | C | tbd | todo | Thread control (handover protocol); builder server code |
| `apps/builder/__tests__/update-pix-redirect-action.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/update-smart-response-delay-schema.test.ts` | B | - | todo | Smart delay; builder jsdom |
| `apps/builder/__tests__/update-trigger-action.test.ts` | C | tbd | todo | Triggers; builder server code |
| `apps/builder/__tests__/update-trigger-settings-action.test.ts` | C | tbd | todo | Triggers; builder server code |
| `apps/builder/__tests__/update-webchat-action-permission.test.ts` | C | tbd | todo | Webchat channel; builder server code |
| `apps/builder/__tests__/update-webhook-action.test.ts` | C | tbd | todo | Outbound webhooks; builder server code |
| `apps/builder/__tests__/update-webhook-settings-action.test.ts` | C | tbd | todo | Outbound webhooks; builder server code |
| `apps/builder/__tests__/update-workspace-advanced-schema.test.ts` | B | - | todo | Workspaces; builder jsdom |
| `apps/builder/__tests__/upload-handlers.test.ts` | C | tbd | todo | Media library and file storage; builder server code |
| `apps/builder/__tests__/upload-logo-action.test.ts` | C | tbd | todo | Media library and file storage; builder server code |
| `apps/builder/__tests__/upsert-sequence-step-schema.test.ts` | B | - | todo | Sequences; builder jsdom |
| `apps/builder/__tests__/upsert-sequence-step.action.test.ts` | C | tbd | todo | Sequences; builder server code |
| `apps/builder/__tests__/usage-number-formatting.test.tsx` | C | tbd | todo | Analytics dashboards; builder server code |
| `apps/builder/__tests__/use-ads-range-url.test.tsx` | B | - | todo | Ads analytics and CTWA tracking; builder jsdom |
| `apps/builder/__tests__/use-ai-agents.test.tsx` | B | - | todo | AI flow steps (text, image, speech, extraction); builder jsdom |
| `apps/builder/__tests__/use-auto-refresh-contact-profile.test.tsx` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/__tests__/use-broadcast-plan-limit.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/use-capi-connect-result.test.tsx` | B | - | todo | Meta Conversions API (CAPI); builder jsdom |
| `apps/builder/__tests__/use-channel-connect-error.test.tsx` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/use-connect-batch.coexist.test.tsx` | B | - | todo | Coexist (existing-app history sync); builder jsdom |
| `apps/builder/__tests__/use-connect-batch.parallelism.test.tsx` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/use-connect-batch.test.tsx` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/use-connect-flow.test.tsx` | B | - | todo | Channel connection and OAuth connect flow; builder jsdom |
| `apps/builder/__tests__/use-contacts.test.tsx` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/__tests__/use-conversation-id-param.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/use-currency-options.test.tsx` | B | - | todo | Ecommerce: settings, open hours, analytics; builder jsdom |
| `apps/builder/__tests__/use-delay-state.test.tsx` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/__tests__/use-field-operation-reset.test.tsx` | B | - | todo | Custom fields; builder jsdom |
| `apps/builder/__tests__/use-filter-value-labels.test.tsx` | B | - | todo | Contact filters and segments; builder jsdom |
| `apps/builder/__tests__/use-interval-router-refresh.test.tsx` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/__tests__/use-invalidate-broadcast-runs.test.tsx` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/__tests__/use-mark-conversation-read.test.tsx` | B | - | todo | Flow steps: contact and conversation actions; builder jsdom |
| `apps/builder/__tests__/use-now.test.tsx` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/__tests__/use-read-only-when-locked.test.tsx` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/__tests__/use-realtime-socket.test.tsx` | B | - | todo | Realtime (WebSocket gateway); builder jsdom |
| `apps/builder/__tests__/use-sequence-step.test.tsx` | B | - | todo | Sequences; builder jsdom |
| `apps/builder/__tests__/use-thread-read-tracking.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/__tests__/use-whatsapp-call-starter.test.ts` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/use-whatsapp-voip-call.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/use-workspace-option-sources.test.tsx` | B | - | todo | Workspaces; builder jsdom |
| `apps/builder/__tests__/user-persistent-menus-public-api.test.ts` | C | tbd | todo | Messenger channel; builder server code |
| `apps/builder/__tests__/user-timezone.test.ts` | C | tbd | todo | Builder app shell and shared components; builder server code |
| `apps/builder/__tests__/v1-etag-interceptor.test.ts` | C | tbd | todo | Tags and labels; builder server code |
| `apps/builder/__tests__/validate-ai-mcp-server.action.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions); builder server code |
| `apps/builder/__tests__/variable-injection-filter-mentions.test.ts` | B | - | todo | Variables (contact/bot/system variable interpolation); builder jsdom |
| `apps/builder/__tests__/variable-injection-mention.test.ts` | B | - | todo | Variables (contact/bot/system variable interpolation); builder jsdom |
| `apps/builder/__tests__/variable-injection-suggestion.test.ts` | B | - | todo | Variables (contact/bot/system variable interpolation); builder jsdom |
| `apps/builder/__tests__/voip-attach-microphone.test.ts` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/webchat-brand-color.test.ts` | B | - | todo | Webchat channel; builder jsdom |
| `apps/builder/__tests__/webchat-browser-profile-fields.test.ts` | B | - | todo | Webchat channel; builder jsdom |
| `apps/builder/__tests__/webchat-guest-session.test.ts` | C | tbd | todo | Webchat channel; builder server code |
| `apps/builder/__tests__/webchat-message-schema.test.ts` | B | - | todo | Webchat channel; builder jsdom |
| `apps/builder/__tests__/webchat-mutation-schema.test.ts` | B | - | todo | Webchat channel; builder jsdom |
| `apps/builder/__tests__/webchat-page-guards.test.tsx` | C | tbd | todo | Webchat channel; builder server code |
| `apps/builder/__tests__/webchat-realtime.test.tsx` | B | - | todo | Realtime (WebSocket gateway); builder jsdom |
| `apps/builder/__tests__/webchat-widget-css.test.ts` | B | - | todo | Webchat channel; builder jsdom |
| `apps/builder/__tests__/webchats-create-visibility-guard.test.ts` | C | tbd | todo | Webchat channel; builder server code |
| `apps/builder/__tests__/webchats-public-api.test.ts` | C | tbd | todo | Webchat channel; builder server code |
| `apps/builder/__tests__/webhook-log.test.ts` | B | - | todo | Outbound webhooks; builder node-env, client logic |
| `apps/builder/__tests__/webhooks-public-api.test.ts` | C | tbd | todo | Outbound webhooks; builder server code |
| `apps/builder/__tests__/webshop-public-api.test.ts` | C | tbd | todo | Ecommerce: webshop storefront; builder server code |
| `apps/builder/__tests__/webshop-shop-locale.test.ts` | C | tbd | todo | Ecommerce: webshop storefront; builder server code |
| `apps/builder/__tests__/webshop-shop-session.test.ts` | C | tbd | todo | Ecommerce: webshop storefront; builder server code |
| `apps/builder/__tests__/whatsapp-account-health-page.test.tsx` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-answer-voip-call-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-blocked-outbound-countries.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-call-ai-summary-dialog.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-call-announcement-options.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-call-announcement-validation-error.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-call-card-real-store.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-call-card.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-call-hours-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-call-hours-section.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-call-hours.test.ts` | B | - | todo | WhatsApp calling (VoIP); builder node-env, client logic |
| `apps/builder/__tests__/whatsapp-call-info-sheet.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-call-panel.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-call-realtime.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-call-recorder.test.ts` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-call-recording-upload-route.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-calling-preflight-messaging-limit.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-calling-preflight.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-calling-public-api.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-calling-settings-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-calls-card.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-calls-public-api.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-capi-connect-button.test.tsx` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/__tests__/whatsapp-capi-scope.test.ts` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/__tests__/whatsapp-coexist-mode.test.ts` | B | - | todo | Coexist (existing-app history sync); builder node-env, client logic |
| `apps/builder/__tests__/whatsapp-connect-action.direct.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-connect-action.follow-ups.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-connect-action.session.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-connect-card.picker.test.tsx` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-connect-card.post-connect.test.tsx` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-connect-card.test.tsx` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-connect-result-intent.test.ts` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/__tests__/whatsapp-connect-schema.test.ts` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/__tests__/whatsapp-connect-stages.test.tsx` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/__tests__/whatsapp-connect-webhook-url.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-effective-calling-settings.test.ts` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-embedded-signup-auto-connect.test.tsx` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/__tests__/whatsapp-embedded-signup.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-end-voip-call-as-agent.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-flow-file-mappings.test.tsx` | B | - | todo | WhatsApp Flows (forms); builder jsdom |
| `apps/builder/__tests__/whatsapp-flow-response-dialog.test.tsx` | B | - | todo | WhatsApp Flows (forms); builder jsdom |
| `apps/builder/__tests__/whatsapp-flow-response.test.ts` | C | tbd | todo | WhatsApp Flows (forms); builder server code |
| `apps/builder/__tests__/whatsapp-flows-public-api.test.ts` | C | tbd | todo | WhatsApp Flows (forms); builder server code |
| `apps/builder/__tests__/whatsapp-hangup-voip-call-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-heartbeat-active-voip-call-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-initiate-outbound-voip-call-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-manual-webhook-route.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-meta-call-permission.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-oauth-callback-outcome.test.ts` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-oauth-popup-cancel.test.tsx` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/__tests__/whatsapp-oauth-relay.test.ts` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/__tests__/whatsapp-onboarding-result-done.test.tsx` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/__tests__/whatsapp-outbound-voip-turn-credentials-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-payment-settings-public-api.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-permission-status.test.ts` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/__tests__/whatsapp-phone-verification-queue.test.tsx` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/__tests__/whatsapp-public-api-lib.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-reconnect-action.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-reconnect-button.test.tsx` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/__tests__/whatsapp-request-call-permission-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-resolve-outbound-call-mode-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-ringing-calls-list.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-settings-actions.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-template-preview-substitute.test.ts` | B | - | todo | WhatsApp message templates; builder jsdom |
| `apps/builder/__tests__/whatsapp-template-status-forwarding.test.ts` | C | tbd | todo | WhatsApp message templates; builder server code |
| `apps/builder/__tests__/whatsapp-templates-more-public-api.test.ts` | C | tbd | todo | WhatsApp message templates; builder server code |
| `apps/builder/__tests__/whatsapp-verification-actions.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/__tests__/whatsapp-voip-call-button.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-voip-call-context.test.tsx` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-voip-call-hangup-route.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/whatsapp-voip-call-store.test.ts` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-voip-ringtone.test.ts` | B | - | todo | WhatsApp calling (VoIP); builder jsdom |
| `apps/builder/__tests__/whatsapp-voip-turn-credentials-action.test.ts` | C | tbd | todo | WhatsApp calling (VoIP); builder server code |
| `apps/builder/__tests__/woocommerce-public-api.test.ts` | C | tbd | todo | WooCommerce integration; builder server code |
| `apps/builder/__tests__/worker-only-business-subpaths.test.ts` | C | tbd | todo | Builder app shell and shared components; builder server code |
| `apps/builder/__tests__/working-hours-actions.test.ts` | C | tbd | todo | Working hours; builder server code |
| `apps/builder/__tests__/working-hours-date-override-state.test.ts` | B | - | todo | Working hours; builder jsdom |
| `apps/builder/__tests__/working-hours-date-override-ui.test.tsx` | B | - | todo | Working hours; builder jsdom |
| `apps/builder/__tests__/working-hours-public-api.test.ts` | C | tbd | todo | Working hours; builder server code |
| `apps/builder/__tests__/working-hours-public-scope.test.ts` | C | tbd | todo | Working hours; builder server code |
| `apps/builder/__tests__/working-hours-schedule-state.test.ts` | C | tbd | todo | Working hours; builder server code |
| `apps/builder/__tests__/working-hours-sla-pill.test.tsx` | B | - | todo | Working hours; builder jsdom |
| `apps/builder/__tests__/working-hours-time-options.test.ts` | B | - | todo | Working hours; builder node-env, client logic |
| `apps/builder/__tests__/workspace-authorized-middleware.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/workspace-members-actions.test.ts` | C | tbd | todo | Workspace members, roles and invitations; builder server code |
| `apps/builder/__tests__/workspace-owner-quota.test.ts` | C | tbd | todo | Licensing, billing and quotas; builder server code |
| `apps/builder/__tests__/workspace-permission-routes.test.ts` | B | - | todo | Workspaces; builder node-env, client logic |
| `apps/builder/__tests__/workspace-presence-report-route.test.ts` | C | tbd | todo | Realtime (WebSocket gateway); builder server code |
| `apps/builder/__tests__/workspace-realtime-provider.test.tsx` | B | - | todo | Realtime (WebSocket gateway); builder jsdom |
| `apps/builder/__tests__/workspace-realtime-shell.test.tsx` | B | - | todo | Realtime (WebSocket gateway); builder jsdom |
| `apps/builder/__tests__/workspace-root-page-scheduled-deletion.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/workspace-route-skeleton.test.ts` | B | - | todo | Workspaces; builder jsdom |
| `apps/builder/__tests__/workspace-scheduled-deletion-enforcer.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/workspace-settings-actions.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/workspace-settings-public-api.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/workspace-status-guard.test.tsx` | B | - | todo | Workspaces; builder jsdom |
| `apps/builder/__tests__/workspace-token-auth-middleware.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links); builder server code |
| `apps/builder/__tests__/workspace-token-scope-enforcement.test.ts` | C | tbd | todo | Workspace API tokens and public API; builder server code |
| `apps/builder/__tests__/workspace-token-scope-registry.test.ts` | B | - | todo | Workspace API tokens and public API; builder jsdom |
| `apps/builder/__tests__/workspace-update-actions-scheduled-deletion-guard.test.ts` | C | tbd | todo | Workspaces; builder server code |
| `apps/builder/__tests__/workspaces-list.community-zero.test.tsx` | B | - | todo | Workspaces; builder jsdom |
| `apps/builder/__tests__/zalo-settings-actions.test.ts` | C | tbd | todo | Zalo channel; builder server code |
| `apps/builder/src/components/__tests__/nav-user.test.tsx` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/src/components/tiptap/extensions/variable-injection/__tests__/suggestion.test.ts` | B | - | todo | Variables (contact/bot/system variable interpolation); builder jsdom |
| `apps/builder/src/features/ads/lib/__tests__/build-whatsapp-retarget-href.test.ts` | B | - | todo | Ads analytics and CTWA tracking; builder jsdom |
| `apps/builder/src/features/appointment-management/components/inbox/__tests__/create-contact-appointment-dialog.test.tsx` | B | - | todo | Appointments and booking; builder jsdom |
| `apps/builder/src/features/appointment-management/lib/__tests__/use-invalidate-appointments.test.ts` | C | tbd | todo | Appointments and booking; builder server code |
| `apps/builder/src/features/booking-webview/components/__tests__/date-time-picker.test.tsx` | B | - | todo | Appointments and booking; builder jsdom |
| `apps/builder/src/features/booking-webview/components/__tests__/slot-picker.test.tsx` | B | - | todo | Appointments and booking; builder jsdom |
| `apps/builder/src/features/broadcasts/lib/__tests__/broadcast-detail-pages.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/src/features/broadcasts/lib/__tests__/broadcast-filter-fields.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/src/features/broadcasts/lib/__tests__/broadcast-flow-targets.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/src/features/broadcasts/lib/__tests__/broadcast-inbox-label.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/src/features/broadcasts/lib/__tests__/broadcast-targets.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/src/features/broadcasts/lib/__tests__/create-broadcast-defaults.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/src/features/broadcasts/lib/__tests__/template-hydration.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/src/features/broadcasts/provider/__tests__/broadcast-stats-store.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/src/features/broadcasts/schema/__tests__/create-broadcast-prefill.test.ts` | B | - | todo | Broadcasts (incl. recurring); builder jsdom |
| `apps/builder/src/features/channel-connect/lib/__tests__/branding-follow-ups.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow; builder server code |
| `apps/builder/src/features/common/hooks/__tests__/use-stats-selection.test.ts` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/src/features/conditions/__tests__/editor.test.tsx` | B | - | todo | Triggers; builder jsdom |
| `apps/builder/src/features/conditions/__tests__/schemas.test.ts` | B | - | todo | Triggers; builder jsdom |
| `apps/builder/src/features/contact-filter/lib/__tests__/list-filter-fields.test.ts` | C | tbd | todo | Contact filters and segments; builder server code |
| `apps/builder/src/features/contact-filter/lib/__tests__/prune-conditions.test.ts` | B | - | todo | Contact filters and segments; builder jsdom |
| `apps/builder/src/features/contact-filter/schema/__tests__/ctwa-retarget-filter.test.ts` | B | - | todo | Contact filters and segments; builder jsdom |
| `apps/builder/src/features/contacts/actions/__tests__/contact-tag-actions.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/src/features/contacts/lib/__tests__/channel-identity.test.ts` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/src/features/contacts/lib/__tests__/refresh-contact-profile.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity); builder server code |
| `apps/builder/src/features/contacts/provider/__tests__/contact-store.test.ts` | B | - | todo | Contacts (profiles, notes, activity); builder jsdom |
| `apps/builder/src/features/conversations/lib/__tests__/is-conversation-unread.test.ts` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/src/features/conversations/queries/__tests__/list-conversations.query.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages); builder server code |
| `apps/builder/src/features/coupons/lib/create-public-import-upload.test.ts` | C | tbd | todo | Coupons and discount codes; builder server code |
| `apps/builder/src/features/coupons/provider/__tests__/coupon-topic-store.test.tsx` | B | - | todo | Coupons and discount codes; builder jsdom |
| `apps/builder/src/features/custom-fields/components/__tests__/field-operation-operand.test.tsx` | B | - | todo | Custom fields; builder jsdom |
| `apps/builder/src/features/custom-fields/components/__tests__/field-value-picker-popover.test.tsx` | B | - | todo | Custom fields; builder jsdom |
| `apps/builder/src/features/custom-fields/lib/__tests__/find-field-by-reference.test.ts` | B | - | todo | Custom fields; builder jsdom |
| `apps/builder/src/features/custom-fields/provider/__tests__/custom-field-hook.test.ts` | B | - | todo | Custom fields; builder jsdom |
| `apps/builder/src/features/decision-connections/schema/public.test.ts` | B | - | todo | Decision engine (routing profiles); builder node-env, client logic |
| `apps/builder/src/features/decision-profiles/components/__tests__/profile-editor.test.tsx` | B | - | todo | Decision engine (routing profiles); builder jsdom |
| `apps/builder/src/features/decision-profiles/lib/to-public-decision-profile.test.ts` | B | - | todo | Decision engine (routing profiles); builder jsdom |
| `apps/builder/src/features/dynamic-images/lib/__tests__/dynamic-image-url.test.ts` | C | tbd | todo | Dynamic images; builder server code |
| `apps/builder/src/features/flows/provider/__tests__/flow-hook.test.tsx` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/__tests__/delete-node-orchestrator.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/__tests__/duplicate-node-orchestrator.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/__tests__/edge-routing.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/__tests__/flow-edit-toolbar-keyboard.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/__tests__/node-hit-test.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/__tests__/react-flow-wrapper-exit-save.test.tsx` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/__tests__/react-flow-wrapper.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/nodes/__tests__/quick-reply-settings-edge.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/nodes/__tests__/selected-node-equality.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/nodes/perform-action/__tests__/menu.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/nodes/send-message/__tests__/menu.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/active-campaign-registration.test.ts` | B | - | todo | ActiveCampaign integration; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/appointment-scheduling-registration.test.ts` | B | - | todo | Appointments and booking; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/execute-javascript-editor.test.tsx` | - | - | dropped | Execute JavaScript flow step; feature dropped (DECISIONS 2026-11-11) |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/execute-javascript-registration.test.ts` | - | - | dropped | Execute JavaScript flow step; feature dropped (DECISIONS 2026-11-11) |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/external-request-registration.test.ts` | B | - | todo | Outbound webhooks; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/get-response-registration.test.ts` | B | - | todo | GetResponse integration; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/json-path.test.ts` | B | - | todo | Flow steps: logic and utilities; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/json-tree-view.test.tsx` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/klaviyo-registration.test.ts` | B | - | todo | Klaviyo integration; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/mailer-lite-registration.test.ts` | B | - | todo | MailerLite integration; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/make-registration.test.ts` | B | - | todo | Make / n8n automation steps; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/message-editor-coupon-variables.test.tsx` | B | - | todo | Coupons and discount codes; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/moosend-registration.test.ts` | B | - | todo | Moosend integration; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/muse-provider-registration.test.ts` | B | - | todo | Muse integration; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/offered-buttons-config.test.ts` | B | - | todo | Flow steps: messages and buttons; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/pipeline-action-fields-variables.test.tsx` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/pipeline-viewer-states.test.tsx` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/sendgrid-registration.test.ts` | B | - | todo | SendGrid integration; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/__tests__/trigger-n8n-registration.test.ts` | B | - | todo | Make / n8n automation steps; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/appointment-scheduling/__tests__/editor.test.tsx` | B | - | todo | Appointments and booking; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/base/__tests__/use-parent-step-commit.test.tsx` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/button/__tests__/media-step-notice.test.tsx` | B | - | todo | Flow steps: messages and buttons; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/button/__tests__/viewer.test.tsx` | B | - | todo | Flow steps: messages and buttons; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/call-phone-number/__tests__/phone-number-field.test.tsx` | B | - | todo | Flow steps: messages and buttons; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/call-phone-number/__tests__/phone-number-parts.test.ts` | B | - | todo | Flow steps: messages and buttons; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/coupon/__tests__/editor.test.tsx` | B | - | todo | Coupons and discount codes; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/coupon/__tests__/viewer.test.tsx` | B | - | todo | Coupons and discount codes; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/pipeline/__tests__/editor.test.tsx` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/pipeline/__tests__/required-fields-warning.test.tsx` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/send-carousel/__tests__/card-link-notice.test.tsx` | B | - | todo | Flow steps: messages and buttons; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/send-carousel/__tests__/validator.test.ts` | B | - | todo | Flow steps: messages and buttons; builder node-env, client logic |
| `apps/builder/src/features/flows/react-flow/steps/send-carousel/__tests__/viewer.test.tsx` | B | - | todo | Flow steps: messages and buttons; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/send-google-ads-conversion/__tests__/steps.test.tsx` | B | - | todo | Google Ads integration; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/send-meta-capi-event/__tests__/viewer.test.tsx` | B | - | todo | Meta Conversions API (CAPI); builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/send-multiple-images/__tests__/viewer.test.tsx` | B | - | todo | Flow steps: messages and buttons; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/send-products/__tests__/form.test.ts` | B | - | todo | Ecommerce: products, categories, vendors; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/send-products/__tests__/product-multi-select.test.tsx` | B | - | todo | Ecommerce: products, categories, vendors; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/send-products/__tests__/send-products-dialog.test.tsx` | B | - | todo | Ecommerce: products, categories, vendors; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/send-products/__tests__/send-products-step.test.tsx` | B | - | todo | Ecommerce: products, categories, vendors; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/send-text/__tests__/character-counter-limit.test.tsx` | B | - | todo | Flow steps: messages and buttons; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/send-text/__tests__/tiktok-title-notice.test.tsx` | B | - | todo | Flow steps: messages and buttons; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/send-view-catalog/__tests__/editor.test.tsx` | B | - | todo | Ecommerce: products, categories, vendors; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/send-view-catalog/__tests__/viewer.test.tsx` | B | - | todo | Ecommerce: products, categories, vendors; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/set-custom-field/__tests__/editor.test.tsx` | B | - | todo | Custom fields; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/start-external-node/__tests__/flow-node-picker-fields.test.tsx` | B | - | todo | Flow steps: logic and utilities; builder jsdom |
| `apps/builder/src/features/flows/react-flow/steps/whatsapp-location/__tests__/editor.test.tsx` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/stores/__tests__/flow-history-store.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/stores/__tests__/flow-template-store.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/flows/react-flow/toolbar/__tests__/duplicate-node-data.test.ts` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/folders/__tests__/change-folder.test.tsx` | B | - | todo | Flow builder and execution; builder jsdom |
| `apps/builder/src/features/import/components/import-error-sample.test.ts` | B | - | todo | Imports and exports (contacts, products, flows); builder jsdom |
| `apps/builder/src/features/inboxes/provider/__tests__/inbox-hook.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/src/features/integration-facebook-ads/lib/__tests__/needs-reconnect.test.ts` | B | - | todo | Facebook Ads integration; builder jsdom |
| `apps/builder/src/features/integration-instagram/actions/__tests__/reconnect-callback.test.ts` | C | tbd | todo | Instagram channel; builder server code |
| `apps/builder/src/features/integration-klaviyo/actions/__tests__/connect.action.test.ts` | C | tbd | todo | Klaviyo integration; builder server code |
| `apps/builder/src/features/integration-messenger/actions/__tests__/disconnect-messenger.test.ts` | C | tbd | todo | Messenger channel; builder server code |
| `apps/builder/src/features/integration-messenger/actions/__tests__/reconnect-callback.test.ts` | C | tbd | todo | Messenger channel; builder server code |
| `apps/builder/src/features/integration-messenger/actions/__tests__/toggle-tag-sync.test.ts` | C | tbd | todo | Messenger channel; builder server code |
| `apps/builder/src/features/integration-whatsapp/actions/__tests__/persist-reconnect-auth.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/src/features/integration-whatsapp/calling/actions/__tests__/claim-conversation-for-call-agent.test.ts` | C | tbd | todo | WhatsApp channel; builder server code |
| `apps/builder/src/features/integration-whatsapp/flows/lib/__tests__/build-flow-field-mappings.test.ts` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/src/features/integration-whatsapp/message-templates/components/__tests__/meta-catalog-product-select.test.tsx` | B | - | todo | WhatsApp channel; builder jsdom |
| `apps/builder/src/features/integration-zalo/actions/__tests__/reconnect-callback.test.ts` | C | tbd | todo | Zalo channel; builder server code |
| `apps/builder/src/features/integration-zalo/actions/__tests__/toggle-tag-sync.test.ts` | C | tbd | todo | Zalo channel; builder server code |
| `apps/builder/src/features/messages/components/__tests__/create-appointment-trigger.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/src/features/messages/components/__tests__/inbound-commerce-bubbles.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/src/features/messages/components/__tests__/input-menu.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/src/features/messages/components/__tests__/product-message-bubble.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/src/features/messages/components/__tests__/send-products-trigger.test.tsx` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/src/features/messages/lib/__tests__/payment-request-content.test.ts` | B | - | todo | Inbox and live chat (conversations, messages); builder jsdom |
| `apps/builder/src/features/minigames/components/play/__tests__/result-dialog-spintax.test.tsx` | B | - | todo | Minigames; builder jsdom |
| `apps/builder/src/features/pipelines/components/board/__tests__/filter-field-section.test.tsx` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/src/features/pipelines/components/board/__tests__/filter-helpers.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/src/features/pipelines/components/board/__tests__/stage-name.test.tsx` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/src/features/pipelines/components/list/__tests__/create-pipeline-form.test.tsx` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/src/features/pipelines/components/settings/__tests__/grant-limits.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/src/features/pipelines/components/settings/__tests__/settings-draft.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/src/features/pipelines/components/settings/__tests__/use-save-settings.test.tsx` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/src/features/pipelines/components/shared/__tests__/date-input.test.tsx` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/src/features/pipelines/lib/__tests__/date-presets.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/src/features/pipelines/lib/__tests__/format-item-value.test.ts` | B | - | todo | Pipelines (CRM deals); builder jsdom |
| `apps/builder/src/features/places-near-me/__tests__/tab.test.ts` | B | - | todo | Places near me; builder jsdom |
| `apps/builder/src/features/products/components/__tests__/meta-catalog-tabs.test.ts` | B | - | todo | Ecommerce: products, categories, vendors; builder jsdom |
| `apps/builder/src/features/products/components/__tests__/product-combobox.test.tsx` | B | - | todo | Ecommerce: products, categories, vendors; builder jsdom |
| `apps/builder/src/features/questionnaires/provider/__tests__/questionnaire-store.test.ts` | B | - | todo | Questionnaires; builder jsdom |
| `apps/builder/src/features/saved-replies/provider/__tests__/saved-reply-hook.test.tsx` | B | - | todo | Saved replies; builder jsdom |
| `apps/builder/src/features/sequences/provider/__tests__/sequence-hook.test.tsx` | B | - | todo | Sequences; builder jsdom |
| `apps/builder/src/features/shared/comment-automation/__tests__/account-choice.test.ts` | B | - | todo | Builder app shell and shared components; builder jsdom |
| `apps/builder/src/features/tags/actions/__tests__/tag-crud-actions.test.ts` | C | tbd | todo | Tags and labels; builder server code |
| `apps/builder/src/features/tags/provider/__tests__/tag-hook.test.ts` | B | - | todo | Tags and labels; builder jsdom |
| `apps/builder/src/features/users/provider/__tests__/assignee-multi-select.test.tsx` | B | - | todo | Workspace members, roles and invitations; builder jsdom |
| `apps/builder/src/features/users/provider/__tests__/user-hook.test.tsx` | B | - | todo | Workspace members, roles and invitations; builder jsdom |
| `apps/builder/src/features/workspaces/__tests__/manage-workspace-tokens.test.tsx` | B | - | todo | Workspaces; builder jsdom |
| `apps/builder/src/features/workspaces/__tests__/workspace-token-action-schema.test.ts` | B | - | todo | Workspaces; builder jsdom |
| `apps/builder/src/features/workspaces/__tests__/workspace-token-dto.test.ts` | B | - | todo | Workspaces; builder jsdom |
| `apps/builder/src/features/workspaces/components/__tests__/account-rail.test.tsx` | B | - | todo | Workspaces; builder jsdom |
| `apps/builder/src/features/workspaces/components/__tests__/workspace-deletion-card.test.tsx` | B | - | todo | Workspaces; builder jsdom |
| `apps/cli/__tests__/command-args.test.ts` | B | - | todo | CLI (chatbotx command); TS client app; runs unchanged against the Rust API |
| `apps/cli/__tests__/dynamic-executor.test.ts` | B | - | todo | CLI (chatbotx command); TS client app; runs unchanged against the Rust API |
| `apps/cli/__tests__/openapi-loader-cache-names.test.ts` | B | - | todo | CLI (chatbotx command); TS client app; runs unchanged against the Rust API |
| `apps/cli/__tests__/openapi-loader-command-names.test.ts` | B | - | todo | CLI (chatbotx command); TS client app; runs unchanged against the Rust API |
| `apps/cli/__tests__/smoke.test.ts` | B | - | todo | CLI (chatbotx command); TS client app; runs unchanged against the Rust API |
| `apps/javascript-executor/__tests__/lifecycle.test.ts` | - | - | dropped | Execute JavaScript flow step; feature dropped (DECISIONS 2026-11-11) |
| `apps/javascript-executor/__tests__/sandbox.test.ts` | - | - | dropped | Execute JavaScript flow step; feature dropped (DECISIONS 2026-11-11) |
| `apps/javascript-executor/__tests__/server.test.ts` | - | - | dropped | Execute JavaScript flow step; feature dropped (DECISIONS 2026-11-11) |
| `apps/mcp-server/__tests__/create-mcp-server.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/mcp-server/__tests__/docker-context.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/mcp-server/__tests__/evals-grading.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/mcp-server/__tests__/evals-multilingual-cases.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/mcp-server/__tests__/evals-run.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/mcp-server/__tests__/evals-sandbox.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/mcp-server/__tests__/execute-tool.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/mcp-server/__tests__/guides.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/mcp-server/__tests__/meta-tools.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/mcp-server/__tests__/oauth.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/mcp-server/__tests__/openapi-loader.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/mcp-server/__tests__/search-normalize.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/mcp-server/__tests__/search-rank.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/mcp-server/__tests__/sse-server.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/mcp-server/__tests__/token-introspection.test.ts` | B | - | todo | MCP server; TS client app; runs unchanged against the Rust API |
| `apps/realtime/__tests__/delivery.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `apps/realtime/__tests__/gateway-config.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `apps/realtime/__tests__/gateway-socket.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `apps/realtime/__tests__/gateway.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `apps/realtime/__tests__/main.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `apps/realtime/__tests__/presence-report.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `apps/realtime/__tests__/presence-reporter.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `apps/realtime/__tests__/replay.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `apps/realtime/__tests__/socket-end.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `apps/realtime/__tests__/stream-reader.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `apps/worker/__tests__/action-executor-contact-inbox-attribution.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `apps/worker/__tests__/active-campaign-handler.test.ts` | C | tbd | todo | ActiveCampaign integration |
| `apps/worker/__tests__/active-campaign-step-registration.test.ts` | C | tbd | todo | ActiveCampaign integration |
| `apps/worker/__tests__/ads-automatic-event.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `apps/worker/__tests__/ads-conversion-worker-dispatch.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `apps/worker/__tests__/agent-tool-stats.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/ai-agent-action-executor.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `apps/worker/__tests__/ai-agent-actions.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `apps/worker/__tests__/ai-agent-handoff-reentry.test.ts` | C | tbd | todo | AI handover (human takeover) |
| `apps/worker/__tests__/ai-agent-runner-openai-compatible.test.ts` | C | tbd | todo | OpenAI-compatible integration |
| `apps/worker/__tests__/ai-agent-worker-audit-context.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `apps/worker/__tests__/ai-analyze-image-handler.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/ai-delete-message-history.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/ai-error-log-provider.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/ai-generate-text-agent-openai-compatible.test.ts` | C | tbd | todo | OpenAI-compatible integration |
| `apps/worker/__tests__/ai-generate-text-handler.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/ai-handover-bulk-toggle.test.ts` | C | tbd | todo | AI handover (human takeover) |
| `apps/worker/__tests__/ai-handover-take-back.test.ts` | C | tbd | todo | AI handover (human takeover) |
| `apps/worker/__tests__/ai-message-history-builders.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/ai-output-length.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/ai-segment-image-step-registration.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/analytics-agent-replied.test.ts` | C | tbd | todo | Analytics dashboards |
| `apps/worker/__tests__/analytics-dashboard-listener.test.ts` | C | tbd | todo | Analytics dashboards |
| `apps/worker/__tests__/appointment-scheduling-handler.test.ts` | C | tbd | todo | Appointments and booking |
| `apps/worker/__tests__/auto-assign-step-handler.test.ts` | C | tbd | todo | Flow steps: contact and conversation actions |
| `apps/worker/__tests__/automated-response-default-reply.test.ts` | C | tbd | todo | Automated responses and keywords |
| `apps/worker/__tests__/automated-response-global-router.test.ts` | C | tbd | todo | Decision engine (routing profiles) |
| `apps/worker/__tests__/automated-response-rich-response.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `apps/worker/__tests__/automated-response-tracking-context.test.ts` | C | tbd | todo | Automated responses and keywords |
| `apps/worker/__tests__/bounded-download.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/bulk-historical-import.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/bulk-import-messages.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/bulk-tag-contacts-handler.test.ts` | C | tbd | todo | Tags and labels |
| `apps/worker/__tests__/call-phone-buttons.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `apps/worker/__tests__/call-phone-number-handler.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `apps/worker/__tests__/capi-scope-checkers.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `apps/worker/__tests__/capture-contact-profile-snapshot.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `apps/worker/__tests__/challenge-handler.test.ts` | C | tbd | todo | Webchat channel |
| `apps/worker/__tests__/change-gamification-score-step.test.ts` | C | tbd | todo | Leaderboards |
| `apps/worker/__tests__/channel-audience-event.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `apps/worker/__tests__/channel-policy-contract.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `apps/worker/__tests__/channel-safe-image.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/chat-worker-quota-gate.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `apps/worker/__tests__/chunkers.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files) |
| `apps/worker/__tests__/clear-custom-field-step.test.ts` | C | tbd | todo | Custom fields |
| `apps/worker/__tests__/clear-expired-support-access.test.ts` | C | tbd | todo | Workspace support access |
| `apps/worker/__tests__/coexist-attachment-download.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/coexist-historical-id.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/coexist-instagram-adapter.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/coexist-instagram-facebook-adapter.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/coexist-instagram-sync.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/coexist-messenger-sync.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/coexist-messenger-template.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/coexist-usage-throttle.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/coexist-whatsapp-buffer.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/coexist-whatsapp-flush-bsuid.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/coexist-whatsapp-flush-lifecycle.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/coexist-whatsapp-flush.payloads.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/coexist-whatsapp-flush.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/coexist-whatsapp-history-photo-intent.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/coexist-whatsapp-pending-patches.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/comment-automation-attachment.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `apps/worker/__tests__/comment-automation-tags.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `apps/worker/__tests__/comment-automation.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `apps/worker/__tests__/comment-media-attachment.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `apps/worker/__tests__/comment-moderation.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `apps/worker/__tests__/condition-evaluator.test.ts` | C | tbd | todo | Triggers |
| `apps/worker/__tests__/condition-handler.test.ts` | C | tbd | todo | Triggers |
| `apps/worker/__tests__/connection-send-gate.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `apps/worker/__tests__/contact-profile-refresh-real-service.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `apps/worker/__tests__/contact-profile-refresh.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `apps/worker/__tests__/contact-scan-engine.test.ts` | C | tbd | todo | Contact scan |
| `apps/worker/__tests__/contact-scan-graph-error.test.ts` | C | tbd | todo | Contact scan |
| `apps/worker/__tests__/contact-scan-instagram-adapter.test.ts` | C | tbd | todo | Instagram channel |
| `apps/worker/__tests__/contact-scan-messenger-adapter.test.ts` | C | tbd | todo | Messenger channel |
| `apps/worker/__tests__/contact-tag-steps.test.ts` | C | tbd | todo | Tags and labels |
| `apps/worker/__tests__/context-source-workspace-scope.test.ts` | C | tbd | todo | Workspaces |
| `apps/worker/__tests__/convert-buttons-call-phone.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `apps/worker/__tests__/convert-buttons-copy-text.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `apps/worker/__tests__/copy-text-handler.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `apps/worker/__tests__/count-characters-step.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `apps/worker/__tests__/coupon-step-handler.test.ts` | C | tbd | todo | Coupons and discount codes |
| `apps/worker/__tests__/create-queue-worker.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/custom-field-value.test.ts` | C | tbd | todo | Custom fields |
| `apps/worker/__tests__/datetime-calculator-timezone.test.ts` | C | tbd | todo | Triggers |
| `apps/worker/__tests__/datetime-trigger-evaluator.test.ts` | C | tbd | todo | Triggers |
| `apps/worker/__tests__/datetime-webhook-evaluator.test.ts` | C | tbd | todo | Outbound webhooks |
| `apps/worker/__tests__/default-worker.test.ts` | C | tbd | todo | Automated responses and keywords |
| `apps/worker/__tests__/deferred-private-reply.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `apps/worker/__tests__/dispatch-profile-snapshots.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `apps/worker/__tests__/docker-entrypoint.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/docker-runtime-deps.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/document-reader-ensure-stored.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files) |
| `apps/worker/__tests__/drip-handler.test.ts` | C | tbd | todo | Drip integration |
| `apps/worker/__tests__/enqueue-attachment-downloads.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/enqueue-broadcast.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `apps/worker/__tests__/enqueue-recurring-broadcasts.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `apps/worker/__tests__/evaluate-conversion-trigger.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `apps/worker/__tests__/evaluate-template-sent.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `apps/worker/__tests__/events-worker-shutdown.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/execute-javascript-handler.test.ts` | - | - | dropped | Execute JavaScript flow step; feature dropped (DECISIONS 2026-11-11) |
| `apps/worker/__tests__/expire-unpaid-orders.test.ts` | C | tbd | todo | Ecommerce: orders and payment providers |
| `apps/worker/__tests__/export-contacts-csv.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/export-contacts-datetime.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/export-contacts-fields.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/export-contacts-handler.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/export-contacts-where.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/export-coupons.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/export-leaderboard-participants.test.ts` | C | tbd | todo | Leaderboards |
| `apps/worker/__tests__/external-request-handler.test.ts` | C | tbd | todo | Outbound webhooks |
| `apps/worker/__tests__/external-request-step-registration.test.ts` | C | tbd | todo | Outbound webhooks |
| `apps/worker/__tests__/extract-data.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/facebook-custom-audience-handler.test.ts` | C | tbd | todo | Facebook Ads integration |
| `apps/worker/__tests__/facebook-custom-audience-step-registration.test.ts` | C | tbd | todo | Facebook Ads integration |
| `apps/worker/__tests__/fan-out-template-update.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `apps/worker/__tests__/finalize-broadcasts.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `apps/worker/__tests__/find-places-near-me-step.test.ts` | C | tbd | todo | Places near me |
| `apps/worker/__tests__/flow-action-sanitize.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `apps/worker/__tests__/flow-continuation-priority.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `apps/worker/__tests__/flow-execution-key.test.ts` | C | tbd | todo | Flow builder and execution |
| `apps/worker/__tests__/flow-import-handler.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/flow-response-files.test.ts` | C | tbd | todo | WhatsApp Flows (forms) |
| `apps/worker/__tests__/flow.test.ts` | C | tbd | todo | Flow builder and execution |
| `apps/worker/__tests__/follow-up-step.test.ts` | C | tbd | todo | Flow steps: contact and conversation actions |
| `apps/worker/__tests__/format-date-step.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `apps/worker/__tests__/get-data-from-json-step.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `apps/worker/__tests__/get-response-handler.test.ts` | C | tbd | todo | GetResponse integration |
| `apps/worker/__tests__/get-user-data.test.ts` | C | tbd | todo | Get user data webview |
| `apps/worker/__tests__/google-ads-housekeeping-schedule.test.ts` | C | tbd | todo | Google Ads integration |
| `apps/worker/__tests__/google-ads-input-error.test.ts` | C | tbd | todo | Google Ads integration |
| `apps/worker/__tests__/google-ads-send-conversion.test.ts` | C | tbd | todo | Google Ads integration |
| `apps/worker/__tests__/google-click-lookup.test.ts` | C | tbd | todo | Google Ads integration |
| `apps/worker/__tests__/handoff-executor.test.ts` | C | tbd | todo | AI handover (human takeover) |
| `apps/worker/__tests__/handover-response.test.ts` | C | tbd | todo | AI handover (human takeover) |
| `apps/worker/__tests__/heavy-analyze-image.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/heavy-provider-error.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/heavy-step-resume.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/heavy-step-runner.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/heavy-timeouts.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/heavy-worker.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/http-retry.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `apps/worker/__tests__/image-options.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/image-reader-ensure-stored.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files) |
| `apps/worker/__tests__/import-contacts-extractor.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/import-contacts-handler.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/import-coupons-handler.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/inbound-webhook-request-crons.test.ts` | C | tbd | todo | Inbound webhooks |
| `apps/worker/__tests__/inbox-labels.test.ts` | C | tbd | todo | Tags and labels |
| `apps/worker/__tests__/install-template-handler.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `apps/worker/__tests__/integration-channel-origin.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/integration-job-context.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/integration-routing.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/integration-worker-boot.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/integration-worker-incoming-message.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `apps/worker/__tests__/integration/heavy-step-outcome.test.ts` | C | tbd | baseline-skip | AI flow steps (text, image, speech, extraction); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `apps/worker/__tests__/integration/inbox-sender.chain.test.ts` | C | tbd | baseline-skip | Inbox and live chat (conversations, messages); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `apps/worker/__tests__/integration/inbox-sender.flow.test.ts` | C | tbd | baseline-skip | Flow builder and execution; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `apps/worker/__tests__/integration/instagram-story.chain.test.ts` | C | tbd | baseline-skip | Instagram channel; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `apps/worker/__tests__/integration/lead-ads.chain.test.ts` | C | tbd | baseline-skip | Facebook Lead Ads automation; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `apps/worker/__tests__/integration/variables.persisted.test.ts` | C | tbd | baseline-skip | Worker runtime (boot, queues, shutdown); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `apps/worker/__tests__/integration/whatsapp-ad.chain.test.ts` | C | tbd | baseline-skip | WhatsApp channel; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `apps/worker/__tests__/integration/whatsapp-conversation-routing.chain.test.ts` | C | tbd | baseline-skip | WhatsApp channel; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `apps/worker/__tests__/integrations-service-threads.test.ts` | C | tbd | todo | Threads channel |
| `apps/worker/__tests__/is-bot-message-quota-reached.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `apps/worker/__tests__/job-attempts.test.ts` | C | tbd | todo | Scheduler and scheduled jobs |
| `apps/worker/__tests__/klaviyo-handler.test.ts` | C | tbd | todo | Klaviyo integration |
| `apps/worker/__tests__/klaviyo-step-registration.test.ts` | C | tbd | todo | Klaviyo integration |
| `apps/worker/__tests__/lead-ads.test.ts` | C | tbd | todo | Facebook Lead Ads automation |
| `apps/worker/__tests__/leaderboard-follower-check.test.ts` | C | tbd | todo | Leaderboards |
| `apps/worker/__tests__/leaderboard-jobs.test.ts` | C | tbd | todo | Leaderboards |
| `apps/worker/__tests__/leaderboard-sources.test.ts` | C | tbd | todo | Leaderboards |
| `apps/worker/__tests__/live-comment.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `apps/worker/__tests__/lock-contention-deferral.test.ts` | C | tbd | todo | Scheduler and scheduled jobs |
| `apps/worker/__tests__/low-worker-boot.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/mailer-lite-handler.test.ts` | C | tbd | todo | MailerLite integration |
| `apps/worker/__tests__/mailer-lite-step-registration.test.ts` | C | tbd | todo | MailerLite integration |
| `apps/worker/__tests__/maintain-mac-partitions.test.ts` | C | tbd | todo | Analytics dashboards |
| `apps/worker/__tests__/make-handler.test.ts` | C | tbd | todo | Make / n8n automation steps |
| `apps/worker/__tests__/mark-conversation-read-state-steps.test.ts` | C | tbd | todo | Flow steps: contact and conversation actions |
| `apps/worker/__tests__/mark-embedding-chunk-failed.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files) |
| `apps/worker/__tests__/marketing-message-webhook.test.ts` | C | tbd | todo | Facebook Marketing Messages |
| `apps/worker/__tests__/marketing-message-worker-boot.test.ts` | C | tbd | todo | Facebook Marketing Messages |
| `apps/worker/__tests__/marketing-messages-schedule.test.ts` | C | tbd | todo | Facebook Marketing Messages |
| `apps/worker/__tests__/materialize-recurring-broadcasts.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `apps/worker/__tests__/message-analytics-quota.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `apps/worker/__tests__/message-failed-error-stack.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `apps/worker/__tests__/message-listener.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `apps/worker/__tests__/messenger-contact-data.test.ts` | C | tbd | todo | Messenger channel |
| `apps/worker/__tests__/messenger-template-handler.test.ts` | C | tbd | todo | Messenger channel |
| `apps/worker/__tests__/messenger-thread-control-rejection.test.ts` | C | tbd | todo | Thread control (handover protocol) |
| `apps/worker/__tests__/meta-catalog-import.test.ts` | C | tbd | todo | Meta Catalog |
| `apps/worker/__tests__/meta-catalog-reconciler.test.ts` | C | tbd | todo | Meta Catalog |
| `apps/worker/__tests__/meta-catalog-sync.test.ts` | C | tbd | todo | Meta Catalog |
| `apps/worker/__tests__/mini-app-buttons.test.ts` | C | tbd | todo | Mini-apps |
| `apps/worker/__tests__/moosend-handler.test.ts` | C | tbd | todo | Moosend integration |
| `apps/worker/__tests__/moosend-rate-limit.test.ts` | C | tbd | todo | Moosend integration |
| `apps/worker/__tests__/moosend-step-registration.test.ts` | C | tbd | todo | Moosend integration |
| `apps/worker/__tests__/muse-image-handlers.test.ts` | C | tbd | todo | Muse integration |
| `apps/worker/__tests__/normalize-epoch-timestamp.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/notification/expo-push-enabled-env.test.ts` | C | tbd | todo | Mobile API and push notifications |
| `apps/worker/__tests__/notification/notify-new-order.test.ts` | C | tbd | todo | Ecommerce: orders and payment providers |
| `apps/worker/__tests__/notification/push-notification-content.test.ts` | C | tbd | todo | Mobile API and push notifications |
| `apps/worker/__tests__/notification/send-push-expo.test.ts` | C | tbd | todo | Mobile API and push notifications |
| `apps/worker/__tests__/operator-handler.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `apps/worker/__tests__/orphaned-integration-cleanup.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/pipeline-matchers.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `apps/worker/__tests__/pipeline-stage-bulk-handler.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `apps/worker/__tests__/pipeline-step.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `apps/worker/__tests__/poll-ref.test.ts` | C | tbd | todo | Polls |
| `apps/worker/__tests__/poll-sentinel-flow-analytics.test.ts` | C | tbd | todo | Polls |
| `apps/worker/__tests__/poll-tap.test.ts` | C | tbd | todo | Polls |
| `apps/worker/__tests__/prepare-broadcast.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `apps/worker/__tests__/process-ai-file.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files) |
| `apps/worker/__tests__/process-broadcast-contacts.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `apps/worker/__tests__/process-conversation-source-ensure-stored.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files) |
| `apps/worker/__tests__/process-inbound-webhook-request.test.ts` | C | tbd | todo | Inbound webhooks |
| `apps/worker/__tests__/process-pending-embeddings.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files) |
| `apps/worker/__tests__/product-import-extractor.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/product-import-handler.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/product-search-reply.test.ts` | C | tbd | todo | Ecommerce: products, categories, vendors |
| `apps/worker/__tests__/provider-account-event.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `apps/worker/__tests__/provider-rate-limiter.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/prune-sourcemaps.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/purge-automation-throttle.test.ts` | C | tbd | todo | Automated responses and keywords |
| `apps/worker/__tests__/purge-broadcasts.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `apps/worker/__tests__/purge-draft-broadcast-recipients.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `apps/worker/__tests__/purge-error-logs.test.ts` | C | tbd | todo | Scheduler and scheduled jobs |
| `apps/worker/__tests__/purge-expired-connect-sessions.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `apps/worker/__tests__/purge-expired-contact-exports.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/purge-expired-oauth-tokens.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `apps/worker/__tests__/purge-whatsapp-signup-sessions.test.ts` | C | tbd | todo | WhatsApp channel |
| `apps/worker/__tests__/purge-workspaces.test.ts` | C | tbd | todo | Workspaces |
| `apps/worker/__tests__/questionnaire-engine.test.ts` | C | tbd | todo | Questionnaires |
| `apps/worker/__tests__/questionnaires-handler.test.ts` | C | tbd | todo | Questionnaires |
| `apps/worker/__tests__/quick-reply-resume.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `apps/worker/__tests__/quick-reply-settings.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `apps/worker/__tests__/read-receipts.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `apps/worker/__tests__/received-message.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `apps/worker/__tests__/reconcile-broadcasts.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `apps/worker/__tests__/reconcile-tenants.test.ts` | C | tbd | todo | Workspaces |
| `apps/worker/__tests__/reconcile.test.ts` | C | tbd | todo | Scheduler and scheduled jobs |
| `apps/worker/__tests__/record-provider-error-log.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/redispatch-template-jobs.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `apps/worker/__tests__/refresh-channel-tokens-table.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `apps/worker/__tests__/refresh-license.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `apps/worker/__tests__/refresh-threads-tokens.test.ts` | C | tbd | todo | Threads channel |
| `apps/worker/__tests__/refresh-tiktok-tokens.test.ts` | C | tbd | todo | TikTok channel |
| `apps/worker/__tests__/refresh-whatsapp-tokens.test.ts` | C | tbd | todo | WhatsApp channel |
| `apps/worker/__tests__/register-schedules-edition.test.ts` | C | tbd | todo | Scheduler and scheduled jobs |
| `apps/worker/__tests__/reply-to-inbound-cart.test.ts` | C | tbd | todo | Ecommerce: products, categories, vendors |
| `apps/worker/__tests__/reregister-telegram-webhooks.test.ts` | C | tbd | todo | Telegram channel |
| `apps/worker/__tests__/resolve-action-contact-inbox.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `apps/worker/__tests__/resolve-matching-templates.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `apps/worker/__tests__/resolve-mcp-token.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `apps/worker/__tests__/resolve-workspace-id.test.ts` | C | tbd | todo | Workspaces |
| `apps/worker/__tests__/resubscribe-messenger-webhook-fields.test.ts` | C | tbd | todo | Messenger channel |
| `apps/worker/__tests__/resubscribe-whatsapp-webhook-fields.test.ts` | C | tbd | todo | WhatsApp channel |
| `apps/worker/__tests__/resume-follow-up.test.ts` | C | tbd | todo | Flow steps: contact and conversation actions |
| `apps/worker/__tests__/resume-wait.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `apps/worker/__tests__/rich-response-tool-preface.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `apps/worker/__tests__/rich-response.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `apps/worker/__tests__/run-import.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/run-job-with-audit-context.test.ts` | C | tbd | todo | Audit logs |
| `apps/worker/__tests__/run-leased-template-job.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `apps/worker/__tests__/run-ref-leaderboard.test.ts` | C | tbd | todo | Leaderboards |
| `apps/worker/__tests__/run-ref-minigame-share.test.ts` | C | tbd | todo | Minigames |
| `apps/worker/__tests__/run-ref-referral.test.ts` | C | tbd | todo | Leaderboards |
| `apps/worker/__tests__/sanitize-capi-error.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `apps/worker/__tests__/save-result-to-custom-field.test.ts` | C | tbd | todo | Custom fields |
| `apps/worker/__tests__/scan-abandoned-carts.test.ts` | C | tbd | todo | Ecommerce: orders and payment providers |
| `apps/worker/__tests__/scan-ai-handover-bulk-runs.test.ts` | C | tbd | todo | AI handover (human takeover) |
| `apps/worker/__tests__/scan-appointment-reminders.test.ts` | C | tbd | todo | Appointments and booking |
| `apps/worker/__tests__/scan-coexist-runs.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `apps/worker/__tests__/scan-contact-scans.test.ts` | C | tbd | todo | Contact scan |
| `apps/worker/__tests__/scan-smart-delay.test.ts` | C | tbd | todo | Smart delay |
| `apps/worker/__tests__/schedule-worker-audit-context.test.ts` | C | tbd | todo | Scheduler and scheduled jobs |
| `apps/worker/__tests__/schedule-worker-connect-session-purge.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `apps/worker/__tests__/schedule-worker-contact-export-purge.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `apps/worker/__tests__/schedule-worker-google-ads-dispatch.test.ts` | C | tbd | todo | Google Ads integration |
| `apps/worker/__tests__/schedule-worker-recurring-broadcasts-dispatch.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `apps/worker/__tests__/schedule-worker-register-failure.test.ts` | C | tbd | todo | Scheduler and scheduled jobs |
| `apps/worker/__tests__/scheduler-tick-interval-env.test.ts` | C | tbd | todo | Scheduler and scheduled jobs |
| `apps/worker/__tests__/segment-image-handler.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/segment-image-retry-policy.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/send-appointment-reminder.test.ts` | C | tbd | todo | Appointments and booking |
| `apps/worker/__tests__/send-audit-log.test.ts` | C | tbd | todo | Audit logs |
| `apps/worker/__tests__/send-conversion-event.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `apps/worker/__tests__/send-email.test.ts` | C | tbd | todo | SMTP email integration |
| `apps/worker/__tests__/send-flow-job.test.ts` | C | tbd | todo | Flow builder and execution |
| `apps/worker/__tests__/send-flow-message.test.ts` | C | tbd | todo | Flow builder and execution |
| `apps/worker/__tests__/send-flow-step.test.ts` | C | tbd | todo | Flow builder and execution |
| `apps/worker/__tests__/send-google-ads-conversion-step-handler.test.ts` | C | tbd | todo | Google Ads integration |
| `apps/worker/__tests__/send-message-handler.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `apps/worker/__tests__/send-messenger-template-create.test.ts` | C | tbd | todo | Messenger channel |
| `apps/worker/__tests__/send-messenger-template.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `apps/worker/__tests__/send-meta-capi-event-step-handler.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `apps/worker/__tests__/send-meta-capi-event.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `apps/worker/__tests__/send-multiple-images-step.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `apps/worker/__tests__/send-typing-to-channel.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `apps/worker/__tests__/send-whatsapp-template.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `apps/worker/__tests__/sendgrid-handler.test.ts` | C | tbd | todo | SendGrid integration |
| `apps/worker/__tests__/sendgrid-step-registration.test.ts` | C | tbd | todo | SendGrid integration |
| `apps/worker/__tests__/sequence-dispatch-processor.test.ts` | C | tbd | todo | Sequences |
| `apps/worker/__tests__/sequence-flow.test.ts` | C | tbd | todo | Sequences |
| `apps/worker/__tests__/sequence-retry-scheduler.test.ts` | C | tbd | todo | Sequences |
| `apps/worker/__tests__/sequence-step-executor.test.ts` | C | tbd | todo | Sequences |
| `apps/worker/__tests__/sequence-worker-consumer.test.ts` | C | tbd | todo | Sequences |
| `apps/worker/__tests__/sequence-worker-logging.test.ts` | C | tbd | todo | Sequences |
| `apps/worker/__tests__/sequence-worker-producer.test.ts` | C | tbd | todo | Sequences |
| `apps/worker/__tests__/set-custom-field-step.test.ts` | C | tbd | todo | Custom fields |
| `apps/worker/__tests__/settle-and-log.test.ts` | C | tbd | todo | Scheduler and scheduled jobs |
| `apps/worker/__tests__/smart-delay-resume-job.test.ts` | C | tbd | todo | Smart delay |
| `apps/worker/__tests__/speech-to-text-handler.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/spreadsheet-handler-bot-fields.test.ts` | C | tbd | todo | Google Sheets integration |
| `apps/worker/__tests__/spreadsheet-handler-temporal.test.ts` | C | tbd | todo | Google Sheets integration |
| `apps/worker/__tests__/spreadsheet-lookup-values.test.ts` | C | tbd | todo | Google Sheets integration |
| `apps/worker/__tests__/spreadsheet-write-values.test.ts` | C | tbd | todo | Google Sheets integration |
| `apps/worker/__tests__/start-external-step-handlers.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `apps/worker/__tests__/step-buttons.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `apps/worker/__tests__/step-input-resolution.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `apps/worker/__tests__/stored-step-content.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `apps/worker/__tests__/story-reply-automation.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `apps/worker/__tests__/summarize-conversation.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/sweep-stale-whatsapp-calls.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `apps/worker/__tests__/sync-ad-labels.test.ts` | C | tbd | todo | Tags and labels |
| `apps/worker/__tests__/sync-channel-labels.test.ts` | C | tbd | todo | Tags and labels |
| `apps/worker/__tests__/sync-retarget-audience.test.ts` | C | tbd | todo | Facebook Ads integration |
| `apps/worker/__tests__/sync-tag-create-attach.test.ts` | C | tbd | todo | Tags and labels |
| `apps/worker/__tests__/sync-tag-delete.test.ts` | C | tbd | todo | Tags and labels |
| `apps/worker/__tests__/sync-tag-detach.test.ts` | C | tbd | todo | Tags and labels |
| `apps/worker/__tests__/sync-tag.test.ts` | C | tbd | todo | Tags and labels |
| `apps/worker/__tests__/sync-template-installation-handler.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `apps/worker/__tests__/sync-user-quota-mac.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `apps/worker/__tests__/sync-user-quota-monthly-bot-messages.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `apps/worker/__tests__/sync-user-quota-reconcile.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `apps/worker/__tests__/system-tools-heavy.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `apps/worker/__tests__/teardown-expired-trial.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `apps/worker/__tests__/template-flow-response.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `apps/worker/__tests__/text-extractor.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files) |
| `apps/worker/__tests__/thread-control-handler.test.ts` | C | tbd | todo | Thread control (handover protocol) |
| `apps/worker/__tests__/thread-control-queue-types.test.ts` | C | tbd | todo | Thread control (handover protocol) |
| `apps/worker/__tests__/thread-control-send-gate.test.ts` | C | tbd | todo | Thread control (handover protocol) |
| `apps/worker/__tests__/thread-control-step.test.ts` | C | tbd | todo | Thread control (handover protocol) |
| `apps/worker/__tests__/tiktok-comment-identity.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `apps/worker/__tests__/tiktok-high-intent-comment.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `apps/worker/__tests__/trigger-action-executor-add-tag.test.ts` | C | tbd | todo | Triggers |
| `apps/worker/__tests__/trigger-action-executor-bot-field.test.ts` | C | tbd | todo | Bot fields |
| `apps/worker/__tests__/trigger-action-executor-conversation.test.ts` | C | tbd | todo | Triggers |
| `apps/worker/__tests__/trigger-action-executor-pipeline.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `apps/worker/__tests__/trigger-action-executor-send-google-ads-conversion.test.ts` | C | tbd | todo | Google Ads integration |
| `apps/worker/__tests__/trigger-action-executor-send-meta-capi-event.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `apps/worker/__tests__/trigger-action-executor-sequence.test.ts` | C | tbd | todo | Sequences |
| `apps/worker/__tests__/trigger-action-executor-start-external-step.test.ts` | C | tbd | todo | Triggers |
| `apps/worker/__tests__/trigger-action-scope-sequence.test.ts` | C | tbd | todo | Sequences |
| `apps/worker/__tests__/trigger-action-scope.test.ts` | C | tbd | todo | Triggers |
| `apps/worker/__tests__/trigger-executor-occurrence-key.test.ts` | C | tbd | todo | Triggers |
| `apps/worker/__tests__/trigger-executor-run-limit.test.ts` | C | tbd | todo | Triggers |
| `apps/worker/__tests__/trigger-n8n-handler.test.ts` | C | tbd | todo | Make / n8n automation steps |
| `apps/worker/__tests__/trigger-worker-audit-context.test.ts` | C | tbd | todo | Triggers |
| `apps/worker/__tests__/trigger-worker-contact-inbox-propagation.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `apps/worker/__tests__/uninstall-template-handler.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `apps/worker/__tests__/unsubscribe-expired-trials.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `apps/worker/__tests__/update-contact-avatar.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `apps/worker/__tests__/wa-payment-request-guard.test.ts` | C | tbd | todo | WhatsApp message templates |
| `apps/worker/__tests__/wa-template-handler.test.ts` | C | tbd | todo | WhatsApp message templates |
| `apps/worker/__tests__/wait-for-chat-job-completion.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `apps/worker/__tests__/webhook-executor-payloads.test.ts` | C | tbd | todo | Outbound webhooks |
| `apps/worker/__tests__/webhook-matcher-catalog.test.ts` | C | tbd | todo | Outbound webhooks |
| `apps/worker/__tests__/webhook-matcher-datetime.test.ts` | C | tbd | todo | Outbound webhooks |
| `apps/worker/__tests__/webhook-payload-contact-inbox-leak.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `apps/worker/__tests__/webhook-pipeline-payload.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `apps/worker/__tests__/webhook-worker-audit-context.test.ts` | C | tbd | todo | Outbound webhooks |
| `apps/worker/__tests__/welcome-message-flows-handler.test.ts` | C | tbd | todo | Messenger channel |
| `apps/worker/__tests__/whatsapp-call-finalize.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `apps/worker/__tests__/whatsapp-call-handler.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `apps/worker/__tests__/whatsapp-call-native-media.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `apps/worker/__tests__/whatsapp-call-native-recording.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `apps/worker/__tests__/whatsapp-call-native-transcript.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `apps/worker/__tests__/whatsapp-call-permission-grant.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `apps/worker/__tests__/whatsapp-call-recording.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `apps/worker/__tests__/whatsapp-call-transcribe.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `apps/worker/__tests__/whatsapp-flow-response.test.ts` | C | tbd | todo | WhatsApp Flows (forms) |
| `apps/worker/__tests__/whatsapp-identity-change.test.ts` | C | tbd | todo | WhatsApp channel |
| `apps/worker/__tests__/whatsapp-voip-signaling.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `apps/worker/__tests__/will-send-retry.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `apps/worker/__tests__/worker-consumer.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/worker-entries.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/worker-shutdown.test.ts` | C | tbd | todo | Worker runtime (boot, queues, shutdown) |
| `apps/worker/__tests__/write-error-log.test.ts` | C | tbd | todo | Error logs |
| `apps/worker/src/default/handlers/meta-catalog/__tests__/safe-error-log.test.ts` | C | tbd | todo | Meta Catalog |
| `apps/worker/src/integration/handlers/comment-automation/__tests__/automation-matching.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `integrations/active-campaign/__tests__/connection.test.ts` | C | tbd | todo | ActiveCampaign integration |
| `integrations/active-campaign/__tests__/integration.test.ts` | C | tbd | todo | ActiveCampaign integration |
| `integrations/api/__tests__/incoming-message.test.ts` | C | tbd | todo | API channel integration (custom channels) |
| `integrations/api/__tests__/outgoing-message.test.ts` | C | tbd | todo | API channel integration (custom channels) |
| `integrations/chatbotx/__tests__/smoke.test.ts` | C | tbd | todo | ChatbotX integration (cross-instance client) |
| `integrations/drip/__tests__/connection.test.ts` | C | tbd | todo | Drip integration |
| `integrations/drip/__tests__/integration.test.ts` | C | tbd | todo | Drip integration |
| `integrations/facebook-ads/__tests__/ad-accounts.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/adcreatives.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/adimages.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/ads.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/adsets.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/advideos.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/audience-users.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/campaigns.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/connection-oauth.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/custom-audiences.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/customer-file-normalizers.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/customer-file-payload.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/exception.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/insights.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/marketing-messages-ad-accounts.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/marketing-messages-audiences.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/marketing-messages-error-classes.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/marketing-messages-insights.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/marketing-messages-send.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/message-campaigns.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/messaging-ads-config.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/messaging-ads-insights.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/facebook-ads/__tests__/welcome-message.test.ts` | C | tbd | todo | Facebook Ads integration |
| `integrations/get-response/__tests__/client.test.ts` | C | tbd | todo | GetResponse integration |
| `integrations/get-response/__tests__/connection.test.ts` | C | tbd | todo | GetResponse integration |
| `integrations/get-response/__tests__/integration.test.ts` | C | tbd | todo | GetResponse integration |
| `integrations/google-ads/__tests__/candidates-logging.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-ads/__tests__/connection-oauth.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-ads/__tests__/data-manager.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-ads/__tests__/exception.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-ads/__tests__/failure-cause.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-ads/__tests__/google-ads-api.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-ads/__tests__/http-client-url.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-ads/__tests__/http-client.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-ads/__tests__/legacy-order-id.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-ads/__tests__/sanitize.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-ads/__tests__/scopes.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-ads/__tests__/wire-connect.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-ads/__tests__/wire-data-manager.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-ads/__tests__/wire-google-ads.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-ads/__tests__/wire-legacy-upload.test.ts` | C | tbd | todo | Google Ads integration |
| `integrations/google-calendar/__tests__/connection-oauth.test.ts` | C | tbd | todo | Google Calendar integration |
| `integrations/google-calendar/__tests__/connection-verify.test.ts` | C | tbd | todo | Google Calendar integration |
| `integrations/google-calendar/__tests__/smoke.test.ts` | C | tbd | todo | Google Calendar integration |
| `integrations/google-sheets/__tests__/connection-oauth.test.ts` | C | tbd | todo | Google Sheets integration |
| `integrations/google-sheets/__tests__/legacy-auth-url.test.ts` | C | tbd | todo | Google Sheets integration |
| `integrations/google-sheets/__tests__/write-value-input.test.ts` | C | tbd | todo | Google Sheets integration |
| `integrations/instagram-facebook/__tests__/auth-scopes.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/call-phone-number-button.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/capi-auth-url.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/comment-automation-button-payload.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/comment-username-passthrough.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/connection-oauth.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/eager-media-compression.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/error-mapper.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/get-contact-profile-pic-url.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/get-message-media-urls.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/get-page-instagram-access.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/get-user-profile-avatar.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/http-client-policy-error.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/incoming-attachment-storage.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/incoming-lazy-media.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/incoming-message-referral.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/list-instagram-media.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/live-comments-webhook.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/outgoing-message-ids.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/post-details-mapping.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/send-comment-media-unsupported.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/send-comment-no-retry.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/send-flow-step-comment-anchor.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/send-flow-step-media.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/send-message-multiple-images.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/send-private-reply.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/story-mention.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/sync-api.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/take-thread-control.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/unsubscribe-instagram-facebook-webhook.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram-facebook/__tests__/webhook-integration-type.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/auth.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/call-phone-number-button.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/comment-automation-api.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/comment-automation-button-payload.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/comment-username-passthrough.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/connection.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/contact-profile.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/eager-media-compression.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/get-contact-profile-pic-url.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/get-message-media-urls.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/get-user-profile-avatar.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/http-client-policy-error.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/incoming-attachment.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/incoming-lazy-media.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/incoming-message-referral.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/live-comments-webhook.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/message-echo-webhook.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/outgoing-message-ids.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/post-details-mapping.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/quick-replies-attachment.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/send-comment-media-unsupported.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/send-comment-no-retry.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/send-flow-step-comment-anchor.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/send-flow-step-media.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/send-message-multiple-images.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/send-multiple-images.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/story-mention.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/sync-api.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/instagram/__tests__/unsubscribe-instagram-webhook.test.ts` | C | tbd | todo | Instagram channel |
| `integrations/klaviyo/__tests__/client.test.ts` | C | tbd | todo | Klaviyo integration |
| `integrations/klaviyo/__tests__/connection.test.ts` | C | tbd | todo | Klaviyo integration |
| `integrations/klaviyo/__tests__/integration.test.ts` | C | tbd | todo | Klaviyo integration |
| `integrations/mailchimp/__tests__/connection.test.ts` | C | tbd | todo | Mailchimp integration |
| `integrations/mailer-lite/__tests__/client.test.ts` | C | tbd | todo | MailerLite integration |
| `integrations/mailer-lite/__tests__/connection.test.ts` | C | tbd | todo | MailerLite integration |
| `integrations/mailer-lite/__tests__/integration.test.ts` | C | tbd | todo | MailerLite integration |
| `integrations/messenger/__tests__/auth-scopes.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/bulk-thread-control.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/call-phone-number-button.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/capi-auth-url.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/comment-automation-button-payload.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/comment-gif-attachment.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/comment-message-tags.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/comment-video.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/connection-oauth.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/conversation-routing-handlers.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/conversation-routing-parsers.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/disconnect.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/eager-media-compression.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/error-mapper.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/generate-auth-url.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/get-message-media-urls.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/get-user-pages.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/get-user-profile-avatar.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/get-user-profile.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/google-click-ref.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/graph-error-message.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/http-client-policy-error.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/incoming-attachment.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/incoming-lazy-media.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/incoming-message-referral.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/incoming-message-template-echo.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/incoming-message-thread-control.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/label.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/leadgen-form-name.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/list-reels-posts.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/marketing-messages-auth-url.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/message-templates-api.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/messenger-ads-json.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/messenger-welcome-profile.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/outgoing-message-ids.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/outgoing-message-policy.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/persistent-menu.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/persona.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/post-details-mapping.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/quick-replies-attachment.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/schema.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/send-carousel-layout.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/send-collection.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/send-comment-no-retry.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/send-flow-step-comment-anchor.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/send-media-v2.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/send-message-multiple-images.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/send-message-quick-replies.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/send-messenger-template.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/send-multiple-images.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/send-private-reply.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/sync.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/unsubscribe-app-webhook.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/upload.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/user-inbox-link.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/webhook-conversation-routing.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/webhook-label-enqueue.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/webhook-marketing-messages.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/messenger/__tests__/welcome-message-flows-api.test.ts` | C | tbd | todo | Messenger channel |
| `integrations/meta-catalog/__tests__/auth.test.ts` | C | tbd | todo | Meta Catalog |
| `integrations/meta-catalog/__tests__/catalogs.test.ts` | C | tbd | todo | Meta Catalog |
| `integrations/meta-catalog/__tests__/exception.test.ts` | C | tbd | todo | Meta Catalog |
| `integrations/meta-catalog/__tests__/http-client.test.ts` | C | tbd | todo | Meta Catalog |
| `integrations/meta-catalog/__tests__/items-batch.test.ts` | C | tbd | todo | Meta Catalog |
| `integrations/meta-catalog/__tests__/mapper.test.ts` | C | tbd | todo | Meta Catalog |
| `integrations/meta-catalog/__tests__/products.test.ts` | C | tbd | todo | Meta Catalog |
| `integrations/meta-catalog/__tests__/remote-product.test.ts` | C | tbd | todo | Meta Catalog |
| `integrations/meta-catalog/__tests__/retailer-id.test.ts` | C | tbd | todo | Meta Catalog |
| `integrations/meta-conversions/__tests__/dataset.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `integrations/meta-conversions/__tests__/events.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `integrations/meta-conversions/__tests__/exception.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `integrations/moosend/__tests__/client.test.ts` | C | tbd | todo | Moosend integration |
| `integrations/moosend/__tests__/connection.test.ts` | C | tbd | todo | Moosend integration |
| `integrations/moosend/__tests__/integration.test.ts` | C | tbd | todo | Moosend integration |
| `integrations/openai/__tests__/smoke.test.ts` | C | tbd | todo | OpenAI integration |
| `integrations/sendgrid/__tests__/client.test.ts` | C | tbd | todo | SendGrid integration |
| `integrations/sendgrid/__tests__/connection.test.ts` | C | tbd | todo | SendGrid integration |
| `integrations/sendgrid/__tests__/integration.test.ts` | C | tbd | todo | SendGrid integration |
| `integrations/shopify/__tests__/lib.test.ts` | C | tbd | todo | Shopify integration |
| `integrations/smtp/__tests__/smoke.test.ts` | C | tbd | todo | SMTP email integration |
| `integrations/telegram/__tests__/call-phone-number-button.test.ts` | C | tbd | todo | Telegram channel |
| `integrations/telegram/__tests__/connection.test.ts` | C | tbd | todo | Telegram channel |
| `integrations/telegram/__tests__/copy-text-button.test.ts` | C | tbd | todo | Telegram channel |
| `integrations/telegram/__tests__/error-mapper.test.ts` | C | tbd | todo | Telegram channel |
| `integrations/telegram/__tests__/group-routing.test.ts` | C | tbd | todo | Telegram channel |
| `integrations/telegram/__tests__/incoming-image-compression.test.ts` | C | tbd | todo | Telegram channel |
| `integrations/telegram/__tests__/incoming-message.test.ts` | C | tbd | todo | Telegram channel |
| `integrations/telegram/__tests__/quick-replies-attachment.test.ts` | C | tbd | todo | Telegram channel |
| `integrations/telegram/__tests__/send-message-multiple-images.test.ts` | C | tbd | todo | Telegram channel |
| `integrations/telegram/__tests__/send-multiple-images.test.ts` | C | tbd | todo | Telegram channel |
| `integrations/threads/__tests__/auth.test.ts` | C | tbd | todo | Threads channel |
| `integrations/threads/__tests__/comment-moderation.test.ts` | C | tbd | todo | Threads channel |
| `integrations/threads/__tests__/comment.test.ts` | C | tbd | todo | Threads channel |
| `integrations/threads/__tests__/connection.test.ts` | C | tbd | todo | Threads channel |
| `integrations/threads/__tests__/errors-and-integration.test.ts` | C | tbd | todo | Threads channel |
| `integrations/threads/__tests__/post.test.ts` | C | tbd | todo | Threads channel |
| `integrations/threads/__tests__/smoke.test.ts` | C | tbd | todo | Threads channel |
| `integrations/threads/__tests__/webhook.test.ts` | C | tbd | todo | Threads channel |
| `integrations/tiktok/__tests__/call-phone-number-button.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/callback.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/comment-api.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/comment-webhook.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/connection-oauth.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/direct-reply-api.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/high-intent-comment-webhook.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/incoming-message.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/message-send.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/outgoing-comment.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/private-reply.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/quick-replies-attachment.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/scopes.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/send-multiple-images.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/subscribe-webhooks.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/tiktok/__tests__/webhook.test.ts` | C | tbd | todo | TikTok channel |
| `integrations/webchat/__tests__/integration.test.ts` | C | tbd | todo | Webchat channel |
| `integrations/webchat/__tests__/realtime-sender.test.ts` | C | tbd | todo | Webchat channel |
| `integrations/webchat/__tests__/send-flow-step.test.ts` | C | tbd | todo | Webchat channel |
| `integrations/whatsapp/__tests__/account-update-webhook.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/app-subscriptions.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/auth.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/automatic-events-webhook.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/bsuid-send-status.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/call-permission-public-message.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/call-permission-reply.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/call-permissions.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/call-phone-number-button.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/calling-recording.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/calling-voip-actions.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/calls-webhook.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/coexist-webhook.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/coexists.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/connection-revocation.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/connection.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/contact-without-profile-webhook.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/conversation-handlers.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/conversation-routing-api.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/conversation-routing-extractor.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/conversation-routing-handlers.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/conversation-routing-parsers.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/conversions.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/delete-flow.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/delivery-receipts.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/error-mapper-calling.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/error-mapper-thread-control.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/flow-response-fields.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/google-click-capture.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/granted-scopes.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/incoming-flow-media.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/incoming-media-inline-compression.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/incoming-message-bsuid.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/incoming-message-parsers.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/incoming-message-referral.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/origin-error.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/outbound-media.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/outgoing-call-button.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/outgoing-call-permission-request.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/outgoing-carousel-card.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/outgoing-link-buttons.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/outgoing-location-request.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/outgoing-location.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/outgoing-message-bsuid.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/outgoing-message-ids.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/outgoing-reply-buttons.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/outgoing-send-products.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/outgoing-send-view-catalog.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/outgoing-template-params.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/payment-method-reply.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/phone-number-pagination.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/phone-number.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/publish-flow-json.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/quick-replies-attachment.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/reaction.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/send-multiple-images.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/signature-policy.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/subscribe-webhook.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/system-message-webhook.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/system-messages.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/thread-control-api.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/waba-candidates.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/waba-owner.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/waba-pagination.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/waba-setup.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/whatsapp/__tests__/webhook-hmac.test.ts` | C | tbd | todo | WhatsApp channel |
| `integrations/woocommerce/__tests__/client.test.ts` | C | tbd | todo | WooCommerce integration |
| `integrations/woocommerce/__tests__/lib.test.ts` | C | tbd | todo | WooCommerce integration |
| `integrations/zalo/__tests__/call-phone-number-button.test.ts` | C | tbd | todo | Zalo channel |
| `integrations/zalo/__tests__/connection-oauth.test.ts` | C | tbd | todo | Zalo channel |
| `integrations/zalo/__tests__/error-mapper.test.ts` | C | tbd | todo | Zalo channel |
| `integrations/zalo/__tests__/follow-event.test.ts` | C | tbd | todo | Zalo channel |
| `integrations/zalo/__tests__/get-user-profile.test.ts` | C | tbd | todo | Zalo channel |
| `integrations/zalo/__tests__/google-click-partner-samples.test.ts` | C | tbd | todo | Zalo channel |
| `integrations/zalo/__tests__/incoming-gif-message.test.ts` | C | tbd | todo | Zalo channel |
| `integrations/zalo/__tests__/incoming-image-compression.test.ts` | C | tbd | todo | Zalo channel |
| `integrations/zalo/__tests__/outgoing-message-ids.test.ts` | C | tbd | todo | Zalo channel |
| `integrations/zalo/__tests__/outgoing-text-with-images.test.ts` | C | tbd | todo | Zalo channel |
| `integrations/zalo/__tests__/quick-replies-attachment.test.ts` | C | tbd | todo | Zalo channel |
| `integrations/zalo/__tests__/refresh-access-token.test.ts` | C | tbd | todo | Zalo channel |
| `integrations/zalo/__tests__/send-multiple-images.test.ts` | C | tbd | todo | Zalo channel |
| `integrations/zalo/__tests__/tag.test.ts` | C | tbd | todo | Zalo channel |
| `integrations/zalo/__tests__/webhook-routing.test.ts` | C | tbd | todo | Zalo channel |
| `packages/ai/__tests__/ai-context-service.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/call-summarizer.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/embedding-model.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/extensions.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/file-tools-logging.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/first-chunk-timeout.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/knowledge-base-prompt.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/knowledge-base-search.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/knowledge-search.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/mcp-client.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/mcp-tools.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/model-catalog.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/muse-provider.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/muse-transcribe.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/openai-compatible-presets.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/openai-compatible.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/prompt-utils.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/provider-errors.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/provider-factory.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/sam.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/stream.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/ai/__tests__/summarizer-openai-compatible.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/analytics-nextjs/__tests__/admins-response-time-columns.test.tsx` | B | - | todo | Analytics dashboards; frontend package |
| `packages/analytics-nextjs/__tests__/agent-response-series-route.test.ts` | B | - | todo | Analytics dashboards; frontend package |
| `packages/analytics-nextjs/__tests__/agent-response-series.test.ts` | B | - | todo | Analytics dashboards; frontend package |
| `packages/analytics-nextjs/__tests__/analysis-store.test.ts` | B | - | todo | Analytics dashboards; frontend package |
| `packages/analytics-nextjs/__tests__/analytics-api-context.test.tsx` | B | - | todo | Analytics dashboards; frontend package |
| `packages/analytics-nextjs/__tests__/channel-filter.test.tsx` | B | - | todo | Analytics dashboards; frontend package |
| `packages/analytics-nextjs/__tests__/dashboard-panel.test.tsx` | B | - | todo | Analytics dashboards; frontend package |
| `packages/analytics-nextjs/__tests__/date-range-preset-filter.test.tsx` | B | - | todo | Analytics dashboards; frontend package |
| `packages/analytics-nextjs/__tests__/filter-form.test.tsx` | B | - | todo | Analytics dashboards; frontend package |
| `packages/analytics-nextjs/__tests__/format-duration.test.ts` | B | - | todo | Analytics dashboards; frontend package |
| `packages/analytics-nextjs/__tests__/resolve-preset-option.test.ts` | B | - | todo | Analytics dashboards; frontend package |
| `packages/analytics/__tests__/agent-response-analytics.service.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/agent-response-stats.repository.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/broadcast-analytics.service.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/broadcast-stats.repository.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/comment-automation-analytics.service.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/comment-automation-stats.repository.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/contact-analytics.service.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/contact-stats.repository.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/contacts-by-dimension-schema.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/email-topic-stats.repository.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/event-bus-idempotency.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/event-filter.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/filtered-stats.repository.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/flow-stats-reset.repository.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/human-agent-stats-response.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/mac-analytics.service.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/mac-period.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/mac-tracking.service.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/mac.repository.sql.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/mac.repository.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/magic-link-analytics.service.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/ref-link-analytics.service.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/sequence-analytics.service.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/stats-contact-ids-page.repository.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/stats-occurred-at.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/analytics/__tests__/time-series.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/auth/__tests__/account-linking.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/bearer-plugin.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/claim-unverified-account.integration.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/claim-unverified-account.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/generic-oauth-config.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/generic-oauth-path.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/id-precision.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/mcp-default-resource.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/mcp-resource.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/mobile-auth.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/mobile-resource.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/provisioning.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/schema-parity.test.ts` | C | `crates/db/tests/schema_parity.rs` | passing | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/send-magic-link.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/signup-gate.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/social-redirect-origin.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/tenant-credential-lookup.integration.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/tenant-scoping.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/auth/__tests__/trusted-origins-build-phase.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/automated-response/__tests__/dispatch-reply-ads-conversion.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/automated-response/__tests__/dispatch-reply-contact-activity.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/automated-response/__tests__/dispatch-reply-leaderboard.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/automated-response/__tests__/enqueue-flow-action.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/automated-response/__tests__/enqueue-handoff-reentry.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/automated-response/__tests__/enqueue-message.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/automated-response/__tests__/keyword-match.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/automated-response/__tests__/process-messages.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/automated-response/__tests__/process-outbound-message.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/automated-response/__tests__/smart-delay.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/business/__tests__/ad-source-platform.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/business/__tests__/admin-workspace-service.test.ts` | C | tbd | todo | Workspaces |
| `packages/business/__tests__/ads-analytics-ad-account-filter.test.ts` | C | tbd | todo | Facebook Ads integration |
| `packages/business/__tests__/ads-analytics-context-memoization.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/business/__tests__/ads-analytics-date-range.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/business/__tests__/ads-analytics-merge.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/business/__tests__/ads-analytics-timeseries.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/business/__tests__/ads-conversion-capi-status.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `packages/business/__tests__/ads-conversion-channel-fields-referral.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/business/__tests__/ads-conversion-export-channel-default.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/business/__tests__/ads-conversion-funnel-service.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/business/__tests__/ads-conversion-funnel.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/business/__tests__/ads-conversion-rule.service.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/business/__tests__/ads-retarget-service.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/business/__tests__/ai-agent-template-lock.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/business/__tests__/ai-agent.service.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/business/__tests__/ai-file.service.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files) |
| `packages/business/__tests__/ai-function.service.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/business/__tests__/ai-handover-bulk-run-service.test.ts` | C | tbd | todo | AI handover (human takeover) |
| `packages/business/__tests__/ai-handover-settings-policy.test.ts` | C | tbd | todo | AI handover (human takeover) |
| `packages/business/__tests__/ai-handover-settings-service.test.ts` | C | tbd | todo | AI handover (human takeover) |
| `packages/business/__tests__/ai-mcp-endpoint-policy.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/business/__tests__/ai-mcp-server.service.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/business/__tests__/appointment-calendar-service-create.test.ts` | C | tbd | todo | Appointments and booking |
| `packages/business/__tests__/appointment-calendar-service-update.test.ts` | C | tbd | todo | Appointments and booking |
| `packages/business/__tests__/appointment-calendar-service.test.ts` | C | tbd | todo | Appointments and booking |
| `packages/business/__tests__/appointment-external-calendar-service.test.ts` | C | tbd | todo | Google Calendar integration |
| `packages/business/__tests__/appointment-inbox-booking-link.test.ts` | C | tbd | todo | Appointments and booking |
| `packages/business/__tests__/appointment-reminder-service.test.ts` | C | tbd | todo | Appointments and booking |
| `packages/business/__tests__/appointment-service-list-contact.test.ts` | C | tbd | todo | Appointments and booking |
| `packages/business/__tests__/appointment-service-webview.test.ts` | C | tbd | todo | Appointments and booking |
| `packages/business/__tests__/audit-actor.test.ts` | C | tbd | todo | Audit logs |
| `packages/business/__tests__/audit-dispatcher.test.ts` | C | tbd | todo | Audit logs |
| `packages/business/__tests__/audit-log-query.test.ts` | C | tbd | todo | Audit logs |
| `packages/business/__tests__/audit.service.test.ts` | C | tbd | todo | Audit logs |
| `packages/business/__tests__/automated-response.service.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/business/__tests__/automation-action-references.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/business/__tests__/automation-throttle.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/business/__tests__/backfill-language-parity.test.ts` | C | tbd | todo | i18n (UI translations) |
| `packages/business/__tests__/billing.service.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/bot-field-resolve-by-name-and-type.test.ts` | C | tbd | todo | Bot fields |
| `packages/business/__tests__/bot-field-unique-violation.test.ts` | C | tbd | todo | Bot fields |
| `packages/business/__tests__/bot-field-value-operation.test.ts` | C | tbd | todo | Bot fields |
| `packages/business/__tests__/branding-menu.test.ts` | C | tbd | todo | Multi-tenancy (white-label tenants) |
| `packages/business/__tests__/broadcast-plan-policy-service.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/__tests__/broadcast-recurrence-ledger.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/__tests__/broadcast-recurrence-service.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/__tests__/broadcast-service-audience.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/__tests__/broadcast-service-clone.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/__tests__/broadcast-service-create.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/__tests__/broadcast-service-drafts.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/__tests__/broadcast-service-lifecycle.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/__tests__/broadcast-service-list-contacts.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/__tests__/broadcast-service-payment-request.test.ts` | C | tbd | todo | WhatsApp message templates |
| `packages/business/__tests__/broadcast-service-prepare.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/__tests__/broadcast-service-resend.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/__tests__/broadcast-service-transitions.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/__tests__/broadcast-service-update.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/__tests__/build-context.test.ts` | C | tbd | todo | Business layer shared services |
| `packages/business/__tests__/bulk-import-channel-contacts.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/business/__tests__/campaign-cleanup.test.ts` | C | tbd | todo | Business layer shared services |
| `packages/business/__tests__/can-use-own-credentials.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/__tests__/capabilities-service.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/channel-ad-accounts.test.ts` | C | tbd | todo | Facebook Ads integration |
| `packages/business/__tests__/channel-integration-service.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/__tests__/channel-post.service.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `packages/business/__tests__/coexist-history-window.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `packages/business/__tests__/coexist-import-service.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `packages/business/__tests__/coexist.service.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `packages/business/__tests__/comment-automation-account-scope.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `packages/business/__tests__/comment-automation-write-methods.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `packages/business/__tests__/comment-automation.list.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `packages/business/__tests__/comment-automation.missed-comments.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `packages/business/__tests__/comment-automation.service.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `packages/business/__tests__/connect-channel.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/__tests__/connect-outcome-parity.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/__tests__/connect-outcome-types.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/__tests__/connect-outcome.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/__tests__/connection-store-bindings-jsonb-defaults.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/__tests__/contact-activity.service.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-create-with-inbox.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-custom-field-apply-operation.test.ts` | C | tbd | todo | Custom fields |
| `packages/business/__tests__/contact-custom-field-bot-routing.test.ts` | C | tbd | todo | Custom fields |
| `packages/business/__tests__/contact-custom-field-delete.test.ts` | C | tbd | todo | Custom fields |
| `packages/business/__tests__/contact-custom-field-list-values-by-keys.test.ts` | C | tbd | todo | Custom fields |
| `packages/business/__tests__/contact-custom-field-normalize.test.ts` | C | tbd | todo | Custom fields |
| `packages/business/__tests__/contact-custom-field-set-values-runtime-coerce.test.ts` | C | tbd | todo | Custom fields |
| `packages/business/__tests__/contact-custom-field-set-values.test.ts` | C | tbd | todo | Custom fields |
| `packages/business/__tests__/contact-delete-and-record.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-export-service.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/business/__tests__/contact-filter-value-labels.test.ts` | C | tbd | todo | Contact filters and segments |
| `packages/business/__tests__/contact-inbox-identity-change.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-inbox-identity-rotation-matrix.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-inbox-identity-sync.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-inbox-identity.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-inbox-post.service.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-inbox-service.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-info-changes.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-insert-imported-batch.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/business/__tests__/contact-list-by-custom-field-value.test.ts` | C | tbd | todo | Custom fields |
| `packages/business/__tests__/contact-list.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-locale.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-note-service.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-phone.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-profile-refresh.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-referral-event-ad-touch.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/business/__tests__/contact-referral-event-history.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/business/__tests__/contact-referral-event-service.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/business/__tests__/contact-resolve-id-by-identifier.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-rich-system-fields.test.ts` | C | tbd | todo | System fields |
| `packages/business/__tests__/contact-scan-availability.test.ts` | C | tbd | todo | Contact scan |
| `packages/business/__tests__/contact-scan.service.test.ts` | C | tbd | todo | Contact scan |
| `packages/business/__tests__/contact-sequence-subscribe.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-sequence-unsubscribe.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-sequence.service.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-service-adopt-phone-number.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-service-backdate-created-at.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-service-delete.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-service-flow-flags.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-service-matches-filter.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-service-set-broadcast-subscription.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-service-update-events.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-service-update-if-profile-name-empty.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-service-upsert.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-service.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/contact-update-fields.test.ts` | C | tbd | todo | Custom fields |
| `packages/business/__tests__/conversation-find-by-contact-with-inboxes.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/conversation-last-incoming-source-id.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/conversation-resolve-contact-inbox-for-send.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/conversation-service-consume-challenge.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/conversation-service.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/copy-name.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/count-mode.test.ts` | C | tbd | todo | Business layer shared services |
| `packages/business/__tests__/coupon-service.test.ts` | C | tbd | todo | Coupons and discount codes |
| `packages/business/__tests__/create-outgoing-agent-replied.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/business/__tests__/custom-domain-service.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/custom-field-resolve-by-name-and-type.test.ts` | C | tbd | todo | Custom fields |
| `packages/business/__tests__/custom-field-runtime-coerce.test.ts` | C | tbd | todo | Custom fields |
| `packages/business/__tests__/custom-field-service-update.test.ts` | C | tbd | todo | Custom fields |
| `packages/business/__tests__/dataset-fallback.test.ts` | C | tbd | todo | Business layer shared services |
| `packages/business/__tests__/default-reply-throttle.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/business/__tests__/device-token-service.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/business/__tests__/dynamic-image-avatar-user.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/edge-safe-import-graph.test.ts` | C | tbd | todo | Builder app shell and shared components |
| `packages/business/__tests__/email-topic-click-url.test.ts` | C | tbd | todo | Email topics and tracking |
| `packages/business/__tests__/email-tracking.service.test.ts` | C | tbd | todo | Email topics and tracking |
| `packages/business/__tests__/entitlements.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/error-log.list.test.ts` | C | tbd | todo | Error logs |
| `packages/business/__tests__/error-log.service.test.ts` | C | tbd | todo | Error logs |
| `packages/business/__tests__/errors.test.ts` | C | tbd | todo | Error logs |
| `packages/business/__tests__/external-request-redirect.integration.test.ts` | C | tbd | todo | Outbound webhooks |
| `packages/business/__tests__/external-request-service.test.ts` | C | tbd | todo | Outbound webhooks |
| `packages/business/__tests__/external-webhook.service.test.ts` | C | tbd | todo | Outbound webhooks |
| `packages/business/__tests__/extract-contact.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/facebook-ads-cached-queries.test.ts` | C | tbd | todo | Facebook Ads integration |
| `packages/business/__tests__/facebook-lead-ads-lead-service.test.ts` | C | tbd | todo | Facebook Lead Ads automation |
| `packages/business/__tests__/facebook-marketing-message-audience-live-status.test.ts` | C | tbd | todo | Facebook Marketing Messages |
| `packages/business/__tests__/facebook-marketing-message-audience-service.test.ts` | C | tbd | todo | Facebook Marketing Messages |
| `packages/business/__tests__/facebook-marketing-message-content-payload.test.ts` | C | tbd | todo | Facebook Marketing Messages |
| `packages/business/__tests__/facebook-marketing-message-delivery-dispatch.test.ts` | C | tbd | todo | Facebook Marketing Messages |
| `packages/business/__tests__/facebook-marketing-message-delivery-lifecycle.test.ts` | C | tbd | todo | Facebook Marketing Messages |
| `packages/business/__tests__/facebook-marketing-message-delivery-webhook.test.ts` | C | tbd | todo | Facebook Marketing Messages |
| `packages/business/__tests__/facebook-marketing-messages-service.test.ts` | C | tbd | todo | Facebook Marketing Messages |
| `packages/business/__tests__/fail-open-circuit.test.ts` | C | tbd | todo | Business layer shared services |
| `packages/business/__tests__/flow-build-export.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/business/__tests__/flow-duplicate.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/business/__tests__/flow-filters.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/business/__tests__/flow-import-flow-export.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/business/__tests__/flow-publishability-enforcement.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/business/__tests__/flow-service-list.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/business/__tests__/flow-service-nodes.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/business/__tests__/flow-version-after-publish-if-live.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/business/__tests__/flow-version-service-publish.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/business/__tests__/flow-version.service.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/business/__tests__/flow.service.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/business/__tests__/folder-service-create.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/business/__tests__/google-ads-click-fields.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-click-lookup.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-consent.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-customer-matching.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-customer-properties.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-delivery.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-hash-user-data.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-housekeeping.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-legacy-delivery.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-list-events.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-login-account.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-owner-schema-service.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-processing-status.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-public-setup.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-record-conversion.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-send-queue.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-settings-service.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-stat-buckets.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-stats.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-timing.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/google-ads-transaction-id.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/has-workspace-access.test.ts` | C | tbd | todo | Workspaces |
| `packages/business/__tests__/human-reply-window-sla.test.ts` | C | tbd | todo | Working hours |
| `packages/business/__tests__/ice-candidate-summary.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/business/__tests__/ig-story-automation.list.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `packages/business/__tests__/ig-story-automation.write.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `packages/business/__tests__/import-column-map.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/business/__tests__/import-create-upload.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/business/__tests__/import-peek-headers.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/business/__tests__/import-start-contact-import.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/business/__tests__/import-start-product-statuses.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/business/__tests__/import.service.test.ts` | C | tbd | todo | Business layer shared services |
| `packages/business/__tests__/inbound-webhook-action-references-semantics.test.ts` | C | tbd | todo | Inbound webhooks |
| `packages/business/__tests__/inbound-webhook-action-references.test.ts` | C | tbd | todo | Inbound webhooks |
| `packages/business/__tests__/inbound-webhook-apply-mapped-fields.test.ts` | C | tbd | todo | Inbound webhooks |
| `packages/business/__tests__/inbound-webhook-apply-payload.test.ts` | C | tbd | todo | Inbound webhooks |
| `packages/business/__tests__/inbound-webhook-effective-channel.test.ts` | C | tbd | todo | Inbound webhooks |
| `packages/business/__tests__/inbound-webhook-extract.test.ts` | C | tbd | todo | Inbound webhooks |
| `packages/business/__tests__/inbound-webhook-resolve-contact.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/inbound-webhook-resolve-inbox.test.ts` | C | tbd | todo | Inbound webhooks |
| `packages/business/__tests__/inbound-webhook-service.test.ts` | C | tbd | todo | Inbound webhooks |
| `packages/business/__tests__/inbox-disconnect-ai-handover.test.ts` | C | tbd | todo | AI handover (human takeover) |
| `packages/business/__tests__/inbox-has-any-channel.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/inbox-profile-link.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/inbox-status-write-guard.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/inbox-team-service-members.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/inbox-update-mark-read-on-outbound.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/inbox-with-integrations-relations.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/input-attachments.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/integration-active-campaign.service.test.ts` | C | tbd | todo | ActiveCampaign integration |
| `packages/business/__tests__/integration-api.service.test.ts` | C | tbd | todo | API channel integration (custom channels) |
| `packages/business/__tests__/integration-facebook-ads.service.test.ts` | C | tbd | todo | Facebook Ads integration |
| `packages/business/__tests__/integration-google-ads.service.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/business/__tests__/integration-inbox-lookup.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/integration-instagram-service.test.ts` | C | tbd | todo | Instagram channel |
| `packages/business/__tests__/integration-klaviyo.service.test.ts` | C | tbd | todo | Klaviyo integration |
| `packages/business/__tests__/integration-llm-connect.service.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/business/__tests__/integration-mailer-lite.service.test.ts` | C | tbd | todo | MailerLite integration |
| `packages/business/__tests__/integration-messenger-handover-flow.service.test.ts` | C | tbd | todo | Thread control (handover protocol) |
| `packages/business/__tests__/integration-messenger-service.test.ts` | C | tbd | todo | Messenger channel |
| `packages/business/__tests__/integration-meta-catalog.service.test.ts` | C | tbd | todo | Meta Catalog |
| `packages/business/__tests__/integration-moosend.service.test.ts` | C | tbd | todo | Moosend integration |
| `packages/business/__tests__/integration-openai-compatible-base-url.test.ts` | C | tbd | todo | OpenAI-compatible integration |
| `packages/business/__tests__/integration-openai-compatible-service.test.ts` | C | tbd | todo | OpenAI-compatible integration |
| `packages/business/__tests__/integration-service.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/__tests__/integration-smtp-service.test.ts` | C | tbd | todo | SMTP email integration |
| `packages/business/__tests__/integration-tag-sync.service.test.ts` | C | tbd | todo | Tags and labels |
| `packages/business/__tests__/integration-telegram-service.test.ts` | C | tbd | todo | Telegram channel |
| `packages/business/__tests__/integration-threads.service.test.ts` | C | tbd | todo | Threads channel |
| `packages/business/__tests__/integration-tiktok-comment-to-message.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `packages/business/__tests__/integration-tiktok-service.test.ts` | C | tbd | todo | TikTok channel |
| `packages/business/__tests__/integration-webchat-branding.test.ts` | C | tbd | todo | Webchat channel |
| `packages/business/__tests__/integration-webchat-schema.test.ts` | C | tbd | todo | Webchat channel |
| `packages/business/__tests__/integration-webchat-service.test.ts` | C | tbd | todo | Webchat channel |
| `packages/business/__tests__/integration-whatsapp-call-settings.service.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/business/__tests__/integration-whatsapp-disconnect.service.test.ts` | C | tbd | todo | WhatsApp channel |
| `packages/business/__tests__/integration-whatsapp-handover-flow.service.test.ts` | C | tbd | todo | Thread control (handover protocol) |
| `packages/business/__tests__/integration-whatsapp-pix-redirect.service.test.ts` | C | tbd | todo | WhatsApp channel |
| `packages/business/__tests__/integration-whatsapp-routing-roles.service.test.ts` | C | tbd | todo | WhatsApp channel |
| `packages/business/__tests__/integration-whatsapp-signup-session.service.test.ts` | C | tbd | todo | WhatsApp channel |
| `packages/business/__tests__/integration-whatsapp-token-refresh.service.test.ts` | C | tbd | todo | WhatsApp channel |
| `packages/business/__tests__/integration-whatsapp-webhook-verified.service.test.ts` | C | tbd | todo | WhatsApp channel |
| `packages/business/__tests__/integration-zalo-service.test.ts` | C | tbd | todo | Zalo channel |
| `packages/business/__tests__/integration/bot-field-folder-tx-db.test.ts` | C | tbd | baseline-skip | Bot fields; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/broadcast-activation-race.test.ts` | C | tbd | baseline-skip | Broadcasts (incl. recurring); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/broadcast-filtered-audience-scan.test.ts` | C | tbd | baseline-skip | Broadcasts (incl. recurring); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/connection-store-bindings-insert.test.ts` | C | tbd | baseline-skip | Channel connection and OAuth connect flow; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/email-topic-delete-reference.test.ts` | C | tbd | baseline-skip | Email topics and tracking; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/human-reply-window-update.test.ts` | C | tbd | baseline-skip | Working hours; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/inbox-create-revive-quota.test.ts` | C | tbd | baseline-skip | Licensing, billing and quotas; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/inbox-disconnect-tenancy.test.ts` | C | tbd | baseline-skip | Inbox and live chat (conversations, messages); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/integration-threads-connect-reconnect.test.ts` | C | tbd | baseline-skip | Threads channel; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/pipeline-automation.test.ts` | C | tbd | baseline-skip | Pipelines (CRM deals); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/pipeline-service-hardening.test.ts` | C | tbd | baseline-skip | Pipelines (CRM deals); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/pipeline-service.test.ts` | C | tbd | baseline-skip | Pipelines (CRM deals); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/resource-deletion-log-channel-race.test.ts` | C | tbd | baseline-skip | Workspaces; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/resource-deletion-log-workspace-race.test.ts` | C | tbd | baseline-skip | Workspaces; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/response-sla-lifecycle.test.ts` | C | tbd | baseline-skip | Response SLA; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-bot-field-collision-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-bot-field-token-order-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-concurrency-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-install-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-install-roots-uninstall-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-lease-fencing-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-lifecycle-stamps-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-list-page-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-locked-save-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-mutation-matrix-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-reference-safety-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-selectable-resources-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-sync-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-sync-reference-safety-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-uninstall-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-uninstall-effects-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-uninstall-engine-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/template-uninstall-lifecycle-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/tenant-default-template-db.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/user-quota-channel-count.test.ts` | C | tbd | baseline-skip | Licensing, billing and quotas; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/integration/workspace-lifecycle-quota-release-deferral.test.ts` | C | tbd | baseline-skip | Licensing, billing and quotas; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/business/__tests__/invitation.service.test.ts` | C | tbd | todo | Workspace members, roles and invitations |
| `packages/business/__tests__/is-super-admin.test.ts` | C | tbd | todo | Workspace members, roles and invitations |
| `packages/business/__tests__/javascript-execution-output-value.test.ts` | - | - | dropped | Execute JavaScript flow step; feature dropped (DECISIONS 2026-11-11) |
| `packages/business/__tests__/javascript-execution.test.ts` | - | - | dropped | Execute JavaScript flow step; feature dropped (DECISIONS 2026-11-11) |
| `packages/business/__tests__/javascript-output-mapping.test.ts` | - | - | dropped | Execute JavaScript flow step; feature dropped (DECISIONS 2026-11-11) |
| `packages/business/__tests__/last-user-input.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/business/__tests__/leaderboard-email-telegram-sources.test.ts` | C | tbd | todo | Leaderboards |
| `packages/business/__tests__/leaderboard-helpers.test.ts` | C | tbd | todo | Leaderboards |
| `packages/business/__tests__/leaderboard-participants-security.test.ts` | C | tbd | todo | Leaderboards |
| `packages/business/__tests__/leaderboard-partitions.test.ts` | C | tbd | todo | Leaderboards |
| `packages/business/__tests__/leaderboard-rewards.test.ts` | C | tbd | todo | Leaderboards |
| `packages/business/__tests__/leaderboard-scoring.test.ts` | C | tbd | todo | Leaderboards |
| `packages/business/__tests__/leaderboard-welcome-flow.test.ts` | C | tbd | todo | Leaderboards |
| `packages/business/__tests__/license.refresh.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/license.service.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/license.startup.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/live-counter-store-consume.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/live-counter-store.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/magic-link.service.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/business/__tests__/map-stats-contact-row.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/mcp-oauth.service.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/__tests__/media-library-file-service.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/business/__tests__/media-library-read-image.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/business/__tests__/media-library.service.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/business/__tests__/message-create-outgoing.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/message-list-for-conversation.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/message-record-delivered-private-reply.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `packages/business/__tests__/message-service.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/message-status-branch-service.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/messaging-ads-connection-revoke-disconnect.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads) |
| `packages/business/__tests__/messenger-integration-clone-targets.test.ts` | C | tbd | todo | Messenger channel |
| `packages/business/__tests__/messenger-message-template-list.test.ts` | C | tbd | todo | Messenger channel |
| `packages/business/__tests__/messenger-welcome-message-flow.service.test.ts` | C | tbd | todo | Messenger channel |
| `packages/business/__tests__/meta-catalog-import-service.test.ts` | C | tbd | todo | Meta Catalog |
| `packages/business/__tests__/meta-catalog-operation-service.test.ts` | C | tbd | todo | Meta Catalog |
| `packages/business/__tests__/meta-catalog-sync-run-service.test.ts` | C | tbd | todo | Meta Catalog |
| `packages/business/__tests__/meta-conversions-channel-identity.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `packages/business/__tests__/meta-conversions-channel-policy.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `packages/business/__tests__/meta-conversions-connection.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `packages/business/__tests__/meta-conversions-service.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `packages/business/__tests__/meta-sibling-integration.service.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `packages/business/__tests__/mini-app-definition.test.ts` | C | tbd | todo | Mini-apps |
| `packages/business/__tests__/mini-app-duplicate.test.ts` | C | tbd | todo | Mini-apps |
| `packages/business/__tests__/mini-app-submission-custom-fields.test.ts` | C | tbd | todo | Mini-apps |
| `packages/business/__tests__/mini-app-upload-service.test.ts` | C | tbd | todo | Mini-apps |
| `packages/business/__tests__/minigame-contact-service-scoping.test.ts` | C | tbd | todo | Minigames |
| `packages/business/__tests__/minigame-leaderboard-award.test.ts` | C | tbd | todo | Minigames |
| `packages/business/__tests__/minigame-outcome-message-text.test.ts` | C | tbd | todo | Minigames |
| `packages/business/__tests__/minigame-referral-bonus.test.ts` | C | tbd | todo | Minigames |
| `packages/business/__tests__/minigame-service-update.test.ts` | C | tbd | todo | Minigames |
| `packages/business/__tests__/mobile-refresh-token-service.test.ts` | C | tbd | todo | Authentication (sign-in, social, mobile, magic links) |
| `packages/business/__tests__/net/ssrf-guard.test.ts` | C | tbd | todo | Business layer shared services |
| `packages/business/__tests__/outbound-media.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/business/__tests__/pipeline-access.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/pipeline-assignees.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/pipeline-currency-import-order.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/pipeline-currency-refresh-import-order.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/pipeline-currency-refresh.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/pipeline-currency.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/pipeline-flow-actions.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/pipeline-flow-publish-check.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/pipeline-item-assignees.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/pipeline-item-base.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/pipeline-mutation-engine.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/pipeline-naming.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/pipeline-update-lock.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/pipeline-variable-targets-light.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/platform-admin-bootstrap.test.ts` | C | tbd | todo | Platform admin |
| `packages/business/__tests__/platform-admin.service.test.ts` | C | tbd | todo | Platform admin |
| `packages/business/__tests__/platform-credential-service.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/__tests__/poll-analytics.test.ts` | C | tbd | todo | Polls |
| `packages/business/__tests__/poll-chat-messages.test.ts` | C | tbd | todo | Polls |
| `packages/business/__tests__/poll-service.test.ts` | C | tbd | todo | Polls |
| `packages/business/__tests__/poll-state.test.ts` | C | tbd | todo | Polls |
| `packages/business/__tests__/poll-vote.test.ts` | C | tbd | todo | Polls |
| `packages/business/__tests__/product-category-service.test.ts` | C | tbd | todo | Ecommerce: products, categories, vendors |
| `packages/business/__tests__/product-message-resolve.test.ts` | C | tbd | todo | Ecommerce: products, categories, vendors |
| `packages/business/__tests__/product-message-service.test.ts` | C | tbd | todo | Ecommerce: products, categories, vendors |
| `packages/business/__tests__/product-service.test.ts` | C | tbd | todo | Ecommerce: products, categories, vendors |
| `packages/business/__tests__/profile-fields.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/business/__tests__/qr-code.service.test.ts` | C | tbd | todo | Public links (open-link, QR codes, reflinks) |
| `packages/business/__tests__/questionnaire-service.test.ts` | C | tbd | todo | Questionnaires |
| `packages/business/__tests__/quota-enforcement-env.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/quota-enforcement-mac-admission.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/quota-enforcement.service.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/realtime-broadcast.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `packages/business/__tests__/realtime-redis-url.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `packages/business/__tests__/realtime-relay-metrics.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `packages/business/__tests__/realtime-stream-publisher.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `packages/business/__tests__/referral-ref-encoding.test.ts` | C | tbd | todo | Leaderboards |
| `packages/business/__tests__/referral-utils-poll.test.ts` | C | tbd | todo | Leaderboards |
| `packages/business/__tests__/reflink-service.test.ts` | C | tbd | todo | Leaderboards |
| `packages/business/__tests__/reply-format-validators.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/resolve-minigame-prize.test.ts` | C | tbd | todo | Minigames |
| `packages/business/__tests__/resolve-sent-whatsapp-flow.test.ts` | C | tbd | todo | WhatsApp Flows (forms) |
| `packages/business/__tests__/resolve-workspace-access.test.ts` | C | tbd | todo | Workspaces |
| `packages/business/__tests__/response-sla-service.test.ts` | C | tbd | todo | Response SLA |
| `packages/business/__tests__/saved-reply-service.test.ts` | C | tbd | todo | Saved replies |
| `packages/business/__tests__/select-labels-by-ids.test.ts` | C | tbd | todo | Tags and labels |
| `packages/business/__tests__/self-serve-config-fields.test.ts` | C | tbd | todo | Instagram channel |
| `packages/business/__tests__/sequence-service-list-step-contacts.test.ts` | C | tbd | todo | Sequences |
| `packages/business/__tests__/sequence-service.test.ts` | C | tbd | todo | Sequences |
| `packages/business/__tests__/sequence-step-payload.test.ts` | C | tbd | todo | Sequences |
| `packages/business/__tests__/smart-delay-service.test.ts` | C | tbd | todo | Smart delay |
| `packages/business/__tests__/spreadsheet.service.test.ts` | C | tbd | todo | Google Sheets integration |
| `packages/business/__tests__/support-consent.test.ts` | C | tbd | todo | Workspace support access |
| `packages/business/__tests__/system-field-service.test.ts` | C | tbd | todo | System fields |
| `packages/business/__tests__/tag-service-attach-detach-replace.test.ts` | C | tbd | todo | Tags and labels |
| `packages/business/__tests__/tag-service-bulk-attach.test.ts` | C | tbd | todo | Tags and labels |
| `packages/business/__tests__/tag-service-create.test.ts` | C | tbd | todo | Tags and labels |
| `packages/business/__tests__/tag-service-detach-from-contact.test.ts` | C | tbd | todo | Tags and labels |
| `packages/business/__tests__/tag-service-ensure-tag-channel.test.ts` | C | tbd | todo | Tags and labels |
| `packages/business/__tests__/tag-service-soft-delete.test.ts` | C | tbd | todo | Tags and labels |
| `packages/business/__tests__/tag-service-sync-helpers.test.ts` | C | tbd | todo | Tags and labels |
| `packages/business/__tests__/tag-sync-service-writes.test.ts` | C | tbd | todo | Tags and labels |
| `packages/business/__tests__/tag-sync.service.test.ts` | C | tbd | todo | Tags and labels |
| `packages/business/__tests__/template-adapters-pipelines.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/template-ai-agents-mcp-dependency.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-ai-file-dispatch.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-ai-file-paths.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-ai-files-adapter.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-ai-mcp-servers-adapter.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-calendar-install-name.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-category-counts-share.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-collect-pipelines-conditions.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/template-collect-pipelines.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/template-column-slot-guards.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-comment-automations-adapter.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `packages/business/__tests__/template-contact-filter-guard.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-coupon-token-rewrite.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-custom-field-concurrent-suffix.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-custom-field-manifest.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-default-template-status.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-field-token-binder.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-fingerprint.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-flow-ref-adapters.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-flow-reference-dependencies.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-flows-adapter.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-flows-backup.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-folder-manifest.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-format-compat.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-inbox-manifest.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-install-copied-objects.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-install-folder-placement.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-install-intent.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-install-locks.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-install-order.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-install-preview.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-install-regressions.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-install-root-placement.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-installability.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-installation-dispatch.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-installation-request.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-installation-stage.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-installation-visibility.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-installed-resource.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-installer-actor.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-item-permissions.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-keywords-adapter.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-list-page.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-manifest-references.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-naming.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-object-keys.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-params-service.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-payload-hash.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-permissions.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-preview-selection-dependencies.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-publisher-installations.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-questionnaires-coupon-topics-adapter.test.ts` | C | tbd | todo | Questionnaires |
| `packages/business/__tests__/template-reference-sanitizer.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-resolve-manifests.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-resource-locks.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-selectable-cursor.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-selection-without-deleted.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-sequences-adapter.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-service-authorization.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-settings-adapter.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-settings-bot-field-batch.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-share-token.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-share-url.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-snapshot-dependency-chain.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-snapshot-dropped-references.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-snapshot-dropped.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-snapshot-manifest-references.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-snapshot-view-only.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-source-order.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-stage-shared-install.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-staged-cleanup.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-sync-deleted-copies.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-sync-execute.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-sync-field-bindings.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-sync-handler-coverage.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-sync-handlers-wave2.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-sync-hash.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-sync-pipeline-mapping.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/business/__tests__/template-sync-request.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-trigger-conditions.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-trigger-field-keys.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-triggers-adapter.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-triggers-remap-warnings.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-uninstall-decide.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-uninstall-effect-runners.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-uninstall-execute.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-uninstall-handlers.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-uninstall-references.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-uninstall-request.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-version-changes.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/template-warning-groups.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/tenant-channel-visibility.test.ts` | C | tbd | todo | Multi-tenancy (white-label tenants) |
| `packages/business/__tests__/tenant-default-template-service.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/business/__tests__/tenant-help-item-service.test.ts` | C | tbd | todo | Multi-tenancy (white-label tenants) |
| `packages/business/__tests__/tenant-lifecycle.test.ts` | C | tbd | todo | Multi-tenancy (white-label tenants) |
| `packages/business/__tests__/tenant-reconcile.test.ts` | C | tbd | todo | Scheduler and scheduled jobs |
| `packages/business/__tests__/tenant-settings-domain.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/tenant-settings-entitlement.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/thread-control-constants.test.ts` | C | tbd | todo | Thread control (handover protocol) |
| `packages/business/__tests__/thread-control-service.test.ts` | C | tbd | todo | Thread control (handover protocol) |
| `packages/business/__tests__/tiktok-post-details.test.ts` | C | tbd | todo | TikTok channel |
| `packages/business/__tests__/trigger-service-executions.test.ts` | C | tbd | todo | Triggers |
| `packages/business/__tests__/trigger-service-update-settings.test.ts` | C | tbd | todo | Triggers |
| `packages/business/__tests__/trigger-service-update-with-conditions.test.ts` | C | tbd | todo | Triggers |
| `packages/business/__tests__/trigger.service.test.ts` | C | tbd | todo | Triggers |
| `packages/business/__tests__/update-from-message.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/__tests__/user-persistent-menu-service.test.ts` | C | tbd | todo | Messenger channel |
| `packages/business/__tests__/user-quota-access-state.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/user-quota-bootstrap-plan.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/user-quota-default-plan.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/user-quota-due-expired-trials.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/user-quota-plan-identity.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/user-quota-reconcile-self.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/webhook-service-builder-methods.test.ts` | C | tbd | todo | Outbound webhooks |
| `packages/business/__tests__/webhook-service-update-with-conditions.test.ts` | C | tbd | todo | Outbound webhooks |
| `packages/business/__tests__/webhook.service.test.ts` | C | tbd | todo | Outbound webhooks |
| `packages/business/__tests__/whatsapp-business-account.service.test.ts` | C | tbd | todo | WhatsApp channel |
| `packages/business/__tests__/whatsapp-call-access-service.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/business/__tests__/whatsapp-call-assignment-triggers.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/business/__tests__/whatsapp-call-history-service.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/business/__tests__/whatsapp-call-lifecycle-service.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/business/__tests__/whatsapp-call-permission-service.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/business/__tests__/whatsapp-call-recording-service.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/business/__tests__/whatsapp-call-ring-targets.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/business/__tests__/whatsapp-call-summary-service.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/business/__tests__/whatsapp-call-voip-call-service.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/business/__tests__/whatsapp-flow-response-service.test.ts` | C | tbd | todo | WhatsApp Flows (forms) |
| `packages/business/__tests__/whatsapp-integration.service.connect-phone.test.ts` | C | tbd | todo | WhatsApp channel |
| `packages/business/__tests__/whatsapp-integration.service.set-coexist.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `packages/business/__tests__/whatsapp-message-template-list.test.ts` | C | tbd | todo | WhatsApp message templates |
| `packages/business/__tests__/whatsapp-voip-sdp.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/business/__tests__/with-blocked-owner-guard.test.ts` | C | tbd | todo | Business layer shared services |
| `packages/business/__tests__/working-hours-date-overrides.test.ts` | C | tbd | todo | Working hours |
| `packages/business/__tests__/working-hours-duration.test.ts` | C | tbd | todo | Working hours |
| `packages/business/__tests__/working-hours-measure.test.ts` | C | tbd | todo | Working hours |
| `packages/business/__tests__/working-hours-on-shift.test.ts` | C | tbd | todo | Working hours |
| `packages/business/__tests__/working-hours-service.test.ts` | C | tbd | todo | Working hours |
| `packages/business/__tests__/working-hours-timezone.test.ts` | C | tbd | todo | Working hours |
| `packages/business/__tests__/workspace-api-token.service.test.ts` | C | tbd | todo | Workspace API tokens and public API |
| `packages/business/__tests__/workspace-bot-message-usage.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/workspace-deletion-record.test.ts` | C | tbd | todo | Workspaces |
| `packages/business/__tests__/workspace-deletion-schedule.test.ts` | C | tbd | todo | Workspaces |
| `packages/business/__tests__/workspace-ensure-first-for-owner.test.ts` | C | tbd | todo | Workspaces |
| `packages/business/__tests__/workspace-lifecycle.channel-switch-exhaustive.test.ts` | C | tbd | todo | Workspaces |
| `packages/business/__tests__/workspace-lifecycle.predicates.test.ts` | C | tbd | todo | Workspaces |
| `packages/business/__tests__/workspace-lifecycle.service.test.ts` | C | tbd | todo | Workspaces |
| `packages/business/__tests__/workspace-lifecycle.smart-delay-cleanup.test.ts` | C | tbd | todo | Smart delay |
| `packages/business/__tests__/workspace-member-permissions.test.ts` | C | tbd | todo | Workspace members, roles and invitations |
| `packages/business/__tests__/workspace-member-predicates.test.ts` | C | tbd | todo | Workspace members, roles and invitations |
| `packages/business/__tests__/workspace-member-synthetic.test.ts` | C | tbd | todo | Workspace members, roles and invitations |
| `packages/business/__tests__/workspace-member.service.test.ts` | C | tbd | todo | Workspace members, roles and invitations |
| `packages/business/__tests__/workspace-member.update.test.ts` | C | tbd | todo | Workspace members, roles and invitations |
| `packages/business/__tests__/workspace-presence-service.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `packages/business/__tests__/workspace-support-access.service.test.ts` | C | tbd | todo | Workspace support access |
| `packages/business/__tests__/workspace-usage.service.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/business/__tests__/workspace.service.test.ts` | C | tbd | todo | Workspaces |
| `packages/business/src/__tests__/errors.test.ts` | C | tbd | todo | Error logs |
| `packages/business/src/broadcast/__tests__/broadcast-targets.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/src/broadcast/__tests__/broadcast.service.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/business/src/connect-session/__tests__/service.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/src/connection/__tests__/credential-providers.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/src/connection/__tests__/provider-account-event-service.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/src/connection/__tests__/provider-account-events.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/src/connection/__tests__/record-refreshed-auth.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/src/connection/__tests__/state-service.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/src/connection/__tests__/state.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/src/connection/__tests__/store-bindings.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/src/connection/__tests__/upsert.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/src/conversation/__tests__/conversation.service.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/src/decision/adapters.test.ts` | C | tbd | todo | Decision engine (routing profiles) |
| `packages/business/src/decision/connection-service.test.ts` | C | tbd | todo | Decision engine (routing profiles) |
| `packages/business/src/decision/contracts.test.ts` | C | tbd | todo | Decision engine (routing profiles) |
| `packages/business/src/decision/global-router-service.test.ts` | C | tbd | todo | Decision engine (routing profiles) |
| `packages/business/src/decision/profile-service.test.ts` | C | tbd | todo | Decision engine (routing profiles) |
| `packages/business/src/decision/transport.test.ts` | C | tbd | todo | Decision engine (routing profiles) |
| `packages/business/src/discount-code/__tests__/discount-code-service.test.ts` | C | tbd | todo | Coupons and discount codes |
| `packages/business/src/discount-code/__tests__/pricing.test.ts` | C | tbd | todo | Coupons and discount codes |
| `packages/business/src/ecommerce-analytics/__tests__/ecommerce-analytics-service.test.ts` | C | tbd | todo | Ecommerce: settings, open hours, analytics |
| `packages/business/src/ecommerce-open-hours/__tests__/ecommerce-open-hours-service.test.ts` | C | tbd | todo | Ecommerce: settings, open hours, analytics |
| `packages/business/src/ecommerce-open-hours/__tests__/is-open.test.ts` | C | tbd | todo | Ecommerce: settings, open hours, analytics |
| `packages/business/src/ecommerce-settings/__tests__/ecommerce-settings-service.test.ts` | C | tbd | todo | Ecommerce: settings, open hours, analytics |
| `packages/business/src/enterprise/tenant/__tests__/service.test.ts` | C | tbd | todo | Multi-tenancy (white-label tenants) |
| `packages/business/src/inbox/__tests__/list-analytics-filter-options.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/src/inbox/__tests__/list-channel-options.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/src/inbox/__tests__/resolve-broadcast-inbox-ids.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/src/inbox/__tests__/service.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/business/src/integration-ai-provider/__tests__/verify.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/business/src/integration-context/__tests__/auth-store.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/src/integration/__tests__/find-token-refresh-errors.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/business/src/media/__tests__/no-avatar-sentinel.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/business/src/media/__tests__/optimize-stored-image.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/business/src/media/__tests__/resolve-media-url.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/business/src/messaging-ads-connection/__tests__/context.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads) |
| `packages/business/src/messaging-ads-connection/__tests__/graph-cache.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads) |
| `packages/business/src/messaging-ads-connection/__tests__/graph-reads.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads) |
| `packages/business/src/messaging-ads-connection/__tests__/service.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads) |
| `packages/business/src/messaging-ads/__tests__/mappers.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads) |
| `packages/business/src/messaging-ads/__tests__/media-preflight.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads) |
| `packages/business/src/messaging-ads/__tests__/resolve-channel-assets.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads) |
| `packages/business/src/messaging-ads/__tests__/service.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads) |
| `packages/business/src/meta-conversions/__tests__/event-input.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `packages/business/src/meta-conversions/__tests__/hash-user-data.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `packages/business/src/order-payment/__tests__/order-payment-service.test.ts` | C | tbd | todo | Ecommerce: orders and payment providers |
| `packages/business/src/order-payment/__tests__/settle.test.ts` | C | tbd | todo | Ecommerce: orders and payment providers |
| `packages/business/src/order-payment/__tests__/webhooks.test.ts` | C | tbd | todo | Ecommerce: orders and payment providers |
| `packages/business/src/order/__tests__/order-service.test.ts` | C | tbd | todo | Ecommerce: orders and payment providers |
| `packages/business/src/order/__tests__/order-sync-external.test.ts` | C | tbd | todo | Ecommerce: orders and payment providers |
| `packages/business/src/payment-provider/__tests__/payment-provider-service.test.ts` | C | tbd | todo | Ecommerce: orders and payment providers |
| `packages/business/src/payment-provider/__tests__/platform-stripe.test.ts` | C | tbd | todo | Ecommerce: orders and payment providers |
| `packages/business/src/place-near-me/__tests__/category-service.test.ts` | C | tbd | todo | Places near me |
| `packages/business/src/place-near-me/__tests__/contact-location.test.ts` | C | tbd | todo | Places near me |
| `packages/business/src/place-near-me/__tests__/location-service.test.ts` | C | tbd | todo | Places near me |
| `packages/business/src/place-near-me/__tests__/nearest.test.ts` | C | tbd | todo | Places near me |
| `packages/business/src/place-near-me/__tests__/preview-url.test.ts` | C | tbd | todo | Places near me |
| `packages/business/src/resource-deletion-log/__tests__/service.test.ts` | C | tbd | todo | Workspaces |
| `packages/business/src/shopify/__tests__/mappers.test.ts` | C | tbd | todo | Shopify integration |
| `packages/business/src/shopify/__tests__/sync-currency.test.ts` | C | tbd | todo | Shopify integration |
| `packages/business/src/shopify/__tests__/webhooks.test.ts` | C | tbd | todo | Shopify integration |
| `packages/business/src/vendor/__tests__/vendor-service.test.ts` | C | tbd | todo | Ecommerce: products, categories, vendors |
| `packages/business/src/webshop/__tests__/cart-lines.test.ts` | C | tbd | todo | Ecommerce: webshop storefront |
| `packages/business/src/webshop/__tests__/collection.test.ts` | C | tbd | todo | Ecommerce: webshop storefront |
| `packages/business/src/webshop/__tests__/link.test.ts` | C | tbd | todo | Ecommerce: webshop storefront |
| `packages/business/src/webshop/__tests__/product-search.test.ts` | C | tbd | todo | Ecommerce: webshop storefront |
| `packages/business/src/webshop/__tests__/shop-categories.test.ts` | C | tbd | todo | Ecommerce: webshop storefront |
| `packages/business/src/webshop/__tests__/webshop-service.test.ts` | C | tbd | todo | Ecommerce: webshop storefront |
| `packages/business/src/woocommerce/__tests__/mappers.test.ts` | C | tbd | todo | WooCommerce integration |
| `packages/business/src/woocommerce/__tests__/sync.service.test.ts` | C | tbd | todo | WooCommerce integration |
| `packages/business/src/woocommerce/__tests__/webhooks.test.ts` | C | tbd | todo | WooCommerce integration |
| `packages/business/src/workspace-lifecycle/__tests__/purge-batch.test.ts` | C | tbd | todo | Workspaces |
| `packages/business/src/workspace-lifecycle/__tests__/service.test.ts` | C | tbd | todo | Workspaces |
| `packages/business/src/workspace/__tests__/service.test.ts` | C | tbd | todo | Workspaces |
| `packages/channel-registry/__tests__/media-hydration.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/channel-registry/__tests__/registry.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/channel-registry/__tests__/thread-control.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/connections/__tests__/connect-session-start.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/connections/__tests__/connect-targets-tamper.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/connections/__tests__/integration/connect-session-flow.test.ts` | C | tbd | baseline-skip | Channel connection and OAuth connect flow; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/connections/__tests__/integration/connection-engine.test.ts` | C | tbd | baseline-skip | Channel connection and OAuth connect flow; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/connections/__tests__/internal.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/connections/__tests__/lifecycle.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/connections/__tests__/messenger-teardown.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/connections/__tests__/registry-self-serve.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/connections/__tests__/registry.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/connections/__tests__/self-serve.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/connections/__tests__/service.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/database/__tests__/ads-conversion-event-repository.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `packages/database/__tests__/ai-agent-actions.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/database/__tests__/ai-conversation-embedding-repository.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files) |
| `packages/database/__tests__/ai-file-chunking-config.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files) |
| `packages/database/__tests__/ai-file-embedding-id.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files) |
| `packages/database/__tests__/ai-file-embedding-repository.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files) |
| `packages/database/__tests__/ai-handover-bulk-eligibility.test.ts` | C | tbd | todo | AI handover (human takeover) |
| `packages/database/__tests__/ai-handover-bulk-run-repository.test.ts` | C | tbd | todo | AI handover (human takeover) |
| `packages/database/__tests__/ai-handover-settings-repository.test.ts` | C | tbd | todo | AI handover (human takeover) |
| `packages/database/__tests__/ai-workspace-scope-repository.test.ts` | C | tbd | todo | AI knowledge base (RAG, AI files) |
| `packages/database/__tests__/appointment-calendar-repository.test.ts` | C | tbd | todo | Appointments and booking |
| `packages/database/__tests__/appointment-partial.test.ts` | C | tbd | todo | Appointments and booking |
| `packages/database/__tests__/automation-throttle-repository.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/database/__tests__/backfill-coexist-id.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `packages/database/__tests__/backfill-contact-event-attributes.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/database/__tests__/better-auth-1-7-migration.test.ts` | C | `crates/db/tests/better_auth_1_7_migration.rs` | passing | Authentication (sign-in, social, mobile, magic links) |
| `packages/database/__tests__/broadcast-partial.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/database/__tests__/broadcast-plan-policy.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/database/__tests__/broadcast-purge.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/database/__tests__/broadcast-recurrence-repository.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/database/__tests__/broadcast-repository.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/database/__tests__/channel-partial.test.ts` | C | tbd | todo | Database (Drizzle, schema, repositories, sharding) |
| `packages/database/__tests__/client-errors.test.ts` | C | `crates/db/tests/client_errors.rs` | passing | Error logs |
| `packages/database/__tests__/coexist-channel-enum.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `packages/database/__tests__/coexist-run-recovery-repository.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `packages/database/__tests__/coexist-sync-run-repository.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `packages/database/__tests__/coexist-sync-run-type-scoping.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `packages/database/__tests__/comment-automation-event-purge.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `packages/database/__tests__/connect-session-cancel-pending.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/database/__tests__/connect-session-repository.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/database/__tests__/connection-manager.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/database/__tests__/contact-activity-filter.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/database/__tests__/contact-activity-partial.test.ts` | C | `crates/db/tests/contact_activity_partial.rs` | passing | Contacts (profiles, notes, activity) |
| `packages/database/__tests__/contact-export-page-repository.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/database/__tests__/contact-filter-commerce.test.ts` | C | tbd | todo | Contact filters and segments |
| `packages/database/__tests__/contact-filter-pipeline-item.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/database/__tests__/contact-filter-reference-coverage.test.ts` | C | tbd | todo | Contact filters and segments |
| `packages/database/__tests__/contact-filter.test.ts` | C | tbd | todo | Contact filters and segments |
| `packages/database/__tests__/contact-inbox-fk-indexes.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/database/__tests__/contact-inbox-repository.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/database/__tests__/contact-inbox-source-identity-history.test.ts` | C | `crates/db/tests/contact_inbox_source_identity_history.rs` | passing | Contacts (profiles, notes, activity) |
| `packages/database/__tests__/contact-inbox-source-parent-user-id.test.ts` | C | `crates/db/tests/contact_inbox_source_parent_user_id.rs` | passing | Contacts (profiles, notes, activity) |
| `packages/database/__tests__/contact-inbox-thread-owner-app-columns.test.ts` | C | `crates/db/tests/contact_inbox_thread_owner_app_columns.rs` | passing | Contacts (profiles, notes, activity) |
| `packages/database/__tests__/contact-on-broadcast-index.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/database/__tests__/contact-partial.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/database/__tests__/contact-referral-event-repository.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/database/__tests__/contact-repository-backdate-created-at.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/database/__tests__/contact-repository-list-where.test.ts` | C | tbd | todo | Contacts (profiles, notes, activity) |
| `packages/database/__tests__/conversation-ai-context-repository.test.ts` | C | tbd | todo | AI agents (tools, MCP client, functions) |
| `packages/database/__tests__/conversation-repository.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/database/__tests__/conversation-unread.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/database/__tests__/credential-aad.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/database/__tests__/credential.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/database/__tests__/ctwa-retarget-predicate.test.ts` | C | tbd | todo | Ads analytics and CTWA tracking |
| `packages/database/__tests__/date-bucket.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/database/__tests__/facebook-marketing-message-audience-schema.test.ts` | C | tbd | todo | Facebook Marketing Messages |
| `packages/database/__tests__/facebook-marketing-message-recipient-schema.test.ts` | C | tbd | todo | Facebook Marketing Messages |
| `packages/database/__tests__/facebook-marketing-message-repository.test.ts` | C | tbd | todo | Facebook Marketing Messages |
| `packages/database/__tests__/facebook-marketing-messages-schema.test.ts` | C | tbd | todo | Facebook Marketing Messages |
| `packages/database/__tests__/fb-comment-reply-texts.test.ts` | C | tbd | todo | Comment automation (FB/IG/Threads/TikTok) |
| `packages/database/__tests__/file-export-repository.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/database/__tests__/file-repository.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/database/__tests__/flow-repository.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/database/__tests__/get-safe-since-time.test.ts` | C | tbd | todo | Database (Drizzle, schema, repositories, sharding) |
| `packages/database/__tests__/google-ads-conversion-event-repository.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/database/__tests__/google-ads-settings-partial.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/database/__tests__/google-ads-settings-repository.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/database/__tests__/google-click-query.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/database/__tests__/google-click-referral-merge.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/database/__tests__/inbound-webhook-partial.test.ts` | C | tbd | todo | Inbound webhooks |
| `packages/database/__tests__/inbound-webhook-repository.test.ts` | C | tbd | todo | Inbound webhooks |
| `packages/database/__tests__/inbox-repository.test.ts` | C | `crates/db/tests/inbox_repository.rs` | passing | Inbox and live chat (conversations, messages) |
| `packages/database/__tests__/inbox-team-member-repository.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/database/__tests__/integration-google-ads-repository.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/database/__tests__/integration-messenger-repository-personas.test.ts` | C | tbd | todo | Messenger channel |
| `packages/database/__tests__/integration-whatsapp-mark-token-refresh-error.test.ts` | C | tbd | todo | WhatsApp channel |
| `packages/database/__tests__/integration-whatsapp-partial.test.ts` | C | tbd | todo | WhatsApp channel |
| `packages/database/__tests__/integration/ai-file-processing-run.test.ts` | C | tbd | baseline-skip | AI knowledge base (RAG, AI files); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/ai-handover-bulk-eligibility.test.ts` | C | tbd | baseline-skip | AI handover (human takeover); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/ai-handover-bulk-run-lifecycle.test.ts` | C | tbd | baseline-skip | AI handover (human takeover); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/backfill-connections.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/database/__tests__/integration/backfill-email-topic-reference.test.ts` | C | tbd | baseline-skip | Email topics and tracking; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/better-auth-1-7-migration.test.ts` | C | - | not-applicable | Authentication (sign-in, social, mobile, magic links); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/broadcast-purge-draft.test.ts` | C | tbd | baseline-skip | Broadcasts (incl. recurring); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/broadcast-recurrence-constraints.test.ts` | C | tbd | baseline-skip | Broadcasts (incl. recurring); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/connect-session-isolation.test.ts` | C | tbd | baseline-skip | Channel connection and OAuth connect flow; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/connection-repository.test.ts` | C | tbd | baseline-skip | Channel connection and OAuth connect flow; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/contact-activity-catalog.test.ts` | C | `crates/db/tests/contact_activity_catalog.rs` | passing | Contacts (profiles, notes, activity); P0.3: needs Postgres/Redis, not run — run in P0.7; some cases not-applicable (P2.3 user decision) |
| `packages/database/__tests__/integration/contact-activity-filter.test.ts` | C | tbd | baseline-skip | Contacts (profiles, notes, activity); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/contact-activity-repository.test.ts` | C | tbd | baseline-skip | Contacts (profiles, notes, activity); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/contact-custom-field-pair-lock.test.ts` | C | tbd | baseline-skip | Custom fields; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/contact-inbox-post.test.ts` | C | `crates/db/tests/contact_inbox_post.rs` | passing | Contacts (profiles, notes, activity); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/contact-inbox-source-identity-history.test.ts` | C | tbd | baseline-skip | Contacts (profiles, notes, activity); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/conversation-close-human-reply-window.test.ts` | C | tbd | baseline-skip | Working hours; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/conversation-insert-dm.test.ts` | C | tbd | baseline-skip | Inbox and live chat (conversations, messages); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/count-query-capped.test.ts` | C | tbd | baseline-skip | Contact filters and segments; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/flow-current-version-nodes.test.ts` | C | tbd | baseline-skip | Flow builder and execution; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/google-ads-conversion-event-fk.test.ts` | C | tbd | baseline-skip | Google Ads integration; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/google-ads-conversion-event-stats.test.ts` | C | tbd | baseline-skip | Google Ads integration; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/google-ads-conversion-event.test.ts` | C | tbd | baseline-skip | Google Ads integration; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/google-click-referral-merge.test.ts` | C | tbd | baseline-skip | Google Ads integration; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/inbox-repository.test.ts` | C | tbd | baseline-skip | Inbox and live chat (conversations, messages); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/insert-required-columns.test.ts` | C | `crates/db/tests/insert_required_columns.rs` | passing | Database (Drizzle, schema, repositories, sharding); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/integration-whatsapp-lifecycle-targets.test.ts` | C | tbd | baseline-skip | WhatsApp channel; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/meta-catalog-item-search.test.ts` | C | tbd | baseline-skip | Meta Catalog; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/mobile-refresh-token-repository.test.ts` | C | tbd | baseline-skip | Authentication (sign-in, social, mobile, magic links); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/pipeline-activity-filter.test.ts` | C | tbd | baseline-skip | Pipelines (CRM deals); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/pipeline-field-filter.test.ts` | C | tbd | baseline-skip | Pipelines (CRM deals); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/pipeline-item-contact-filter.test.ts` | C | tbd | baseline-skip | Pipelines (CRM deals); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/pipeline-repository.test.ts` | C | tbd | baseline-skip | Pipelines (CRM deals); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/poll-vote-upsert.test.ts` | C | tbd | baseline-skip | Polls; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/product-import-identifiers.test.ts` | C | tbd | baseline-skip | Imports and exports (contacts, products, flows); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/resource-deletion-log-repository.test.ts` | C | tbd | baseline-skip | Workspaces; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/schema-default-parity.test.ts` | C | `crates/db/tests/schema_default_parity.rs` | passing | Database (Drizzle, schema, repositories, sharding); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/shopify-upsert-product-concurrency.test.ts` | C | tbd | baseline-skip | Shopify integration; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/statement-timeout.test.ts` | C | `crates/db/tests/integration_statement_timeout.rs` | passing | Database (Drizzle, schema, repositories, sharding); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/template-installed-resource-kinds.test.ts` | C | tbd | baseline-skip | Template marketplace (install/share/sync); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/thread-control-guarded-update.test.ts` | C | tbd | baseline-skip | Thread control (handover protocol); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/whatsapp-call-recording-retention.test.ts` | C | tbd | baseline-skip | WhatsApp calling (VoIP); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/working-hours-constraints.test.ts` | C | tbd | baseline-skip | Working hours; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/working-hours-contact-filter.test.ts` | C | tbd | baseline-skip | Contact filters and segments; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/working-hours-date-overrides.test.ts` | C | tbd | baseline-skip | Working hours; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/working-hours-members-concurrency.test.ts` | C | tbd | baseline-skip | Ecommerce: settings, open hours, analytics; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/workspace-repository.test.ts` | C | tbd | baseline-skip | Workspaces; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/integration/zoned-date-key-timezone.test.ts` | C | `crates/db/tests/zoned_date_key_timezone.rs` | passing | Analytics dashboards; P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/database/__tests__/keyword-condition-schema.test.ts` | C | tbd | todo | Automated responses and keywords |
| `packages/database/__tests__/leaderboard-participant-repository.test.ts` | C | tbd | todo | Leaderboards |
| `packages/database/__tests__/mac-partitions-repository.test.ts` | C | tbd | todo | Analytics dashboards |
| `packages/database/__tests__/media-library-file-repository.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/database/__tests__/media-library-folder-repository.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/database/__tests__/message-repository-factory-static-import.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/database/__tests__/message-repository-factory.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/database/__tests__/messaging-ad-media-partial.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads) |
| `packages/database/__tests__/messaging-ad-operation-repository.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads) |
| `packages/database/__tests__/messaging-ads-connection-repository.test.ts` | C | tbd | todo | Ads campaigns (Meta messaging ads) |
| `packages/database/__tests__/messenger-credential-schema.test.ts` | C | tbd | todo | Messenger channel |
| `packages/database/__tests__/messenger-welcome-message-flow-repository.test.ts` | C | tbd | todo | Messenger channel |
| `packages/database/__tests__/meta-catalog-sendable-products-repository.test.ts` | C | tbd | todo | Meta Catalog |
| `packages/database/__tests__/money-scale.test.ts` | C | tbd | todo | Ecommerce: settings, open hours, analytics |
| `packages/database/__tests__/oauth-consent.repository.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/database/__tests__/partition-migrations.test.ts` | C | tbd | todo | Database (Drizzle, schema, repositories, sharding) |
| `packages/database/__tests__/pipeline-item-filter.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/database/__tests__/pipeline-partial.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/database/__tests__/platform-admin-repository.test.ts` | C | tbd | todo | Platform admin |
| `packages/database/__tests__/product-repository-list.test.ts` | C | tbd | todo | Ecommerce: products, categories, vendors |
| `packages/database/__tests__/purge-fk-indexes.test.ts` | C | `crates/db/tests/purge_fk_indexes.rs` | passing | Scheduler and scheduled jobs |
| `packages/database/__tests__/reference-existence-repository.test.ts` | C | tbd | todo | Database (Drizzle, schema, repositories, sharding) |
| `packages/database/__tests__/reflink-repository.test.ts` | C | tbd | todo | Leaderboards |
| `packages/database/__tests__/sequence-dispatch-repository.test.ts` | C | tbd | todo | Sequences |
| `packages/database/__tests__/sharded-message-repository.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/database/__tests__/sharding-shared.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/database/__tests__/statement-timeout.test.ts` | C | `crates/db/tests/statement_timeout.rs` | passing | Database (Drizzle, schema, repositories, sharding); some cases not-applicable (P2.3 user decision) |
| `packages/database/__tests__/tag-channel-repository.test.ts` | C | tbd | todo | Tags and labels |
| `packages/database/__tests__/template-selectable-resource-repository.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/database/__tests__/template-version-pin.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/database/__tests__/thread-control-partial.test.ts` | C | tbd | todo | Thread control (handover protocol) |
| `packages/database/__tests__/thread-control-repository.test.ts` | C | tbd | todo | Thread control (handover protocol) |
| `packages/database/__tests__/thread-control-sdk-parity.test.ts` | C | tbd | todo | Thread control (handover protocol) |
| `packages/database/__tests__/timescale.test.ts` | C | tbd | todo | Database (Drizzle, schema, repositories, sharding) |
| `packages/database/__tests__/trigger-repository.test.ts` | C | tbd | todo | Triggers |
| `packages/database/__tests__/user-repository.test.ts` | C | tbd | todo | Database (Drizzle, schema, repositories, sharding) |
| `packages/database/__tests__/whatsapp-business-account-repository.test.ts` | C | tbd | todo | WhatsApp channel |
| `packages/database/__tests__/whatsapp-call-display-outcome.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/database/__tests__/whatsapp-call-list-for-workspace.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/database/__tests__/whatsapp-call-outcome.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/database/__tests__/whatsapp-call-repository.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/database/__tests__/whatsapp-call-workspace-fk-cascade.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/database/__tests__/whatsapp-credential-schema.test.ts` | C | tbd | todo | WhatsApp channel |
| `packages/database/__tests__/whatsapp-signup-session-repository.test.ts` | C | tbd | todo | WhatsApp channel |
| `packages/database/__tests__/with-id-tie-break.test.ts` | C | tbd | todo | Database (Drizzle, schema, repositories, sharding) |
| `packages/database/__tests__/working-hours-repository.test.ts` | C | tbd | todo | Working hours |
| `packages/database/__tests__/working-hours-schema.test.ts` | C | tbd | todo | Working hours |
| `packages/database/__tests__/workspace-api-token-repository.test.ts` | C | tbd | todo | Workspace API tokens and public API |
| `packages/database/__tests__/workspace-member-repository.test.ts` | C | tbd | todo | Workspace members, roles and invitations |
| `packages/database/__tests__/workspace-purge-coverage.test.ts` | C | `crates/db/tests/workspace_purge_coverage.rs` | passing | Workspaces |
| `packages/database/__tests__/workspace-purge-repository.test.ts` | C | tbd | todo | Workspaces |
| `packages/database/__tests__/zoned-date-key.test.ts` | C | `crates/db/tests/zoned_date_key.rs` | passing | Analytics dashboards |
| `packages/decision-config/__tests__/profile.test.ts` | C | tbd | todo | Decision engine (routing profiles) |
| `packages/encryption/__tests__/appointment-tokens.test.ts` | C | tbd | todo | Encryption and signed tokens |
| `packages/encryption/__tests__/encryption.test.ts` | C | tbd | todo | Encryption and signed tokens |
| `packages/encryption/__tests__/keys.test.ts` | C | tbd | todo | Encryption and signed tokens |
| `packages/encryption/__tests__/leaderboard-view-token.test.ts` | C | tbd | todo | Encryption and signed tokens |
| `packages/encryption/__tests__/link-signature.test.ts` | C | tbd | todo | Encryption and signed tokens |
| `packages/encryption/__tests__/media-token.test.ts` | C | tbd | todo | Encryption and signed tokens |
| `packages/encryption/__tests__/minigame-play-token.test.ts` | C | tbd | todo | Encryption and signed tokens |
| `packages/encryption/__tests__/open-link-token.test.ts` | C | tbd | todo | Encryption and signed tokens |
| `packages/encryption/__tests__/shop-token.test.ts` | C | tbd | todo | Encryption and signed tokens |
| `packages/encryption/__tests__/user-data-webview-token.test.ts` | C | tbd | todo | Encryption and signed tokens |
| `packages/encryption/__tests__/user-hash.test.ts` | C | tbd | todo | Encryption and signed tokens |
| `packages/event-bus/__tests__/configured-buses.test.ts` | C | tbd | todo | Event bus and domain events |
| `packages/event-bus/__tests__/event-bus.test.ts` | C | tbd | todo | Event bus and domain events |
| `packages/event-bus/__tests__/index.test.ts` | C | tbd | todo | Event bus and domain events |
| `packages/event-bus/__tests__/worker.test.ts` | C | tbd | todo | Event bus and domain events |
| `packages/events/__tests__/commerce-events.test.ts` | C | tbd | todo | Event bus and domain events |
| `packages/events/__tests__/commerce-webhook-context.test.ts` | C | tbd | todo | Event bus and domain events |
| `packages/events/__tests__/contact-inbox-context.test.ts` | C | tbd | todo | Event bus and domain events |
| `packages/events/__tests__/evaluator-filtered-cache.test.ts` | C | tbd | todo | Event bus and domain events |
| `packages/events/__tests__/trigger-emitter-contact-inbox-roundtrip.test.ts` | C | tbd | todo | Event bus and domain events |
| `packages/events/__tests__/webhook-context.test.ts` | C | tbd | todo | Event bus and domain events |
| `packages/filesystem/__tests__/bounded-fetch.test.ts` | C | `crates/files/tests/bounded_fetch.rs` (8 tests) | passing | Media library and file storage |
| `packages/filesystem/__tests__/channel-media-prefix.test.ts` | C | `crates/files/tests/channel_media_prefix.rs` (2 tests) | passing | Media library and file storage |
| `packages/filesystem/__tests__/copy-source.test.ts` | C | `crates/files/tests/copy_source.rs` (5 tests) | passing | Media library and file storage |
| `packages/filesystem/__tests__/mime-type.test.ts` | C | `crates/files/tests/mime_type.rs` (11 tests) | passing | Media library and file storage |
| `packages/filesystem/__tests__/upload-from-url.test.ts` | C | `crates/files/tests/upload_from_url.rs` (7 tests) | passing | Media library and file storage |
| `packages/filesystem/__tests__/upload-writer.test.ts` | C | `crates/files/tests/upload_writer.rs` (7 tests) | passing | Media library and file storage |
| `packages/flow-config/__tests__/active-campaign-sync-contact.test.ts` | C | tbd | todo | ActiveCampaign integration |
| `packages/flow-config/__tests__/ai-model-catalog.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `packages/flow-config/__tests__/ai-muse-steps.test.ts` | C | tbd | todo | Muse integration |
| `packages/flow-config/__tests__/ai-segment-image.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `packages/flow-config/__tests__/ai-speech-to-text.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `packages/flow-config/__tests__/ai-text-to-speech.test.ts` | C | tbd | todo | AI flow steps (text, image, speech, extraction) |
| `packages/flow-config/__tests__/appointment-scheduling-schema.test.ts` | C | tbd | todo | Appointments and booking |
| `packages/flow-config/__tests__/authoring/compile.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/flow-config/__tests__/authoring/errors.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/flow-config/__tests__/authoring/layout.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/flow-config/__tests__/authoring/normalize-graph.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/flow-config/__tests__/authoring/pipeline-spec.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/flow-config/__tests__/authoring/set-custom-field-operations.test.ts` | C | tbd | todo | Custom fields |
| `packages/flow-config/__tests__/authoring/spec-schema.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/flow-config/__tests__/button-payload.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `packages/flow-config/__tests__/call-phone-number-button.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `packages/flow-config/__tests__/channel-policies.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/flow-config/__tests__/channel-validator.test.ts` | C | tbd | todo | Flow builder and execution |
| `packages/flow-config/__tests__/condition-schema.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `packages/flow-config/__tests__/copy-text-button.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `packages/flow-config/__tests__/deep-link.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `packages/flow-config/__tests__/drip-subscribe-subscriber.test.ts` | C | tbd | todo | Drip integration |
| `packages/flow-config/__tests__/evaluate-decision.test.ts` | C | tbd | todo | Decision engine (routing profiles) |
| `packages/flow-config/__tests__/execute-javascript.test.ts` | - | - | dropped | Execute JavaScript flow step; feature dropped (DECISIONS 2026-11-11) |
| `packages/flow-config/__tests__/external-request.test.ts` | C | tbd | todo | Outbound webhooks |
| `packages/flow-config/__tests__/field-reference.test.ts` | C | tbd | todo | Custom fields |
| `packages/flow-config/__tests__/find-places-near-me.test.ts` | C | tbd | todo | Places near me |
| `packages/flow-config/__tests__/flow-action-target.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `packages/flow-config/__tests__/follow-up.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `packages/flow-config/__tests__/get-response-add-contact.test.ts` | C | tbd | todo | GetResponse integration |
| `packages/flow-config/__tests__/get-user-data-step.test.ts` | C | tbd | todo | Get user data webview |
| `packages/flow-config/__tests__/import-export.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/flow-config/__tests__/klaviyo-sync-profile.test.ts` | C | tbd | todo | Klaviyo integration |
| `packages/flow-config/__tests__/mailer-lite-add-subscriber.test.ts` | C | tbd | todo | MailerLite integration |
| `packages/flow-config/__tests__/mark-conversation-read-state.test.ts` | C | tbd | todo | Flow steps: contact and conversation actions |
| `packages/flow-config/__tests__/media-step-rules.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `packages/flow-config/__tests__/messenger-template.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/flow-config/__tests__/moosend-create-contact.test.ts` | C | tbd | todo | Moosend integration |
| `packages/flow-config/__tests__/open-website.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `packages/flow-config/__tests__/openai-compatible-ai-steps.test.ts` | C | tbd | todo | OpenAI-compatible integration |
| `packages/flow-config/__tests__/payment-request.test.ts` | C | tbd | todo | WhatsApp message templates |
| `packages/flow-config/__tests__/pipeline-action.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/flow-config/__tests__/pipeline-required-fields.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/flow-config/__tests__/pipeline-step.test.ts` | C | tbd | todo | Pipelines (CRM deals) |
| `packages/flow-config/__tests__/quick-replies-carrier.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `packages/flow-config/__tests__/quick-reply-settings.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `packages/flow-config/__tests__/reference-registry-completeness.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `packages/flow-config/__tests__/remap.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `packages/flow-config/__tests__/routable-handle.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `packages/flow-config/__tests__/send-google-ads-conversion.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/flow-config/__tests__/send-meta-capi-event.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `packages/flow-config/__tests__/send-multiple-images.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `packages/flow-config/__tests__/send-products.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `packages/flow-config/__tests__/send-text-length-rules.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `packages/flow-config/__tests__/send-view-catalog-step.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `packages/flow-config/__tests__/sendgrid-add-contact.test.ts` | C | tbd | todo | SendGrid integration |
| `packages/flow-config/__tests__/set-custom-field-operation.test.ts` | C | tbd | todo | Custom fields |
| `packages/flow-config/__tests__/spreadsheet-write-mapping.test.ts` | C | tbd | todo | Google Sheets integration |
| `packages/flow-config/__tests__/step-input-resolution.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `packages/flow-config/__tests__/template-button-open-mini-app.test.ts` | C | tbd | todo | Mini-apps |
| `packages/flow-config/__tests__/template-export-schema.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/flow-config/__tests__/template-flow-node-schema.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/flow-config/__tests__/template-parameter-values.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/flow-config/__tests__/template-structure.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/flow-config/__tests__/thread-control-step.test.ts` | C | tbd | todo | Thread control (handover protocol) |
| `packages/flow-config/__tests__/tiktok-text-rules.test.ts` | C | tbd | todo | TikTok channel |
| `packages/flow-config/__tests__/unresolved-references.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `packages/flow-config/__tests__/wa-template-flow-token.test.ts` | C | tbd | todo | WhatsApp message templates |
| `packages/flow-config/__tests__/wa-template-limited-time-offer.test.ts` | C | tbd | todo | WhatsApp message templates |
| `packages/flow-config/__tests__/wa-template-mpm-rules.test.ts` | C | tbd | todo | WhatsApp message templates |
| `packages/flow-config/__tests__/wa-template-payment-request.test.ts` | C | tbd | todo | WhatsApp message templates |
| `packages/flow-config/__tests__/wa-template-quick-reply-buttons.test.ts` | C | tbd | todo | Flow steps: messages and buttons |
| `packages/flow-config/__tests__/wa-template.test.ts` | C | tbd | todo | Template marketplace (install/share/sync) |
| `packages/flow-config/__tests__/whatsapp-flow-step.test.ts` | C | tbd | todo | WhatsApp Flows (forms) |
| `packages/flow-config/__tests__/whatsapp-location.test.ts` | C | tbd | todo | WhatsApp channel |
| `packages/imports/__tests__/cell.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/imports/__tests__/contacts-header-match.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/imports/__tests__/file-validation.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/imports/__tests__/header-match.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/imports/__tests__/headers.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/imports/__tests__/stream-guard.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/imports/__tests__/tabular.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/imports/__tests__/utils.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/imports/__tests__/xlsx-parser.test.ts` | C | tbd | todo | Imports and exports (contacts, products, flows) |
| `packages/javascript-sandbox/__tests__/client.test.ts` | - | - | dropped | Execute JavaScript flow step; feature dropped (DECISIONS 2026-11-11) |
| `packages/javascript-sandbox/__tests__/contract.test.ts` | - | - | dropped | Execute JavaScript flow step; feature dropped (DECISIONS 2026-11-11) |
| `packages/logger/__tests__/diagnostic.test.ts` | C | tbd | todo | Logger |
| `packages/logger/__tests__/err-serializer.test.ts` | C | tbd | todo | Logger |
| `packages/logger/__tests__/redact.test.ts` | C | tbd | todo | Logger |
| `packages/mail/__tests__/smoke.test.ts` | C | tbd | todo | Transactional mail (system emails) |
| `packages/mini-app/__tests__/__snapshots__/components.test.ts.snap` | C | tbd | todo | Mini-apps |
| `packages/mini-app/__tests__/components.test.ts` | C | tbd | todo | Mini-apps |
| `packages/mini-app/__tests__/custom-fields.test.ts` | C | tbd | todo | Mini-apps |
| `packages/mini-app/__tests__/display-text.test.ts` | C | tbd | todo | Mini-apps |
| `packages/mini-app/__tests__/expression.test.ts` | C | tbd | todo | Mini-apps |
| `packages/mini-app/__tests__/runtime.test.ts` | C | tbd | todo | Mini-apps |
| `packages/mini-app/__tests__/serialize.test.ts` | C | tbd | todo | Mini-apps |
| `packages/mini-app/__tests__/tree.test.ts` | C | tbd | todo | Mini-apps |
| `packages/mini-app/__tests__/uploads.test.ts` | C | tbd | todo | Mini-apps |
| `packages/mini-app/__tests__/validate-issue-codes.test.ts` | C | tbd | todo | Mini-apps |
| `packages/mini-app/__tests__/validate.test.ts` | C | tbd | todo | Mini-apps |
| `packages/payments/__tests__/minor-units.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/payments/__tests__/paypal.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/payments/__tests__/stripe.test.ts` | C | tbd | todo | Licensing, billing and quotas |
| `packages/realtime-protocol/__tests__/auth.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `packages/realtime-protocol/__tests__/frame-reader.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `packages/realtime-protocol/__tests__/presence.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `packages/realtime-protocol/__tests__/realtime-socket.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `packages/realtime-protocol/__tests__/schemas.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `packages/realtime-protocol/__tests__/stream.test.ts` | C | tbd | todo | Realtime (WebSocket gateway) |
| `packages/redis/__tests__/cache-utils.test.ts` | C | tbd | todo | Redis (cache, locks, queues, stores) |
| `packages/redis/__tests__/cas-store.test.ts` | C | tbd | todo | Redis (cache, locks, queues, stores) |
| `packages/redis/__tests__/distributed-lock.test.ts` | C | tbd | todo | Redis (cache, locks, queues, stores) |
| `packages/redis/__tests__/distributed-store.test.ts` | C | tbd | todo | Redis (cache, locks, queues, stores) |
| `packages/redis/__tests__/integration/leaderboard-store.test.ts` | C | tbd | baseline-skip | Redis (cache, locks, queues, stores); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/redis/__tests__/integration/live-counter-scripts.test.ts` | C | tbd | baseline-skip | Redis (cache, locks, queues, stores); P0.3: needs Postgres/Redis, not run — run in P0.7 |
| `packages/redis/__tests__/presence-store.test.ts` | C | tbd | todo | Redis (cache, locks, queues, stores) |
| `packages/scheduler/__tests__/smoke.test.ts` | C | tbd | todo | Scheduler and scheduled jobs |
| `packages/sdk/__tests__/auth.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/sdk/__tests__/channel-error.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/sdk/__tests__/connection-helpers.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/sdk/__tests__/ensure-fresh-auth.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/sdk/__tests__/integration-channel-handler.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/sdk/__tests__/media-storage-prefix.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/sdk/__tests__/message.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/sdk/__tests__/pending-incoming-attachment.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/sdk/__tests__/profile-fields.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/sdk/__tests__/thread-control-result-kinds.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/sdk/__tests__/to-keep-original.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/sdk/__tests__/whatsapp-payment-method-entity.test.ts` | C | tbd | todo | Channel SDK and registry |
| `packages/sequence-scheduler/__tests__/advance-enrollment.test.ts` | C | tbd | todo | Sequences |
| `packages/sequence-scheduler/__tests__/calculate-next-run-at.test.ts` | C | tbd | todo | Sequences |
| `packages/sequence-scheduler/__tests__/cancel-pending-dispatches.test.ts` | C | tbd | todo | Sequences |
| `packages/sequence-scheduler/__tests__/contacts-on-sequences.test.ts` | C | tbd | todo | Sequences |
| `packages/sequence-scheduler/__tests__/dispatch-manager.test.ts` | C | tbd | todo | Sequences |
| `packages/sequence-scheduler/__tests__/enroll-contact.test.ts` | C | tbd | todo | Sequences |
| `packages/sequence-scheduler/__tests__/send-time-validator.test.ts` | C | tbd | todo | Sequences |
| `packages/sequence-scheduler/__tests__/sequence-dispatch.test.ts` | C | tbd | todo | Sequences |
| `packages/ui/__tests__/bar-chart.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/chart-colors.test.ts` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/chart-value-format.test.ts` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/checkbox-group-field.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/combobox-field.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/data-table-labels.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/data-table-row-card.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/data-table-sort-undefined.test.ts` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/data-table.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/date-time-picker-month.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/dialog-mobile-width.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/form-optional-label.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/input-number-field.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/input-number.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/multi-select-select-all.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/sidebar-mobile-handle.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/tags-input-field.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/use-mobile.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/__tests__/weekday-toggle-field.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/src/hooks/__tests__/create-debounced-fn.test.ts` | B | - | todo | UI component library; frontend package |
| `packages/ui/src/hooks/__tests__/use-debounced-callback.test.tsx` | B | - | todo | UI component library; frontend package |
| `packages/ui/src/lib/__tests__/format.test.ts` | B | - | todo | UI component library; frontend package |
| `packages/utils/__tests__/call-capable-channels.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/utils/__tests__/coexist-channels.test.ts` | C | tbd | todo | Coexist (existing-app history sync) |
| `packages/utils/__tests__/concurrency.test.ts` | C | tbd | todo | Ecommerce: settings, open hours, analytics |
| `packages/utils/__tests__/connection.test.ts` | C | tbd | todo | Channel connection and OAuth connect flow |
| `packages/utils/__tests__/coordinates.test.ts` | C | tbd | todo | Utils (shared helpers) |
| `packages/utils/__tests__/country-currency.test.ts` | C | tbd | todo | Ecommerce: settings, open hours, analytics |
| `packages/utils/__tests__/created-at-from-id.test.ts` | C | tbd | todo | Utils (shared helpers) |
| `packages/utils/__tests__/crypto.test.ts` | C | tbd | todo | Utils (shared helpers) |
| `packages/utils/__tests__/csv.test.ts` | C | tbd | todo | Utils (shared helpers) |
| `packages/utils/__tests__/custom-field-resolution-plan.test.ts` | C | tbd | todo | Custom fields |
| `packages/utils/__tests__/custom-field.test.ts` | C | tbd | todo | Custom fields |
| `packages/utils/__tests__/date-expression.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `packages/utils/__tests__/datetime.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `packages/utils/__tests__/env.test.ts` | C | tbd | todo | Utils (shared helpers) |
| `packages/utils/__tests__/error-log.test.ts` | C | tbd | todo | Error logs |
| `packages/utils/__tests__/field-value-calculator.test.ts` | C | tbd | todo | Custom fields |
| `packages/utils/__tests__/form-answer-format.test.ts` | C | tbd | todo | Utils (shared helpers) |
| `packages/utils/__tests__/google-click.test.ts` | C | tbd | todo | Google Ads integration |
| `packages/utils/__tests__/graph-error.test.ts` | C | tbd | todo | Error logs |
| `packages/utils/__tests__/graph-pagination.test.ts` | C | tbd | todo | Utils (shared helpers) |
| `packages/utils/__tests__/id.test.ts` | C | tbd | todo | Utils (shared helpers) |
| `packages/utils/__tests__/image-optimize-settings.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/utils/__tests__/image-optimize.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/utils/__tests__/inbound-webhook-channel.test.ts` | C | tbd | todo | Inbound webhooks |
| `packages/utils/__tests__/inbox-image-support.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/utils/__tests__/json-path.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `packages/utils/__tests__/live-media.test.ts` | C | tbd | todo | Inbox and live chat (conversations, messages) |
| `packages/utils/__tests__/math-expression.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `packages/utils/__tests__/media-download.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/utils/__tests__/meta-capi.test.ts` | C | tbd | todo | Meta Conversions API (CAPI) |
| `packages/utils/__tests__/money.test.ts` | C | tbd | todo | Ecommerce: settings, open hours, analytics |
| `packages/utils/__tests__/normalize-for-match.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `packages/utils/__tests__/placeholder-email.test.ts` | C | tbd | todo | Utils (shared helpers) |
| `packages/utils/__tests__/recurrence-oracle.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/utils/__tests__/recurrence.test.ts` | C | tbd | todo | Broadcasts (incl. recurring) |
| `packages/utils/__tests__/request.test.ts` | C | tbd | todo | Utils (shared helpers) |
| `packages/utils/__tests__/sanitize-file-name.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/utils/__tests__/spintax.test.ts` | C | tbd | todo | Flow steps: logic and utilities |
| `packages/utils/__tests__/stack-discount-percents.test.ts` | C | tbd | todo | Coupons and discount codes |
| `packages/utils/__tests__/storage.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/utils/__tests__/store-incoming-image.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/utils/__tests__/temporal-input.test.ts` | C | tbd | todo | Utils (shared helpers) |
| `packages/utils/__tests__/temporal-storage.test.ts` | C | tbd | todo | Media library and file storage |
| `packages/utils/__tests__/thread-control-channels.test.ts` | C | tbd | todo | Thread control (handover protocol) |
| `packages/utils/__tests__/web-url.test.ts` | C | tbd | todo | Utils (shared helpers) |
| `packages/utils/__tests__/whatsapp-call-hours.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/utils/__tests__/whatsapp-call.test.ts` | C | tbd | todo | WhatsApp calling (VoIP) |
| `packages/utils/__tests__/zod.test.ts` | C | tbd | todo | Utils (shared helpers) |
| `packages/variables/__tests__/assigned-helpers.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/bot-field-variable.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/contact-ad-ids.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/contact-variable.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/deep-replace-contact-variables.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/integration-fields-channel-coverage.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/interpolate-into-javascript.test.ts` | - | - | dropped | Variables (contact/bot/system variable interpolation); feature dropped (DECISIONS 2026-11-11) |
| `packages/variables/__tests__/last-call.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/last-input.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/pipeline-variable.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/queued-messages.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/refresh-profile-snapshot.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/render-custom-field-value.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/replace-custom-field-timezone.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/system-field-timezone.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/system-fields-exhaustive.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/system-fields-helpers.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/variables/__tests__/system-fields.test.ts` | C | tbd | todo | Variables (contact/bot/system variable interpolation) |
| `packages/vitest-config/__tests__/msw.test.ts` | C | tbd | todo | Tooling configs |
| `packages/vitest-config/__tests__/setup-dom.test.ts` | C | tbd | todo | Tooling configs |
| `packages/worker-config/__tests__/ai-agent-job-schema.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/ai-file-job-ids.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/broadcast-job-ids.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/coexist-job-ids.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/connection-build-phase.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/connection-queue-groups.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/enqueue-integration-job.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/heavy-job-schema.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/inbound-webhook-request-job.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/job-wait-strict.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/job-wait.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/marketing-message-job-ids.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/no-redis-env.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/queue-events.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/resolve-step-input-job.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/stored-image-optimization.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/template-queue.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/whatsapp-call-native-media-job-ids.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/whatsapp-identity-change-job.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
| `packages/worker-config/__tests__/whatsapp-voip-signaling-job-ids.test.ts` | C | tbd | todo | Worker config (queues, job schemas) |
