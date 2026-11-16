// The client part of apps/builder/src/env.ts. The Rust server writes
// `window.__ENV` into index.html (the builder's PublicEnvScript did it).
import { clientEnv } from "./lib/client-env"

type Edition = "community" | "enterprise" | "cloud"

const edition = (clientEnv("NEXT_PUBLIC_EDITION") || "community") as Edition

export const env = {
  NEXT_PUBLIC_BUILDER_URL:
    clientEnv("NEXT_PUBLIC_BUILDER_URL") || window.location.origin,
  NEXT_PUBLIC_BROKER_URL: clientEnv("NEXT_PUBLIC_BROKER_URL"),
  NEXT_PUBLIC_EDITION: edition,
  NEXT_PUBLIC_INTERNAL_STORAGE_URL:
    clientEnv("NEXT_PUBLIC_INTERNAL_STORAGE_URL") ||
    "http://localhost:9000/chatbotx/",
  NEXT_PUBLIC_STORAGE_URL: clientEnv("NEXT_PUBLIC_STORAGE_URL"),
  NEXT_PUBLIC_PANCAKE_CHAT_PAGE_ID: clientEnv(
    "NEXT_PUBLIC_PANCAKE_CHAT_PAGE_ID",
  ),
  NEXT_PUBLIC_AUTH_OAUTH_DISPLAY_NAME: clientEnv(
    "NEXT_PUBLIC_AUTH_OAUTH_DISPLAY_NAME",
  ),
  NEXT_PUBLIC_AUTH_OAUTH_LOGO_URL: clientEnv("NEXT_PUBLIC_AUTH_OAUTH_LOGO_URL"),
  NEXT_PUBLIC_ALLOWED_DEV_ORIGINS: clientEnv("NEXT_PUBLIC_ALLOWED_DEV_ORIGINS")
    ?.split(",")
    .map((value) => value.trim())
    .filter(Boolean),
}

export const isEnterprise = () => env.NEXT_PUBLIC_EDITION === "enterprise"
export const isCloud = () => env.NEXT_PUBLIC_EDITION === "cloud"
export const isCommunity = () => env.NEXT_PUBLIC_EDITION === "community"
