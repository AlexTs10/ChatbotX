// apps/builder/src/features/integrations/queries/get-ai-integrations.ts for
// the SPA: the same check in the Rust server (P4 spec §5.1).
import { serverQuery } from "@/lib/server-action"

export const hasAIIntegration = (workspaceId: string): Promise<boolean> =>
  serverQuery("hasAIIntegration", workspaceId, [workspaceId])
