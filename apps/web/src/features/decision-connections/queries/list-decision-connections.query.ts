// apps/builder/src/features/decision-connections/queries for the SPA:
// `decisionConnectionService.listSafe` in the Rust server.
import type { DecisionConnectionSafe } from "@chatbotx.io/business"
import { serverQuery } from "@/lib/server-action"

export const listDecisionConnections = (workspaceId: string) =>
  serverQuery<DecisionConnectionSafe[]>("listDecisionConnections", workspaceId, [])
