// Scores and ranks: leaderboards.participants.adjust / ban / unban / reset /
// history / list, leaderboards.top, leaderboards.auditLog.list,
// leaderboards.resetPoints, the live `stats` counters and
// leaderboards.getViewLink. Scores come from the manual adjust route (the
// automatic scoring rules need a real channel event). Participants are
// fixture contacts of the workspace.

import { describe, expect, it } from "vitest"
import { uniqueId } from "../../src/unique.ts"
import { type Api, expectError, expectInvalid, expectNotFound, expectValidation, ISO, newTenant, SNOWFLAKE, UNKNOWN_ID } from "./helpers.ts"
import {
  activeLeaderboard,
  adjust,
  contact,
  createLeaderboard,
  getLeaderboard,
  type Leaderboard,
  NOT_FOUND,
  transition,
  viewUrlOf,
  zaloBody,
  zaloInbox,
} from "./helpers-leaderboards.ts"

type Participant = {
  contactId: string
  rank: number | null
  score: number
  isBanned: boolean
  breakdown: Record<string, number>
  referrerContactId: string | null
  fullName: string | null
  avatar: string | null
  phoneNumber: string | null
  lastActivityAt: string
  createdAt: string
}
type ParticipantList = { data: Participant[]; pageCount: number; total: number }

const PARTICIPANT_KEYS = [
  "avatar",
  "breakdown",
  "contactId",
  "createdAt",
  "fullName",
  "isBanned",
  "lastActivityAt",
  "phoneNumber",
  "rank",
  "referrerContactId",
  "score",
]

const participants = async (api: Api, id: string, query: Record<string, string | number> = {}) => {
  const response = await api.get<ParticipantList>(`/v1/leaderboards/${id}/participants`, { query })
  expect(response.status, response.text).toBe(200)
  return response.json
}

const top = async (api: Api, id: string, limit?: number) => {
  const response = await api.get<{ data: Array<{ rank: number; contactId: string; score: number; fullName: string | null; avatar: string | null }> }>(
    `/v1/leaderboards/${id}/top`,
    { query: { limit } },
  )
  expect(response.status, response.text).toBe(200)
  return response.json.data
}

const post = (api: Api, id: string, contactId: string, action: string, body: Record<string, unknown> = {}) =>
  api.post(`/v1/leaderboards/${id}/participants/${contactId}/${action}`, body)

/** Three named contacts: Ann (phone), Bob, Cat. */
const players = async (tenant: Parameters<typeof contact>[0]) => {
  const tag = uniqueId()
  return {
    ann: await contact(tenant, { firstName: `Ann${tag}`, phoneNumber: "+84901234567" }),
    bob: await contact(tenant, { firstName: `Bob${tag}` }),
    cat: await contact(tenant, { firstName: `Cat${tag}` }),
    tag,
  }
}

