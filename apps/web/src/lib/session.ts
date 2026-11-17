// The signed-in user and the tenant settings (white-label) for every page:
// the Rust page loader `pages.session` (P4 spec §5.2). The builder read
// both on the server in each layout.

import { queryOptions } from "@tanstack/react-query"
import type { TenantSettings } from "@/lib/server-types"
import { loadPage } from "./server-action"

export type SessionUser = {
  id: string
  name: string | null
  email: string
  image: string | null
  emailVerified: boolean
  mustChangePassword: boolean
}

export type SessionPayload = {
  user: SessionUser | null
  tenant: TenantSettings
}

const HTTP_RE = /^http/

const defaultTenant = (): TenantSettings => {
  const appUrl = window.location.origin
  return {
    appUrl,
    publicRealtimeUrl: `${appUrl.replace(HTTP_RE, "ws")}/ws`,
    storageUrl: `${appUrl}/storage/`,
    name: "ChatbotX",
    logoLightUrl: `${appUrl}/brand/logo_white.svg`,
    logoDarkUrl: `${appUrl}/brand/logo_black.svg`,
    faviconUrl: `${appUrl}/brand/icon_black.svg`,
    theme: null,
    customJS: null,
    customCSS: null,
    policyUrl: "https://chatbotx.io/privacy/",
    termsOfServiceUrl: "https://chatbotx.io/terms/",
    signupEmailTemplate: null,
    forgotPasswordEmailTemplate: null,
    magicLinkEmailTemplate: null,
    accountCredentialsEmailTemplate: null,
    helpItems: [],
  }
}

export const sessionQuery = queryOptions({
  queryKey: ["spa", "session"],
  queryFn: async (): Promise<SessionPayload> => {
    const payload: Partial<SessionPayload> = await loadPage<
      Partial<SessionPayload>
    >("session").catch(() => ({}))
    return {
      user: payload.user ?? null,
      tenant: { ...defaultTenant(), ...payload.tenant },
    }
  },
  staleTime: 60_000,
})
