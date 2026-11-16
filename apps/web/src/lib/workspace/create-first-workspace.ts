// The client-safe part of the builder module (error keys and the
// `/channels/create` path); the workspace create runs on the Rust server.
import type { MessageKey } from "@/features/channel-connect/lib/message-key"

/** Query param `/channels/create` reads to explain why a channel connect could not start. */
const CREATE_CHANNEL_ERROR_PARAM = "error"

/**
 * Known failures keyed by `ChatbotXException.code` or connect-flow error code
 * → the copy `/channels/create` shows for them. Any other error keeps
 * propagating, so an unexpected failure is never dressed up as a known error.
 */
export const CREATE_CHANNEL_ERROR_MESSAGE_KEYS = {
  workspaceLimitReached: "channels.connectMany.reason.workspaceLimit",
  trialExpired: "channels.connectMany.sessionError.trialExpired",
  macLimitReached: "channels.connectMany.sessionError.macLimitReached",
  sessionExpired: "channels.connectMany.sessionError.sessionExpired",
  noPages: "channels.connectMany.sessionError.noPages",
  noInstagramAccounts: "channels.connectMany.sessionError.noInstagramAccounts",
  exchangeFailed: "channels.connectMany.sessionError.exchangeFailed",
  providerError: "channels.connectMany.sessionError.providerError",
  notMember: "channels.connectMany.sessionError.notMember",
  connectFailed: "channels.connectMany.sessionError.connectFailed",
  tooManyAttempts: "channels.connectMany.sessionError.tooManyAttempts",
} as const satisfies Record<string, MessageKey>

export type CreateChannelErrorCode =
  keyof typeof CREATE_CHANNEL_ERROR_MESSAGE_KEYS

export function isCreateChannelErrorCode(
  value: unknown,
): value is CreateChannelErrorCode {
  return (
    typeof value === "string" &&
    Object.hasOwn(CREATE_CHANNEL_ERROR_MESSAGE_KEYS, value)
  )
}

/**
 * The `/channels/create` URL. With a `workspaceId` the picker connects into
 * that workspace; without one it is the "first/new workspace" flow, which
 * creates a workspace. So a failure inside a workspace MUST keep its id, or
 * the user's retry would try to create a workspace (and can hit the plan's
 * workspace limit) instead of connecting where they were.
 */
export function createChannelPath(params: {
  error?: CreateChannelErrorCode
  workspaceId?: string | null
}): string {
  const query = new URLSearchParams()
  if (params.workspaceId) {
    query.set("workspaceId", params.workspaceId)
  }
  if (params.error) {
    query.set(CREATE_CHANNEL_ERROR_PARAM, params.error)
  }
  const search = query.toString()
  return search ? `/channels/create?${search}` : "/channels/create"
}
