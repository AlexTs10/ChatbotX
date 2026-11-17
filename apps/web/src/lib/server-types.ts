// Types of `@chatbotx.io/business` that the UI reads. Copied here: a type
// import of the package root pulls the whole server graph into the SPA type
// check. Keep them the same as the business package.
import type {
  AIEmbeddingStatus,
  AIFileChunking,
  AIFileProcessingRun,
  ResolvedAIFileChunkingConfig,
} from "@chatbotx.io/database/partials"
import type {
  AIFileModel,
  IntegrationMetaCatalogModel,
  PipelineItemModel,
  PipelineStageModel,
  TenantHelpItemModel,
} from "@chatbotx.io/database/types"

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

/** `referral/utils.ts` */
export type RefConfig =
  | { type: "flow"; flowId: string; nodeId?: string }
  | { type: "draft"; flowId: string }
  | { type: "minigame-share"; minigameId: string; referrerContactId: string }
  | { type: "leaderboard"; leaderboardId: string; contactId: string }
  | { type: "poll"; pollId: string }
  | { type: "qr-code"; name: string }
  | { type: "reflink"; name: string }

/** `ai-file/service.ts` (`AIFileWithProcessing`) */
export type AIFileWithProcessing = Omit<
  AIFileModel,
  "chunking" | "chunkingConfig"
> & {
  url: string
  chunksCount: number
  processingStatus: AIEmbeddingStatus
  chunking: AIFileChunking | null
  chunkingOutdated: boolean
  chunkingConfig: ResolvedAIFileChunkingConfig & { isDefault: boolean }
  indexOutdated: boolean
  processingRun: (AIFileProcessingRun & { stale: boolean }) | null
}

export type AIFileWithEmbeddingStatus = AIFileWithProcessing

/** `meta-catalog/integration.service.ts` (`findByWorkspaceId`) */
export type MetaCatalogConnectionModel = IntegrationMetaCatalogModel

/** `pipeline/item-query.service.ts` */
export type PipelineItemView = {
  item: PipelineItemModel
  contact: {
    id: string
    fullName: string | null
    avatar: string | null
    email: string | null
    phoneNumber: string | null
    channel: string | null
  } | null
  assignees: { workspaceMemberIds: string[]; inboxTeamIds: string[] }
  commentCount: number
}

/** `pipeline/item-query.service.ts` */
export type PipelineBoardStage = {
  stage: PipelineStageModel
  count: number
  totalValue: number
  items: PipelineItemView[]
  nextCursor: string | null
}
