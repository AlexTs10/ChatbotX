// apps/builder/src/features/facebook-marketing-messages/queries (+ the page
// data of `app/space/[workspaceId]/fb-marketing-messages/*`) for the SPA: the
// Rust queries of crates/api/src/rpc_procedures/ads.rs.
import type { FacebookMarketingMessageModel } from "@chatbotx.io/database/types"
import { serverQuery } from "@/lib/server-action"

export type CampaignCounts = { sent: number; delivered: number; failed: number }

export type MarketingMessagesGrantState = "missing" | "expired" | "valid"

export const getMarketingMessagesPage = (workspaceId: string) =>
  serverQuery<{
    grantState: MarketingMessagesGrantState
    rows: FacebookMarketingMessageModel[]
    counts: Record<string, CampaignCounts>
  }>("getMarketingMessagesPage", workspaceId, [workspaceId])

export const getMarketingMessagePage = (workspaceId: string, id: string) =>
  serverQuery<{
    grantState: MarketingMessagesGrantState
    row: FacebookMarketingMessageModel
    // biome-ignore lint/suspicious/noExplicitAny: `getStats` lives on the server
    stats: any
  }>("getMarketingMessagePage", workspaceId, [workspaceId, id])

export const resolveMarketingMessagesGrant = (workspaceId: string) =>
  serverQuery<{ state: MarketingMessagesGrantState }>(
    "resolveMarketingMessagesGrant",
    workspaceId,
    [workspaceId],
  )