describe("leaderboards.participants.adjust", () => {
  it("adds and subtracts points (never below 0) and returns the new score", async () => {
    const { tenant, api } = await newTenant("p110-lbp-adjust")
    const board = await activeLeaderboard(api, tenant)
    const { ann, bob } = await players(tenant)
    const response = await post(api, board.id, ann, "adjust", { delta: 30, reason: "  bonus  " })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ score: 30 })
    expect(await adjust(api, board.id, ann, -5)).toBe(25)
    expect(await adjust(api, board.id, ann, -100)).toBe(0)
    // A first negative adjust still creates the participant at 0.
    expect(await adjust(api, board.id, bob, -7)).toBe(0)
    const list = await participants(api, board.id)
    expect(list.total).toBe(2)
  })

  it("is refused on a draft (409) and a finished leaderboard (409); paused is fine", async () => {
    const { tenant, api } = await newTenant("p110-lbp-adjust-state")
    const inboxId = await zaloInbox(tenant)
    const draft = await createLeaderboard(api, zaloBody(inboxId))
    const { ann } = await players(tenant)
    expectError(await post(api, draft.id, ann, "adjust", { delta: 1, reason: "r" }), 409, "leaderboardNotPublished", {
      defined: true,
      message: "Publish the leaderboard before editing scores",
    })
    await transition(api, draft.id, "publish")
    await transition(api, draft.id, "pause")
    expect(await adjust(api, draft.id, ann, 4)).toBe(4)
    await transition(api, draft.id, "start")
    await transition(api, draft.id, "finish")
    expectError(await post(api, draft.id, ann, "adjust", { delta: 1, reason: "r" }), 409, "leaderboardFinished", {
      defined: true,
      message: "Scores are locked because the leaderboard has finished",
    })
    expectError(await post(api, draft.id, ann, "ban"), 409, "leaderboardFinished")
    expectError(await post(api, draft.id, ann, "reset"), 409, "leaderboardFinished")
  })

  it("validates the input; unknown leaderboard 404; contact of another workspace 404", async () => {
    const { tenant, api } = await newTenant("p110-lbp-adjust-bad")
    const board = await activeLeaderboard(api, tenant)
    const { ann } = await players(tenant)
    expectInvalid(await post(api, board.id, ann, "adjust", { delta: 0, reason: "r" }), "delta")
    expectInvalid(await post(api, board.id, ann, "adjust", { delta: 1.5, reason: "r" }), "delta")
    expectInvalid(await post(api, board.id, ann, "adjust", { delta: 1 }), "reason")
    expectInvalid(await post(api, board.id, ann, "adjust", { delta: 1, reason: "   " }), "reason")
    expectInvalid(await post(api, board.id, ann, "adjust", { delta: 1, reason: "x".repeat(501) }), "reason")
    expectInvalid(await post(api, board.id, "abc", "adjust", { delta: 1, reason: "r" }), "contactId")
    expectValidation(await post(api, board.id, ann, "adjust", { delta: 90_000_000, reason: "r" }), "Enter a non-zero number of points")
    expectNotFound(await post(api, UNKNOWN_ID, ann, "adjust", { delta: 1, reason: "r" }), NOT_FOUND)
    expectNotFound(await post(api, board.id, UNKNOWN_ID, "adjust", { delta: 1, reason: "r" }), "Contact not found")
    const { tenant: other } = await newTenant("p110-lbp-adjust-bad-o")
    const foreign = await contact(other, { firstName: "Foreign" })
    expectNotFound(await post(api, board.id, foreign, "adjust", { delta: 1, reason: "r" }), "Contact not found")
    expect((await participants(api, board.id)).total).toBe(0)
  })
})

