// apps/builder/src/features/inbound-webhooks/queries for the SPA: the
// `inboundWebhookService` reads run in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"
import type { toInboundWebhookSummary } from "../schema/public"

type InboundWebhookSummary = ReturnType<typeof toInboundWebhookSummary>

type Page<T> = { data: T[]; pageCount: number }

export const listInboundWebhooks = (input: {
  workspaceId: string
  page: number
  perPage: number
}) =>
  serverQuery<Page<InboundWebhookSummary>>(
    "listInboundWebhooks",
    input.workspaceId,
    [input],
  )

// biome-ignore lint/suspicious/noExplicitAny: the editor reads the stored row
export const findInboundWebhookForEditor = (input: {
  workspaceId: string
  id: string
}) => serverQuery<any>("findInboundWebhookForEditor", input.workspaceId, [input])

export const listInboundWebhookRequests = (input: {
  workspaceId: string
  inboundWebhookId: string
  page: number
  perPage: number
}) =>
  // biome-ignore lint/suspicious/noExplicitAny: the request summary resource
  serverQuery<Page<any>>("listInboundWebhookRequests", input.workspaceId, [
    input,
  ])
