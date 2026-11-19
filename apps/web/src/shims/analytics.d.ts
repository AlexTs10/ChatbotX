// Type-check view of `@chatbotx.io/analytics` (tsconfig `paths` only; Vite
// bundles the real module). The dashboard components import only types from
// it, but its root also exports the Postgres repositories and services, which
// pull the server code into the SPA type check.
export * from "../../../../packages/analytics/src/lib"
export * from "../../../../packages/analytics/src/schemas"

// The server routes of `@chatbotx.io/analytics-nextjs` (type-checked through
// its package graph) import these services; the SPA never runs them.
// biome-ignore-start lint/suspicious/noExplicitAny: server services, not in the SPA
export declare const agentResponseAnalyticsService: any
export declare const botMessageAnalyticsService: any
export declare const broadcastAnalyticsService: any
export declare const commentAutomationAnalyticsService: any
export declare const contactAnalyticsService: any
export declare const conversationAnalyticsService: any
export declare const flowAnalyticsService: any
export declare const macAnalyticsService: any
export declare const magicLinkAnalyticsService: any
export declare const messageAnalyticsService: any
export declare const refLinkAnalyticsService: any
export declare const sequenceAnalyticsService: any
// The business services (type-checked through `@chatbotx.io/business`).
export declare const emailTopicAnalyticsService: any
export declare const macRepository: any
export declare const macTrackingService: any
export type CreateEmailRecipientInput = any
export type EmailRecipientTransition = any
export type EmailTopicCounterIncrement = any
// biome-ignore-end lint/suspicious/noExplicitAny: server services, not in the SPA