describe("leaderboards.participants.list / top / stats", () => {
  it("ranks by score; lists the per-rule breakdown and a masked phone; top N and stats follow", async () => {
    const { tenant, api } = await newTenant("p110-lbp-list")
    const board = await activeLeaderboard(api, tenant)
    const { ann, bob, cat, tag } = await players(tenant)
    await adjust(api, board.id, ann, 30)
    await adjust(api, board.id, ann, -5)
    await adjust(api, board.id, bob, 50)
    await adjust(api, board.id, cat, 10)

    const list = await participants(api, board.id)
    expect(list).toMatchObject({ total: 3, pageCount: 1 })
    for (const row of list.data) {
      expect(Object.keys(row).sort()).toEqual(PARTICIPANT_KEYS)
      expect(row.lastActivityAt).toMatch(ISO)
      expect(row.createdAt).toMatch(ISO)
    }
    expect(list.data.map(({ lastActivityAt: _l, createdAt: _c, ...row }) => row)).toEqual([
      { contactId: bob, rank: 1, score: 50, isBanned: false, breakdown: { manual: 1 }, referrerContactId: null, fullName: `Bob${tag} Parity`, avatar: null, phoneNumber: null },
      { contactId: ann, rank: 2, score: 25, isBanned: false, breakdown: { manual: 2 }, referrerContactId: null, fullName: `Ann${tag} Parity`, avatar: null, phoneNumber: "••••••••567" },
      { contactId: cat, rank: 3, score: 10, isBanned: false, breakdown: { manual: 1 }, referrerContactId: null, fullName: `Cat${tag} Parity`, avatar: null, phoneNumber: null },
    ])

    expect(await top(api, board.id)).toEqual([
      { rank: 1, contactId: bob, score: 50, fullName: `Bob${tag} Parity`, avatar: null },
      { rank: 2, contactId: ann, score: 25, fullName: `Ann${tag} Parity`, avatar: null },
      { rank: 3, contactId: cat, score: 10, fullName: `Cat${tag} Parity`, avatar: null },
    ])
    expect((await top(api, board.id, 1)).map((entry) => entry.contactId)).toEqual([bob])
    expect((await getLeaderboard(api, board.id)).stats).toEqual({ participants: 3, totalPoints: 85, newToday: 3 })
    const listed = await api.get<{ data: Leaderboard[] }>("/v1/leaderboards")
    expect(listed.json.data[0]?.stats).toEqual({ participants: 3, totalPoints: 85, newToday: 3 })

    // Search by name, phone digits or contact id; filters; paging.
    expect((await participants(api, board.id, { keyword: `ann${tag}` })).data.map((row) => row.contactId)).toEqual([ann])
    expect((await participants(api, board.id, { keyword: "4567" })).data.map((row) => row.contactId)).toEqual([ann])
    expect((await participants(api, board.id, { keyword: cat })).data.map((row) => row.contactId)).toEqual([cat])
    expect((await participants(api, board.id, { filter: "top10" })).total).toBe(3)
    expect((await participants(api, board.id, { filter: "banned" })).data).toEqual([])
    const page = await participants(api, board.id, { page: 2, perPage: 2 })
    expect(page).toMatchObject({ total: 3, pageCount: 2 })
    expect(page.data.map((row) => [row.contactId, row.rank])).toEqual([[cat, 3]])
  })

  it("validates the query; unknown leaderboard 404; a draft lists nobody", async () => {
    const { tenant, api } = await newTenant("p110-lbp-list-bad")
    const draft = await createLeaderboard(api, zaloBody(await zaloInbox(tenant)))
    expect(await participants(api, draft.id)).toEqual({ data: [], total: 0, pageCount: 0 })
    expect(await top(api, draft.id)).toEqual([])
    expectInvalid(await api.get(`/v1/leaderboards/${draft.id}/participants`, { query: { filter: "top5" } }), "filter")
    expectInvalid(await api.get(`/v1/leaderboards/${draft.id}/participants`, { query: { perPage: 51 } }), "perPage")
    expectInvalid(await api.get(`/v1/leaderboards/${draft.id}/top`, { query: { limit: 0 } }), "limit")
    expectInvalid(await api.get(`/v1/leaderboards/${draft.id}/top`, { query: { limit: 501 } }), "limit")
    expectNotFound(await api.get(`/v1/leaderboards/${UNKNOWN_ID}/participants`), NOT_FOUND)
    expectNotFound(await api.get(`/v1/leaderboards/${UNKNOWN_ID}/top`), NOT_FOUND)
  })
})

