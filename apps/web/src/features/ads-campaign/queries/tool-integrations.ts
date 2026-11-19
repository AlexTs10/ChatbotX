// apps/builder/src/features/ads-campaign/queries/{tool-integrations,
// tool-active-integration-ids,index}.ts for the SPA: the Rust queries of
// crates/api/src/rpc_procedures/ads.rs.
import type { MessagingAdChannel } from "@chatbotx.io/database/partials"
import { serverQuery } from "@/lib/server-action"

/** Redacted to id + name only (never the full integration row). */
export type MessagingAdsToolIntegration = {
  id: string
  name: string
}

export type MessagingAdsToolIntegrationsResult = {
  integrations: MessagingAdsToolIntegration[]
  hasUnsupportedIntegrations: boolean
}

type ToolInput = { workspaceId: string; channel: MessagingAdChannel }

export const listMessagingAdsToolIntegrations = (input: ToolInput) =>
  serverQuery<MessagingAdsToolIntegrationsResult>(
    "listMessagingAdsToolIntegrations",
    input.workspaceId,
    [input],
  )

export const listActiveMessagingAdsIntegrationIds = (input: ToolInput) =>
  serverQuery<string[]>(
    "listActiveMessagingAdsIntegrationIds",
    input.workspaceId,
    [input],
  )

export const checkMessagingAdsConnectionState = (
  input: ToolInput & { integrationId: string },
) =>
  serverQuery<{ connected: boolean; reconnectNeeded: boolean }>(
    "checkMessagingAdsConnectionState",
    input.workspaceId,
    [input],
  )
