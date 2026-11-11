// Helpers of the P1.7 `conversations` tests (01-09). Not a test file.

import { expect } from "vitest"
import type { P107ContactInboxInput, P107ContactInput } from "../../src/fixtures/ext/p1-07.ts"
import { signedInClient, type Tenant } from "../../src/fixtures/index.ts"
import { connectRealtime, type RealtimeSocket, workspaceSocketUrl } from "../../src/realtime.ts"
import { rpc } from "../../src/rpc.ts"
import { uniqueId } from "../../src/unique.ts"
import { type Api, ext } from "./helpers.ts"

/** Keys of one `conversations.list` (view full) / `conversations.get` item. */
export const CONVERSATION_KEYS = [
  "additionalAttributes",
  "adminRepliedAt",
  "agentLastReadAt",
  "aiContextLastMessageId",
  "archivedAt",
  "assignedInboxTeam",
  "assignedInboxTeamId",
  "assignedUser",
  "assignedUserAt",
  "assignedUserId",
  "awaitingHumanReplySince",
  "botEnabled",
  "botResumeAt",
  "contact",
  "contactId",
  "contactInboxes",
  "contactLastReadAt",
  "contactRepliedAt",
  "createdAt",
  "currentStep",
  "followed",
  "id",
  "lastActivityAt",
  "lastStep",
  "messages",
  "responseDueAt",
  "responseSlaBreachingAt",
  "responseSlaTargetSeconds",
  "sourceId",
  "updatedAt",
  "workspaceId",
].sort()

/** Keys of a `contactInboxes[]` item, view full. `inbox` carries only `name`. */
export const CONTACT_INBOX_KEYS = [
  "adReferral",
  "channel",
  "contactId",
  "contactLastReadAt",
  "googleAdsClick",
  "id",
  "inbox",
  "inboxId",
  "language",
  "lastIncomingMessageAt",
  "lastMessageAt",
  "source",
  "sourceId",
  "sourceUserId",
  "sourceUsername",
  "threadControlLastEvent",
  "threadControlState",
  "threadControlUpdatedAt",
  "threadOwnerAppId",
  "threadOwnerExpiresAt",
  "threadOwnerRole",
].sort()

/** Keys of a `contactInboxes[]` item, view compact (no routing / ad data, `inbox` = id, name, channel). */
export const COMPACT_CONTACT_INBOX_KEYS = [
  "channel",
  "contactId",
  "contactLastReadAt",
  "id",
  "inbox",
  "inboxId",
  "language",
  "lastIncomingMessageAt",
  "lastMessageAt",
  "source",
  "sourceId",
  "sourceUserId",
  "sourceUsername",
].sort()

/** Every `Contact` column (same `contactResource` as P1.6). */
export const CONTACT_KEYS = [
  "avatar",
  "blockedAt",
  "broadcastSubscribedAt",
  "city",
  "country",
  "createdAt",
  "email",
  "emailOptIn",
  "emailVerified",
  "firstName",
  "fullName",
  "gender",
  "id",
  "lastName",
  "lastReadAt",
  "locale",
  "location",
  "phoneNumber",
  "ref",
  "state",
  "subscribedAt",
  "timezone",
  "totalNewTagged",
  "totalTagged",
  "updatedAt",
  "workspaceId",
].sort()

export const keysOf = (value: unknown): string[] => Object.keys(value as object).sort()

export type ConversationContactInbox = {
  id: string
  contactId: string
  inboxId: string
  channel: string
  source: string
  sourceId: string
  threadControlState?: string | null
  threadOwnerRole?: string | null
  threadOwnerAppId?: string | null
  threadControlUpdatedAt?: string | null
  threadOwnerExpiresAt?: string | null
  threadControlLastEvent?: string | null
  inbox: Record<string, unknown>
  [key: string]: unknown
}

export type ConversationItem = {
  id: string
  contactId: string
  workspaceId: string
  archivedAt: string | null
  followed: boolean
  botEnabled: boolean
  botResumeAt: string | null
  agentLastReadAt: string | null
  lastActivityAt: string | null
  assignedUserId: string | null
  assignedInboxTeamId: string | null
  assignedUserAt: string | null
  assignedUser: Record<string, unknown> | null
  assignedInboxTeam: Record<string, unknown> | null
  contact: Record<string, unknown> | null
  contactInboxes: ConversationContactInbox[]
  messages: Array<Record<string, unknown>>
  [key: string]: unknown
}

export type ConversationPage = {
  data: ConversationItem[]
  nextCursor: string | null
  prevCursor: string | null
}

/**
 * `GET /v1/conversations`. `status`, `tags` and `contactFilter` are sent as
 * JSON-encoded strings (the documented encoding); other values as they are.
 */
export const listConversations = (
  api: Api,
  params: {
    status?: unknown
    tags?: unknown
    contactFilter?: unknown
    [key: string]: unknown
  } = {},
) => {
  const search = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined) {
      continue
    }
    const raw = ["status", "tags", "contactFilter"].includes(key) && typeof value !== "string"
    search.set(key, raw ? JSON.stringify(value) : String(value))
  }
  const text = search.toString()
  return api.get<ConversationPage>(`/v1/conversations${text ? `?${text}` : ""}`)
}

/** The conversation ids of a list page, in response order. */
export const idsOf = (response: { status: number; text: string; json: ConversationPage }): string[] => {
  expect(response.status, response.text).toBe(200)
  return response.json.data.map((item) => item.id)
}

export const getConversation = async (api: Api, id: string): Promise<ConversationItem> => {
  const response = await api.get<{ data: ConversationItem }>(`/v1/conversations/${id}`)
  expect(response.status, response.text).toBe(200)
  return response.json.data
}

/** A contact on a fresh plain inbox of `channel` with one conversation. */
export const seedConversation = async (
  workspaceId: string,
  input: Omit<P107ContactInput, "workspaceId" | "inboxes"> & {
    channel?: string
    inboxId?: string
    contactInbox?: Omit<P107ContactInboxInput, "inboxId" | "channel">
  } = {},
) => {
  const fx = await ext()
  const channel = input.channel ?? "telegram"
  const inboxId =
    input.inboxId ?? (await fx.createInbox({ workspaceId, channel, name: `Parity ${channel} ${uniqueId()}` })).inboxId
  const { channel: _c, inboxId: _i, contactInbox, ...contact } = input
  const created = await fx.createContact({
    workspaceId,
    ...contact,
    inboxes: [{ inboxId, channel, ...(contactInbox ?? {}) }],
  })
  return {
    ...created,
    inboxId,
    conversationId: created.conversationId as string,
    contactInboxId: created.contactInboxIds[0] as string,
    sourceId: created.sourceIds[0] as string,
  }
}

/** A member connect token (session RPC) and an open workspace socket. */
export const openWorkspaceSocket = async (tenant: Tenant): Promise<RealtimeSocket> => {
  const http = await signedInClient(tenant)
  const minted = await rpc<{ token: string }>(
    http,
    "realtimeAPI/mintWorkspaceConnectTokenAuthenticatedAPI",
    { workspaceId: tenant.workspaceId },
    { method: "GET" },
  )
  expect(minted.status, minted.response.text).toBe(200)
  const socket = connectRealtime(workspaceSocketUrl(tenant.workspaceId, minted.json.token))
  await socket.opened
  return socket
}

/** Seconds-precision instant `ms` ago (seeded rows; outside the normalizer window). */
export const ago = (ms: number): string => new Date(Date.now() - ms).toISOString()

export const HOUR = 60 * 60 * 1000
export const DAY = 24 * HOUR
