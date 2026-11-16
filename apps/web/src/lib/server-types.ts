// Types of `@chatbotx.io/business` that the UI reads. Copied here: a type
// import of the package root pulls the whole server graph into the SPA type
// check. Keep them the same as the business package.
import type { TenantHelpItemModel } from "@chatbotx.io/database/types"

export type EmailTemplate = { subject?: string; body?: string }

/** `platform/settings.ts` */
export type TenantSettings = {
  appUrl: string
  publicRealtimeUrl: string
  storageUrl: string
  name: string
  logoLightUrl: string
  logoDarkUrl: string
  faviconUrl: string
  theme: string | null
  customJS: string | null
  customCSS: string | null
  policyUrl: string | null
  termsOfServiceUrl: string | null
  signupEmailTemplate: EmailTemplate | null
  forgotPasswordEmailTemplate: EmailTemplate | null
  magicLinkEmailTemplate: EmailTemplate | null
  accountCredentialsEmailTemplate: EmailTemplate | null
  helpItems: TenantHelpItemModel[]
}

/** `whatsapp-call/call-summary-service.ts` */
export type CallTranscriptSegment = {
  speaker?: string
  start: number
  end: number
  text: string
}

/** `integration/service.ts` */
export type TokenRefreshErrorChannel =
  | "zalo"
  | "tiktok"
  | "instagram"
  | "instagramFacebook"
  | "messenger"
  | "whatsapp"
  | "threads"

export type TokenRefreshErrorIntegration = {
  id: string
  channel: TokenRefreshErrorChannel
  name: string
  error: string
}

/** `@chatbotx.io/auth/server` `SOCIAL_PROVIDERS` */
export type SocialProvider = "google" | "facebook"

/** `@chatbotx.io/ai/server` `services/call-summarizer.ts` */
export type CallSummaryResult = {
  summary: string
  keyPoints?: string[]
  actionItems?: string[]
}

/** `@chatbotx.io/ai` `schemas/ai-model.ts` `aiProviders` */
export type AIProvider =
  | "openai"
  | "gemini"
  | "claude"
  | "deepseek"
  | "openrouter"
  | "muse"
