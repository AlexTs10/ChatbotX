// Type-check view of `@chatbotx.io/analytics-nextjs/provider/analytics-api-context`
// (tsconfig `paths` only; Vite bundles the real module). The real file types
// the context with the server router (`../routes`), which pulls the analytics
// server code into the SPA type check.
import type { ReactNode } from "react"

// biome-ignore lint/suspicious/noExplicitAny: the router type lives on the server
export type AnalyticsApi = any

export declare const AnalyticsApiProvider: (props: {
  api: AnalyticsApi
  children: ReactNode
}) => ReactNode

export declare const useAnalyticsApi: () => AnalyticsApi
