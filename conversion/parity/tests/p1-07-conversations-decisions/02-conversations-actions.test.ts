// The conversation actions (scope `inbox`): assign, archive / unarchive,
// read / unread, follow / unfollow, enableBot / disableBot. Each returns
// `{success: true}`; the effect is read back with conversations.get / list,
// and the realtime events (conversationAssigned, conversationUpdated) are
// read from a workspace socket of the owner.

import { describe, expect, it } from "vitest"
import { DEFAULT_PASSWORD, fixtures } from "../../src/fixtures/index.ts"
import { uniqueEmail } from "../../src/unique.ts"
import { expectError, expectInvalid, expectIsoNear, ext, newTenant, UNKNOWN_ID } from "./helpers.ts"
import {
  ago,
  type ConversationItem,
  DAY,
  getConversation,
  HOUR,
  idsOf,
  listConversations,
  openWorkspaceSocket,
  seedConversation,
} from "./helpers-conversations.ts"

const ACTIONS = ["archive", "unarchive", "read", "unread", "follow", "unfollow", "enable-bot", "disable-bot"] as const

describe("conversations.assign", () => {
  it("assigns to a user (u_<id>), to an inbox team (t_<id>) and unassigns (null); conversationAssigned events", async () => {
    const testStart = Date.now()
    const { tenant, api } = await newTenant("p107-assign")
    const seeded = await seedConversation(tenant.workspaceId, { conversation: { lastActivityAt: ago(HOUR) } })
    const { inboxTeamId } = await (await ext()).createInboxTeam({ workspaceId: tenant.workspaceId, name: "Team A" })
    const socket = await openWorkspaceSocket(tenant)
    try {
      const toUser = await api.post(`/v1/conversations/${seeded.conversationId}/assign`, {
        assignedId: `u_${tenant.userId}`,
      })
      expect(toUser.status, toUser.text).toBe(200)
      expect(toUser.json).toEqual({ success: true })
      const assigned = await getConversation(api, seeded.conversationId)
      expect(assigned).toMatchObject({ assignedUserId: tenant.userId, assignedInboxTeamId: null, assignedInboxTeam: null })
      expectIsoNear(assigned.assignedUserAt, testStart)
      expect(assigned.assignedUser).toMatchObject({ id: tenant.userId, email: tenant.email, name: "Parity p107-assign" })
      const event = await socket.waitFor("conversationAssigned", (e) =>
        JSON.stringify(e.data).includes(seeded.conversationId),
      )
      expect(event.data).toEqual({
        conversationIds: [seeded.conversationId],
        assignedUserId: tenant.userId,
        assignedInboxTeamId: null,
      })

      const toTeam = await api.post(`/v1/conversations/${seeded.conversationId}/assign`, {
        assignedId: ` t_${inboxTeamId} `,
      })
      expect(toTeam.status, toTeam.text).toBe(200)
      const team = await getConversation(api, seeded.conversationId)
      expect(team).toMatchObject({
        assignedUserId: null,
        assignedUserAt: null,
        assignedUser: null,
        assignedInboxTeamId: inboxTeamId,
        assignedInboxTeam: { id: inboxTeamId, workspaceId: tenant.workspaceId, name: "Team A" },
      })
      await socket.waitFor("conversationAssigned", (e) => (e.data as { assignedInboxTeamId?: string }).assignedInboxTeamId === inboxTeamId)

      const unassign = await api.post(`/v1/conversations/${seeded.conversationId}/assign`, { assignedId: null })
      expect(unassign.status, unassign.text).toBe(200)
      expect(await getConversation(api, seeded.conversationId)).toMatchObject({
        assignedUserId: null,
        assignedInboxTeamId: null,
        assignedUser: null,
        assignedInboxTeam: null,
        assignedUserAt: null,
      })
      const cleared = await socket.waitFor(
        "conversationAssigned",
        (e) => {
          const data = e.data as { assignedUserId: unknown; assignedInboxTeamId: unknown }
          return data.assignedUserId === null && data.assignedInboxTeamId === null
        },
      )
      expect(cleared.data).toEqual({
        conversationIds: [seeded.conversationId],
        assignedUserId: null,
        assignedInboxTeamId: null,
      })
      expect(idsOf(await listConversations(api, { assignedId: "unassigned" }))).toEqual([seeded.conversationId])
    } finally {
      await socket.close()
    }
  })

  it("assigns to an agent member of the workspace; re-assigning the same user keeps assignedUserAt", async () => {
    const { tenant, api } = await newTenant("p107-assign-agent")
    const driver = await fixtures()
    const { userId } = await driver.createUser({ email: uniqueEmail("agent"), password: DEFAULT_PASSWORD, name: "Agent" })
    await (await ext()).addMember({ workspaceId: tenant.workspaceId, userId })
    const seeded = await seedConversation(tenant.workspaceId)
    const path = `/v1/conversations/${seeded.conversationId}/assign`
    expect((await api.post(path, { assignedId: `u_${userId}` })).status).toBe(200)
    const first = await getConversation(api, seeded.conversationId)
    expect(first).toMatchObject({ assignedUserId: userId, assignedUser: { id: userId, name: "Agent" } })
    expect((await api.post(path, { assignedId: `u_${userId}` })).status).toBe(200)
    const second = await getConversation(api, seeded.conversationId)
    expect(second.assignedUserAt).toBe(first.assignedUserAt)
  })

  it("400 invalidAssignee for a user who is not a member; 404 for an unknown or foreign inbox team", async () => {
    const { tenant, api } = await newTenant("p107-assign-invalid")
    const other = await newTenant("p107-assign-other")
    const { inboxTeamId: foreignTeam } = await (await ext()).createInboxTeam({
      workspaceId: other.tenant.workspaceId,
      name: "Foreign team",
    })
    const seeded = await seedConversation(tenant.workspaceId)
    const path = `/v1/conversations/${seeded.conversationId}/assign`
    expectError(await api.post(path, { assignedId: `u_${other.tenant.userId}` }), 400, "invalidAssignee", {
      message: "User is not valid",
    })
    expectError(await api.post(path, { assignedId: `t_${UNKNOWN_ID}` }), 404, "notFound")
    expectError(await api.post(path, { assignedId: `t_${foreignTeam}` }), 404, "notFound")
    // Nothing changed.
    expect(await getConversation(api, seeded.conversationId)).toMatchObject({
      assignedUserId: null,
      assignedInboxTeamId: null,
    })
  })

  it("422 for a missing assignedId or one without the u_ / t_ prefix", async () => {
    const { tenant, api } = await newTenant("p107-assign-422")
    const seeded = await seedConversation(tenant.workspaceId)
    const path = `/v1/conversations/${seeded.conversationId}/assign`
    expectInvalid(await api.post(path, {}), "assignedId")
    expectInvalid(await api.post(path, { assignedId: "x_1" }), "assignedId")
    expectInvalid(await api.post(path, { assignedId: "u_" }), "assignedId")
    expectInvalid(await api.post(path, { assignedId: 5 }), "assignedId")
  })
})

