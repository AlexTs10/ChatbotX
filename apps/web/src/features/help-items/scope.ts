// The client half of apps/builder/src/features/help-items/scope.ts; the
// tenant resolution (`resolveHelpItemTenantId`) runs in the Rust server.
import { z } from "zod"

export const helpItemScopeSchema = z.enum(["tenant", "platform"])
export type HelpItemScope = z.infer<typeof helpItemScopeSchema>