describe("leaderboards.participants.ban / unban / reset / history and auditLog", () => {
  it("a banned participant leaves the ranking and cannot score; unban restores it", async () => {
    const { tenant, api } = await newTenant("p110-lbp-ban")
    const board = await activeLeaderboard(api, tenant)
    const { ann, bob, cat } = await players(tenant)
    await adjust(api, board.id, ann, 30)
    await adjust(api, board.id, bob, 20)
    await adjust(api, board.id, cat, 10)

    const banned = await post(api, board.id, ann, "ban", { reason: "cheating" })
    expect(banned.status, banned.text).toBe(200)
    expect(banned.json).toEqual({ ok: true })
    const list = await participants(api, board.id)
    expect(list.data.map((row) => [row.contactId, row.rank, row.isBanned, row.score])).toEqual([
      [bob, 1, false, 20],
      [cat, 2, false, 10],
      [ann, null, true, 30],
    ])
    expect((await top(api, board.id)).map((entry) => [entry.rank, entry.contactId])).toEqual([
      [1, bob],
      [2, cat],
    ])
    expect((await participants(api, board.id, { filter: "banned" })).data.map((row) => row.contactId)).toEqual([ann])
    expectError(await post(api, board.id, ann, "adjust", { delta: 5, reason: "r" }), 409, "leaderboardParticipantBanned", {
      defined: true,
      message: "This participant is banned",
    })
    expectError(await post(api, board.id, ann, "reset"), 409, "leaderboardParticipantBanned")

    const unbanned = await post(api, board.id, ann, "unban", {})
    expect(unbanned.status, unbanned.text).toBe(200)
    expect(unbanned.json).toEqual({ ok: true })
    expect((await top(api, board.id)).map((entry) => [entry.rank, entry.contactId])).toEqual([
      [1, ann],
      [2, bob],
      [3, cat],
    ])

    const stranger = await contact(tenant, { firstName: "NoScore" })
    expectNotFound(await post(api, board.id, stranger, "ban"), "Participant not found")
    expectNotFound(await post(api, board.id, stranger, "unban"), "Participant not found")
    expectNotFound(await post(api, UNKNOWN_ID, ann, "ban"), NOT_FOUND)
    expectInvalid(await post(api, board.id, ann, "ban", { reason: "x".repeat(501) }), "reason")
  })

  it("reset sets one score to 0 (no-op at 0); history lists every change newest first", async () => {
    const { tenant, api } = await newTenant("p110-lbp-reset")
    const board = await activeLeaderboard(api, tenant)
    const { ann, bob } = await players(tenant)
    await adjust(api, board.id, ann, 40)
    await adjust(api, board.id, ann, -15)
    const reset = await post(api, board.id, ann, "reset", { reason: "fresh start" })
    expect(reset.status, reset.text).toBe(200)
    expect(reset.json).toEqual({ score: 0 })
    expect((await post(api, board.id, ann, "reset")).json).toEqual({ score: 0 })
    expectNotFound(await post(api, board.id, bob, "reset"), "Participant not found")

    const history = await api.get<{ data: Array<Record<string, unknown>> }>(`/v1/leaderboards/${board.id}/participants/${ann}/history`)
    expect(history.status, history.text).toBe(200)
    expect(history.json.data.map(({ id, createdAt, ...entry }) => {
      expect(id).toMatch(SNOWFLAKE)
      expect(createdAt).toMatch(ISO)
      return entry
    })).toEqual([
      { delta: -25, source: "reset", rule: null, sourceRef: null },
      { delta: -15, source: "manual", rule: null, sourceRef: null },
      { delta: 40, source: "manual", rule: null, sourceRef: null },
    ])
    const row = (await participants(api, board.id)).data[0]
    expect(row).toMatchObject({ contactId: ann, score: 0, breakdown: { manual: 2 } })
    expect((await api.get(`/v1/leaderboards/${board.id}/participants/${bob}/history`)).json).toEqual({ data: [] })
    expectNotFound(await api.get(`/v1/leaderboards/${UNKNOWN_ID}/participants/${ann}/history`), NOT_FOUND)
  })

  it("the audit log records adjustments, bans and resets with reason and contact, newest first, paged", async () => {
    const { tenant, api } = await newTenant("p110-lbp-audit")
    const board = await activeLeaderboard(api, tenant)
    const { ann, bob, tag } = await players(tenant)
    await adjust(api, board.id, ann, 30, "first")
    await adjust(api, board.id, bob, 10, "second")
    await post(api, board.id, bob, "ban", { reason: " spam " })
    await post(api, board.id, bob, "unban")
    await post(api, board.id, ann, "reset", { reason: "redo" })

    const response = await api.get<{ data: Array<Record<string, unknown>>; total: number; pageCount: number }>(
      `/v1/leaderboards/${board.id}/audit-log`,
    )
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ total: 5, pageCount: 1 })
    const entries = response.json.data.map(({ id, createdAt, ...entry }) => {
      expect(id).toMatch(SNOWFLAKE)
      expect(createdAt).toMatch(ISO)
      return entry
    })
    const annRef = { id: ann, fullName: `Ann${tag} Parity`, avatar: null }
    const bobRef = { id: bob, fullName: `Bob${tag} Parity`, avatar: null }
    expect(entries).toEqual([
      { action: "resetOne", delta: -30, reason: "redo", contact: annRef, actor: null },
      { action: "unban", delta: 0, reason: "", contact: bobRef, actor: null },
      { action: "ban", delta: 0, reason: "spam", contact: bobRef, actor: null },
      { action: "adjust", delta: 10, reason: "second", contact: bobRef, actor: null },
      { action: "adjust", delta: 30, reason: "first", contact: annRef, actor: null },
    ])
    const page = await api.get<{ data: Array<{ action: string }>; total: number; pageCount: number }>(
      `/v1/leaderboards/${board.id}/audit-log`,
      { query: { page: 2, perPage: 2 } },
    )
    expect(page.json).toMatchObject({ total: 5, pageCount: 3 })
    expect(page.json.data.map((entry) => entry.action)).toEqual(["ban", "adjust"])
    expectNotFound(await api.get(`/v1/leaderboards/${UNKNOWN_ID}/audit-log`), NOT_FOUND)
  })
})