describe("conversations.archive / unarchive", () => {
  it("archive sets archivedAt (hidden from the default list), unarchive clears it", async () => {
    const testStart = Date.now()
    const { tenant, api } = await newTenant("p107-archive")
    const seeded = await seedConversation(tenant.workspaceId, { conversation: { lastActivityAt: ago(HOUR) } })
    const archived = await api.post(`/v1/conversations/${seeded.conversationId}/archive`, {})
    expect(archived.status, archived.text).toBe(200)
    expect(archived.json).toEqual({ success: true })
    const item = await getConversation(api, seeded.conversationId)
    expectIsoNear(item.archivedAt, testStart)
    expect(idsOf(await listConversations(api))).toEqual([])
    expect(idsOf(await listConversations(api, { tags: ["archived"] }))).toEqual([seeded.conversationId])

    // Archiving twice is allowed and moves archivedAt forward.
    expect((await api.post(`/v1/conversations/${seeded.conversationId}/archive`)).status).toBe(200)
    const again = await getConversation(api, seeded.conversationId)
    expect(Date.parse(again.archivedAt as string)).toBeGreaterThanOrEqual(Date.parse(item.archivedAt as string))

    const restored = await api.post(`/v1/conversations/${seeded.conversationId}/unarchive`, {})
    expect(restored.status, restored.text).toBe(200)
    expect(restored.json).toEqual({ success: true })
    expect((await getConversation(api, seeded.conversationId)).archivedAt).toBeNull()
    expect(idsOf(await listConversations(api))).toEqual([seeded.conversationId])
    // Unarchiving a conversation that is not archived is a no-op success.
    expect((await api.post(`/v1/conversations/${seeded.conversationId}/unarchive`)).status).toBe(200)
  })
})

