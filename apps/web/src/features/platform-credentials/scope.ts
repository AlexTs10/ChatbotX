// The client part of apps/builder/src/features/platform-credentials/scope.ts
// (`resolveCredentialScopedUserId` runs in the Rust server).
import { z } from "zod"

export const credentialScopeSchema = z.enum(["user", "platform"])
export type CredentialScope = z.infer<typeof credentialScopeSchema>
