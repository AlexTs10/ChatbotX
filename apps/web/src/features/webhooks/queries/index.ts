// apps/builder/src/features/webhooks/queries for the SPA: the webhook reads
// run in the Rust server (P4 spec §5.1).
import type { StoreSource } from "@chatbotx.io/database/partials"
import { serverQuery } from "@/lib/server-action"
import type { WebhookCollection } from "../schema"
import type { GetWebhooksSchema } from "../schema/get-webhook-schema"
import type UpdateWebhookForm from "../update-webhook-form"

type WebhookWithConditions = Parameters<typeof UpdateWebhookForm>[0]["webhook"]

export const getWebhooks = (input: GetWebhooksSchema) =>
  serverQuery<WebhookCollection>("getWebhooks", input.workspaceId, [input])

/** `findWebhookWithConditions`: `null` when not in the workspace. */
export const findWebhook = (params: { id: string; workspaceId: string }) =>
  serverQuery<WebhookWithConditions | null>("findWebhook", params.workspaceId, [
    params,
  ])

/** `ecommerceSettingsService.storeSource` (the edit page's condition editor). */
export const getWebhookStoreSource = (workspaceId: string) =>
  serverQuery<StoreSource>("webhookStoreSource", workspaceId, [])