describe("conversations.read / unread", () => {
  it("read sets agentLastReadAt = now; unread with fewer than two incoming messages clears it; conversationUpdated events", async () => {
    const testStart = Date.now()
    const { tenant, api } = await newTenant("p107-read")
    const seeded = await seedConversation(tenant.workspaceId, {
      conversation: { lastActivityAt: ago(HOUR), agentLastReadAt: null },
    })
    expect(idsOf(await listConversations(api, { tags: ["unread"] }))).toEqual([seeded.conversationId])
    const socket = await openWorkspaceSocket(tenant)
    try {
      const read = await api.post(`/v1/conversations/${seeded.conversationId}/read`, {})
      expect(read.status, read.text).toBe(200)
      expect(read.json).toEqual({ success: true })
      const item = await getConversation(api, seeded.conversationId)
      expectIsoNear(item.agentLastReadAt, testStart)
      expect(idsOf(await listConversations(api, { tags: ["unread"] }))).toEqual([])
      const readEvent = await socket.waitFor("conversationUpdated", (e) =>
        JSON.stringify(e.data).includes(seeded.conversationId),
      )
      expect(readEvent.data).toEqual({
        conversationIds: [seeded.conversationId],
        changes: { agentLastReadAt: item.agentLastReadAt },
      })

      const unread = await api.post(`/v1/conversations/${seeded.conversationId}/unread`, {})
      expect(unread.status, unread.text).toBe(200)
      expect(unread.json).toEqual({ success: true })
      expect((await getConversation(api, seeded.conversationId)).agentLastReadAt).toBeNull()
      expect(idsOf(await listConversations(api, { tags: ["unread"] }))).toEqual([seeded.conversationId])
      const unreadEvent = await socket.waitFor(
        "conversationUpdated",
        (e) => (e.data as { changes?: { agentLastReadAt?: unknown } }).changes?.agentLastReadAt === null,
      )
      expect(unreadEvent.data).toEqual({
        conversationIds: [seeded.conversationId],
        changes: { agentLastReadAt: null },
      })
    } finally {
      await socket.close()
    }
  })

  it("unread on a conversation without activity leaves it out of the unread tag", async () => {
    const { tenant, api } = await newTenant("p107-unread-idle")
    const seeded = await seedConversation(tenant.workspaceId, {
      conversation: { lastActivityAt: null, agentLastReadAt: ago(DAY) },
    })
    expect((await api.post(`/v1/conversations/${seeded.conversationId}/unread`)).status).toBe(200)
    expect((await getConversation(api, seeded.conversationId)).agentLastReadAt).toBeNull()
    expect(idsOf(await listConversations(api, { tags: ["unread"] }))).toEqual([])
  })
})

