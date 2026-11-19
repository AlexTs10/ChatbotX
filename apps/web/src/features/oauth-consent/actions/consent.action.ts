// The client side of apps/builder/src/features/oauth-consent/actions/consent.action.ts:
// the Rust server serves it as `/rpc/authActions/submitOAuthConsent` (Q.A1).
import { client } from "@/lib/orpc/orpc"
import { customAction } from "@/lib/server-action"

export const submitOAuthConsentAction = customAction<{ url?: string }>(
  (input: { accept: boolean; oauthQuery: string; workspaceId?: string }) =>
    client.authActions.submitOAuthConsent(input),
)
