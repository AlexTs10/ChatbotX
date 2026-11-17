// apps/builder/src/lib/auth/auth-page-options.ts: the server part is the
// Rust page loader `pages.authOptions` (P4 spec §5.2).

import { loadPage } from "@/lib/server-action"
import type { SocialProvider } from "@/lib/server-types"

export type GenericOAuthButton = {
  displayName: string
  logoUrl?: string
}

export type AuthPageOptions = {
  enabledProviders: SocialProvider[]
  genericOAuth: GenericOAuthButton | null
  signUpAllowed: boolean
}

export const resolveAuthPageOptions = (): Promise<AuthPageOptions> =>
  loadPage<AuthPageOptions>("authOptions").catch(() => ({
    enabledProviders: [],
    genericOAuth: null,
    signUpAllowed: true,
  }))