describe("conversations.follow / unfollow", () => {
  it("follow sets followed (followUp tag), unfollow clears it; both are idempotent", async () => {
    const { tenant, api } = await newTenant("p107-follow")
    const seeded = await seedConversation(tenant.workspaceId)
    for (let i = 0; i < 2; i += 1) {
      const followed = await api.post(`/v1/conversations/${seeded.conversationId}/follow`, {})
      expect(followed.status, followed.text).toBe(200)
      expect(followed.json).toEqual({ success: true })
    }
    expect((await getConversation(api, seeded.conversationId)).followed).toBe(true)
    expect(idsOf(await listConversations(api, { tags: ["followUp"] }))).toEqual([seeded.conversationId])
    for (let i = 0; i < 2; i += 1) {
      const unfollowed = await api.post(`/v1/conversations/${seeded.conversationId}/unfollow`, {})
      expect(unfollowed.status, unfollowed.text).toBe(200)
      expect(unfollowed.json).toEqual({ success: true })
    }
    expect((await getConversation(api, seeded.conversationId)).followed).toBe(false)
    expect(idsOf(await listConversations(api, { tags: ["followUp"] }))).toEqual([])
  })
})

describe("conversations.enableBot / disableBot", () => {
  it("disableBot pauses the bot for 24 h (botResumeAt), enableBot clears the pause", async () => {
    const testStart = Date.now()
    const { tenant, api } = await newTenant("p107-bot")
    const seeded = await seedConversation(tenant.workspaceId)
    const disabled = await api.post(`/v1/conversations/${seeded.conversationId}/disable-bot`, {})
    expect(disabled.status, disabled.text).toBe(200)
    expect(disabled.json).toEqual({ success: true })
    const paused = await getConversation(api, seeded.conversationId)
    expect(paused.botEnabled).toBe(false)
    const resumeAt = Date.parse(paused.botResumeAt as string)
    expect(resumeAt).toBeGreaterThanOrEqual(testStart + DAY - 5_000)
    expect(resumeAt).toBeLessThanOrEqual(Date.now() + DAY + 5_000)
    expect(idsOf(await listConversations(api, { botCategory: "human" }))).toEqual([seeded.conversationId])

    const enabled = await api.post(`/v1/conversations/${seeded.conversationId}/enable-bot`, {})
    expect(enabled.status, enabled.text).toBe(200)
    expect(enabled.json).toEqual({ success: true })
    expect(await getConversation(api, seeded.conversationId)).toMatchObject({ botEnabled: true, botResumeAt: null })
    expect(idsOf(await listConversations(api, { botCategory: "human" }))).toEqual([])
    // Enabling an enabled bot is a no-op success.
    expect((await api.post(`/v1/conversations/${seeded.conversationId}/enable-bot`)).status).toBe(200)
  })
})

describe("conversation actions on an unknown, foreign or malformed id", () => {
  it.each(ACTIONS.map((action) => [action] as const))("%s -> 404 notFound / 422", async (action) => {
    const { api } = await newTenant(`p107-404-${action}`)
    const other = await newTenant(`p107-404f-${action}`)
    const foreign = await seedConversation(other.tenant.workspaceId)
    for (const id of [UNKNOWN_ID, foreign.conversationId]) {
      expectError(await api.post(`/v1/conversations/${id}/${action}`, {}), 404, "notFound", {
        message: "Conversation not found",
      })
    }
    expectInvalid(await api.post(`/v1/conversations/abc/${action}`, {}), "id")
    // The foreign conversation did not change.
    const after: ConversationItem = await getConversation(other.api, foreign.conversationId)
    expect(after).toMatchObject({ archivedAt: null, followed: false, botEnabled: true, agentLastReadAt: null })
  })

  it("assign -> 404 for an unknown or foreign conversation (before the assignee check)", async () => {
    const { tenant, api } = await newTenant("p107-404-assign")
    const other = await newTenant("p107-404f-assign")
    const foreign = await seedConversation(other.tenant.workspaceId)
    for (const id of [UNKNOWN_ID, foreign.conversationId]) {
      expectError(
        await api.post(`/v1/conversations/${id}/assign`, { assignedId: `u_${tenant.userId}` }),
        404,
        "notFound",
        { message: "Conversation not found" },
      )
    }
    expect((await getConversation(other.api, foreign.conversationId)).assignedUserId).toBeNull()
  })
})