describe("leaderboards.resetPoints", () => {
  it("sets every score of an active leaderboard to 0 and logs one resetAll entry", async () => {
    const { tenant, api } = await newTenant("p110-lb-reset-all")
    const board = await activeLeaderboard(api, tenant)
    const { ann, bob } = await players(tenant)
    await adjust(api, board.id, ann, 30)
    await adjust(api, board.id, bob, 20)
    const response = await api.post(`/v1/leaderboards/${board.id}/reset-points`, { reason: "season 2" })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ reset: 2 })
    const list = await participants(api, board.id)
    expect(list.data.map((row) => [row.score, row.breakdown])).toEqual([
      [0, {}],
      [0, {}],
    ])
    expect((await getLeaderboard(api, board.id)).stats).toMatchObject({ participants: 2, totalPoints: 0 })
    const audit = await api.get<{ data: Array<Record<string, unknown>> }>(`/v1/leaderboards/${board.id}/audit-log`)
    expect(audit.json.data[0]).toMatchObject({ action: "resetAll", delta: 0, reason: "season 2", contact: null, actor: null })
  })

  it("only an active leaderboard can be reset (409); unknown id 404", async () => {
    const { tenant, api } = await newTenant("p110-lb-reset-all-409")
    const board = await activeLeaderboard(api, tenant)
    await transition(api, board.id, "pause")
    expectError(await api.post(`/v1/leaderboards/${board.id}/reset-points`, {}), 409, "leaderboardActionNotAllowed", {
      defined: true,
      message: "Cannot resetPoints a paused leaderboard",
    })
    const draft = await createLeaderboard(api, zaloBody(await zaloInbox(tenant)))
    expectError(await api.post(`/v1/leaderboards/${draft.id}/reset-points`, {}), 409, "leaderboardActionNotAllowed")
    expectNotFound(await api.post(`/v1/leaderboards/${UNKNOWN_ID}/reset-points`, {}), NOT_FOUND)
    expectInvalid(await api.post(`/v1/leaderboards/${board.id}/reset-points`, { reason: "x".repeat(501) }), "reason")
  })
})

describe("leaderboards.getViewLink", () => {
  it("without contactId the link holds the flow variable; with a contact it is signed for that contact", async () => {
    const { tenant, api } = await newTenant("p110-lb-view")
    const inboxId = await zaloInbox(tenant)
    const board = await createLeaderboard(api, zaloBody(inboxId))
    const plain = await api.get(`/v1/leaderboards/${board.id}/view-link`)
    expect(plain.status, plain.text).toBe(200)
    expect(plain.json).toEqual({ url: viewUrlOf(board.id), personalized: false })

    const withInbox = await contact(tenant, { firstName: "Viewer", inboxId })
    const signed = await api.get<{ url: string; personalized: boolean }>(`/v1/leaderboards/${board.id}/view-link`, {
      query: { contactId: withInbox },
    })
    expect(signed.status, signed.text).toBe(200)
    expect(signed.json.personalized).toBe(true)
    const url = new URL(signed.json.url)
    expect(`${url.origin}${url.pathname}`).toBe("http://app/leaderboards")
    expect(url.searchParams.get("leaderboardId")).toBe(board.id)
    const token = url.searchParams.get("token") ?? ""
    expect(token.length).toBeGreaterThan(20)
    expect(token).not.toContain("{{")

    // A contact with no channel identity gets the unsigned link.
    const noInbox = await contact(tenant, { firstName: "Ghost" })
    expect((await api.get(`/v1/leaderboards/${board.id}/view-link`, { query: { contactId: noInbox } })).json).toEqual({
      url: viewUrlOf(board.id),
      personalized: false,
    })
    expectNotFound(await api.get(`/v1/leaderboards/${UNKNOWN_ID}/view-link`), NOT_FOUND)
  })
})
