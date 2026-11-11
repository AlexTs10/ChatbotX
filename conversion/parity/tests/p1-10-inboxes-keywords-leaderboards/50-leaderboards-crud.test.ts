// leaderboards.create / get / list / update (PUT, full replace) / delete,
// defaultRules, telegramChats and the status machine publish / pause / start /
// finish (draft -> active <-> paused -> finished), including the scheduler
// that finishes a leaderboard at its end time (`finishOverdueLeaderboards`,
// every minute). Host inboxes and Telegram groups come from the P1.10 fixture
// extension.

import { describe, expect, it } from "vitest"
import { uniqueId } from "../../src/unique.ts"
import { expectError, expectInvalid, expectNotFound, expectValidation, ext, ISO, newTenant, SNOWFLAKE, UNKNOWN_ID, uniqueName, wait, createPublishedFlow } from "./helpers.ts"
import {
  ACTIONS,
  activeLeaderboard,
  createLeaderboard,
  DAY,
  getLeaderboard,
  HOUR,
  isoFromNow,
  LEADERBOARD_KEYS,
  type Leaderboard,
  NOT_FOUND,
  rule,
  transition,
  viewUrlOf,
  zaloBody,
  zaloInbox,
} from "./helpers-leaderboards.ts"

const notAllowed = (action: string, status: string) => ({
  defined: true,
  message: `Cannot ${action} a ${status} leaderboard`,
})

describe("leaderboards.create", () => {
  it("creates a draft with defaults filled in (201)", async () => {
    const { tenant, api } = await newTenant("p110-lb-create")
    const inboxId = await zaloInbox(tenant)
    const startsAt = isoFromNow(-HOUR)
    const endsAt = isoFromNow(7 * DAY)
    const name = uniqueName("Board")
    const response = await api.post<Leaderboard>("/v1/leaderboards", {
      name: `  ${name}  `,
      startsAt,
      endsAt,
      platform: "zalo",
      inboxId,
      rules: { message: { enabled: true, points: 10 } },
    })
    expect(response.status, response.text).toBe(201)
    expect(Object.keys(response.json).sort()).toEqual(LEADERBOARD_KEYS)
    const id = response.json.id
    expect(response.json).toEqual({
      id: expect.stringMatching(SNOWFLAKE),
      name,
      description: "",
      coverImageUrl: "",
      startsAt,
      endsAt,
      status: "draft",
      platform: "zalo",
      inboxId,
      minigameId: null,
      scope: { postScope: "all", postIds: [], telegramChatIds: [] },
      rules: { message: rule({ enabled: true, points: 10 }) },
      followersOnly: false,
      tagIds: [],
      resetTime: "22:00",
      cooldownSeconds: 0,
      referralWelcomeFlowId: null,
      rewardSettings: { flowId: null, topN: 3, autoOnFinish: false },
      rewardsTriggeredAt: null,
      finishedAt: null,
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
      stats: { participants: 0, totalPoints: 0, newToday: 0 },
      availableActions: ACTIONS.draft,
      viewUrl: viewUrlOf(id),
    })
    expect(await getLeaderboard(api, id)).toEqual(response.json)
  })

  it("stores every configurable field (tags deduplicated, flows referenced)", async () => {
    const { tenant, api } = await newTenant("p110-lb-create-full")
    const inboxId = await zaloInbox(tenant)
    const tag = await api.post<{ id: string }>("/v1/tags", { name: uniqueName("VIP") })
    expect(tag.status, tag.text).toBe(201)
    const rewardFlow = await createPublishedFlow(api, uniqueName("Reward"), "You won")
    const welcomeFlow = await createPublishedFlow(api, uniqueName("Welcome"), "Welcome friend")
    const board = await createLeaderboard(
      api,
      zaloBody(inboxId, {
        description: "Win prizes",
        coverImageUrl: "https://cdn.parity.test/cover.png",
        rules: {
          follow: { enabled: true, points: 5 },
          message: { enabled: true, points: 10, maxPerDay: 2 },
          referral: { enabled: true, points: 50, maxPerDay: 10 },
        },
        followersOnly: true,
        tagIds: [tag.json.id, tag.json.id],
        resetTime: "05:30",
        cooldownSeconds: 60,
        referralWelcomeFlowId: welcomeFlow,
        rewardSettings: { flowId: rewardFlow, topN: 5, autoOnFinish: true },
      }),
    )
    expect(board).toMatchObject({
      description: "Win prizes",
      coverImageUrl: "https://cdn.parity.test/cover.png",
      rules: {
        follow: rule({ enabled: true, points: 5 }),
        message: rule({ enabled: true, points: 10, maxPerDay: 2 }),
        referral: rule({ enabled: true, points: 50, maxPerDay: 10 }),
      },
      followersOnly: true,
      tagIds: [tag.json.id],
      resetTime: "05:30",
      cooldownSeconds: 60,
      referralWelcomeFlowId: welcomeFlow,
      rewardSettings: { flowId: rewardFlow, topN: 5, autoOnFinish: true },
    })
  })

  it("referral on without a welcome flow creates and links a published welcome flow", async () => {
    const { tenant, api } = await newTenant("p110-lb-create-welcome")
    const inboxId = await zaloInbox(tenant)
    const name = uniqueName("Referral")
    const board = await createLeaderboard(
      api,
      zaloBody(inboxId, { name, rules: { referral: { enabled: true, points: 50 } } }),
    )
    expect(board.referralWelcomeFlowId).toMatch(SNOWFLAKE)
    const flow = await api.get<{ id: string; name: string; currentVersionId: string | null }>(
      `/v1/flows/${board.referralWelcomeFlowId}`,
    )
    expect(flow.status, flow.text).toBe(200)
    expect(flow.json).toMatchObject({ name: `Leaderboard welcome - ${name}`, currentVersionId: expect.stringMatching(SNOWFLAKE) })
  })

  it("checks the configuration (422 validation)", async () => {
    const { tenant, api } = await newTenant("p110-lb-create-biz")
    const inboxId = await zaloInbox(tenant)
    const telegram = (await (await ext()).createInbox({ workspaceId: tenant.workspaceId, channel: "telegram", name: "TG" }))
      .inboxId
    const { tenant: other } = await newTenant("p110-lb-create-biz-o")
    const foreignInbox = await zaloInbox(other)
    const start = Date.now() - HOUR
    const cases: Array<[Record<string, unknown>, string]> = [
      [{ inboxId: null }, "Choose a channel"],
      [{ inboxId: telegram }, "Choose a connected zalo channel"],
      [{ inboxId: foreignInbox }, "Choose a connected zalo channel"],
      [{ inboxId: UNKNOWN_ID }, "Choose a connected zalo channel"],
      [{ startsAt: isoFromNow(0), endsAt: isoFromNow(-60_000) }, "End time must be after start time"],
      [
        { startsAt: new Date(start).toISOString(), endsAt: new Date(start + 100_000_000_000).toISOString() },
        "A leaderboard can run for at most 3 years",
      ],
      [{ rules: { comments: { enabled: true, points: 5 } } }, 'Rule "comments" is not available on zalo'],
      [{ rules: { message: { enabled: true, points: 0 } } }, "Enter the points this rule awards"],
      [{ rules: { message: { enabled: true, points: 5, minChars: 3 } } }, '"minChars" only applies to the comments rule'],
      [{ scope: { postScope: "specific", postIds: ["p1"] } }, "Scoring specific posts is not available on zalo"],
      [{ referralWelcomeFlowId: UNKNOWN_ID }, "Flow not found"],
      [{ rewardSettings: { flowId: UNKNOWN_ID } }, "Flow not found"],
      [{ tagIds: [UNKNOWN_ID] }, "Tag not found"],
      [{ platform: "gamification", rules: { minigameWin: { enabled: true, points: 5 } } }, "Choose a minigame"],
      [
        { platform: "telegram", inboxId: telegram, rules: { joinGroup: { enabled: true, points: 5 } } },
        "Choose at least one Telegram group or channel",
      ],
    ]
    for (const [overrides, message] of cases) {
      expectValidation(await api.post("/v1/leaderboards", zaloBody(inboxId, overrides)), message)
    }
    const listed = await api.get<{ total: number }>("/v1/leaderboards")
    expect(listed.json.total).toBe(0)
  })

  it("validates the body (422 invalidRequestData)", async () => {
    const { tenant, api } = await newTenant("p110-lb-create-invalid")
    const inboxId = await zaloInbox(tenant)
    const body = (overrides: Record<string, unknown>) => zaloBody(inboxId, overrides)
    expectInvalid(await api.post("/v1/leaderboards", body({ name: "   " })), "name")
    expectInvalid(await api.post("/v1/leaderboards", body({ name: "x".repeat(151) })), "name")
    expectInvalid(await api.post("/v1/leaderboards", body({ startsAt: "not a date" })), "startsAt")
    expectInvalid(await api.post("/v1/leaderboards", body({ platform: "fax" })), "platform")
    expectInvalid(await api.post("/v1/leaderboards", body({ rules: undefined })), "rules")
    expectInvalid(await api.post("/v1/leaderboards", body({ rules: { message: { enabled: true, points: -1 } } })), "rules.message.points")
    expectInvalid(await api.post("/v1/leaderboards", body({ resetTime: "24:00" })), "resetTime")
    expectInvalid(await api.post("/v1/leaderboards", body({ cooldownSeconds: 86_401 })), "cooldownSeconds")
    expectInvalid(await api.post("/v1/leaderboards", body({ inboxId: "abc" })), "inboxId")
    expectInvalid(await api.post("/v1/leaderboards", body({ rewardSettings: { topN: 0 } })), "rewardSettings.topN")
    expectInvalid(await api.post("/v1/leaderboards", body({ description: "x".repeat(5001) })), "description")
  })
})

describe("leaderboards.get / list", () => {
  it("get is 404 for unknown and foreign ids", async () => {
    const { tenant, api } = await newTenant("p110-lb-get")
    const board = await createLeaderboard(api, zaloBody(await zaloInbox(tenant)))
    expectNotFound(await api.get(`/v1/leaderboards/${UNKNOWN_ID}`), NOT_FOUND)
    const { api: otherApi } = await newTenant("p110-lb-get-o")
    expectNotFound(await otherApi.get(`/v1/leaderboards/${board.id}`), NOT_FOUND)
    expectInvalid(await api.get("/v1/leaderboards/abc"), "id")
  })

  it("lists newest first with filters, paging, totals and per-status summary of the whole workspace", async () => {
    const { tenant, api } = await newTenant("p110-lb-list")
    const empty = await api.get("/v1/leaderboards")
    expect(empty.status, empty.text).toBe(200)
    expect(empty.json).toEqual({
      data: [],
      pageCount: 0,
      total: 0,
      summary: { total: 0, active: 0, paused: 0, finished: 0, draft: 0 },
    })

    const inboxId = await zaloInbox(tenant)
    const tag = uniqueId()
    const draft = await createLeaderboard(api, zaloBody(inboxId, { name: `Spring ${tag}` }))
    const active = await transition(api, (await createLeaderboard(api, zaloBody(inboxId, { name: `Summer ${tag}` }))).id, "publish")
    const paused = await transition(
      api,
      (await transition(api, (await createLeaderboard(api, zaloBody(inboxId, { name: `Autumn ${tag}` }))).id, "publish")).id,
      "pause",
    )
    const summary = { total: 3, active: 1, paused: 1, finished: 0, draft: 1 }

    const all = await api.get<{ data: Leaderboard[]; pageCount: number; total: number; summary: unknown }>("/v1/leaderboards")
    expect(all.json.data.map((board) => board.id)).toEqual([paused.id, active.id, draft.id])
    expect(all.json).toMatchObject({ pageCount: 1, total: 3, summary })
    expect(all.json.data[1]).toEqual({ ...active, updatedAt: expect.stringMatching(ISO) })

    const byStatus = await api.get<{ data: Leaderboard[]; total: number; summary: unknown }>("/v1/leaderboards", {
      query: { status: "active" },
    })
    expect(byStatus.json.data.map((board) => board.id)).toEqual([active.id])
    expect(byStatus.json).toMatchObject({ total: 1, summary })

    const byKeyword = await api.get<{ data: Leaderboard[] }>("/v1/leaderboards", { query: { keyword: "  summer " } })
    expect(byKeyword.json.data.map((board) => board.id)).toEqual([active.id])
    const byPlatform = await api.get<{ data: Leaderboard[] }>("/v1/leaderboards", { query: { platform: "telegram" } })
    expect(byPlatform.json.data).toEqual([])

    const page = await api.get<{ data: Leaderboard[]; pageCount: number; total: number }>("/v1/leaderboards", {
      query: { page: 2, perPage: 2 },
    })
    expect(page.json.data.map((board) => board.id)).toEqual([draft.id])
    expect(page.json).toMatchObject({ pageCount: 2, total: 3 })

    expectInvalid(await api.get("/v1/leaderboards", { query: { status: "done" } }), "status")
    expectInvalid(await api.get("/v1/leaderboards", { query: { perPage: 51 } }), "perPage")
  })
})

describe("leaderboards.defaultRules", () => {
  it("returns the platform's rules with the spec defaults", async () => {
    const { api } = await newTenant("p110-lb-defaults")
    const zalo = await api.get("/v1/leaderboards/default-rules", { query: { platform: "zalo" } })
    expect(zalo.status, zalo.text).toBe(200)
    expect(zalo.json).toEqual({
      rules: {
        follow: rule(),
        message: rule({ maxPerDay: 2 }),
        referral: rule({ enabled: true, points: 50, maxPerDay: 10 }),
      },
    })
    const messenger = await api.get("/v1/leaderboards/default-rules", { query: { platform: "messenger" } })
    expect(messenger.json).toEqual({
      rules: {
        comments: rule({ enabled: true, points: 30, minChars: 30, bonusMinutes: 60 }),
        liveComments: rule({ enabled: true, points: 300 }),
        keywords: rule(),
        referral: rule({ enabled: true, points: 50, maxPerDay: 10 }),
      },
    })
    const email = await api.get("/v1/leaderboards/default-rules", { query: { platform: "email" } })
    expect(email.json).toEqual({ rules: { emailOpen: rule(), emailClick: rule({ maxPerDay: 2 }) } })
    expectInvalid(await api.get("/v1/leaderboards/default-rules"), "platform")
    expectInvalid(await api.get("/v1/leaderboards/default-rules", { query: { platform: "fax" } }), "platform")
  })
})

describe("leaderboards.telegramChats", () => {
  it("lists the groups/channels where the inbox's bot is an admin, by title", async () => {
    const { tenant, api } = await newTenant("p110-lb-tg")
    const fx = await ext()
    const { inboxId } = await fx.createInbox({ workspaceId: tenant.workspaceId, channel: "telegram", name: "TG bot" })
    const { inboxId: otherInbox } = await fx.createInbox({ workspaceId: tenant.workspaceId, channel: "telegram", name: "TG 2" })
    const chat = (chatId: string, title: string | null, type: "group" | "supergroup" | "channel", botStatus: "administrator" | "creator" | "member" | "left") =>
      fx.createTelegramChat({ workspaceId: tenant.workspaceId, inboxId, chatId, title, type, botStatus })
    await chat("-1003", "Bravo group", "supergroup", "administrator")
    await chat("-1001", "Alpha channel", "channel", "creator")
    await chat("-1002", "Charlie", "group", "member")
    await chat("-1004", "Delta", "group", "left")
    await fx.createTelegramChat({
      workspaceId: tenant.workspaceId,
      inboxId: otherInbox,
      chatId: "-1005",
      title: "Other inbox",
      type: "group",
      botStatus: "administrator",
    })
    const response = await api.get("/v1/leaderboards/telegram-chats", { query: { inboxId } })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      data: [
        { chatId: "-1001", title: "Alpha channel", type: "channel" },
        { chatId: "-1003", title: "Bravo group", type: "supergroup" },
      ],
    })
    const { api: otherApi } = await newTenant("p110-lb-tg-o")
    expect((await otherApi.get("/v1/leaderboards/telegram-chats", { query: { inboxId } })).json).toEqual({ data: [] })
    expect((await api.get("/v1/leaderboards/telegram-chats", { query: { inboxId: UNKNOWN_ID } })).json).toEqual({ data: [] })
    expectInvalid(await api.get("/v1/leaderboards/telegram-chats"), "inboxId")
  })
})

describe("leaderboards.update", () => {
  it("replaces a draft's configuration, including platform and channel", async () => {
    const { tenant, api } = await newTenant("p110-lb-update")
    const board = await createLeaderboard(api, zaloBody(await zaloInbox(tenant)))
    const telegram = (await (await ext()).createInbox({ workspaceId: tenant.workspaceId, channel: "telegram", name: "TG" }))
      .inboxId
    const startsAt = isoFromNow(HOUR)
    const response = await api.put<Leaderboard>(`/v1/leaderboards/${board.id}`, {
      name: "Renamed",
      startsAt,
      endsAt: board.endsAt,
      platform: "telegram",
      inboxId: telegram,
      scope: { telegramChatIds: ["-100"] },
      rules: { joinGroup: { enabled: true, points: 3 } },
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      ...board,
      name: "Renamed",
      startsAt,
      platform: "telegram",
      inboxId: telegram,
      scope: { postScope: "all", postIds: [], telegramChatIds: ["-100"] },
      rules: { joinGroup: rule({ enabled: true, points: 3 }) },
      updatedAt: expect.stringMatching(ISO),
    })
  })

  it("after publishing: name/rules may change, platform/channel/start may not; finished cannot be edited", async () => {
    const { tenant, api } = await newTenant("p110-lb-update-live")
    const inboxId = await zaloInbox(tenant)
    const body = zaloBody(inboxId)
    const board = await transition(api, (await createLeaderboard(api, body)).id, "publish")
    const ok = await api.put<Leaderboard>(`/v1/leaderboards/${board.id}`, {
      ...body,
      name: "Live rename",
      rules: { message: { enabled: true, points: 20 } },
    })
    expect(ok.status, ok.text).toBe(200)
    expect(ok.json).toMatchObject({ name: "Live rename", status: "active", rules: { message: rule({ enabled: true, points: 20 }) } })

    const otherInbox = await zaloInbox(tenant)
    expectValidation(
      await api.put(`/v1/leaderboards/${board.id}`, { ...body, inboxId: otherInbox }),
      "The platform and channel cannot change after publishing",
    )
    expectValidation(
      await api.put(`/v1/leaderboards/${board.id}`, { ...body, startsAt: isoFromNow(-2 * HOUR) }),
      "The start time cannot change after publishing",
    )
    await transition(api, board.id, "finish")
    expectError(await api.put(`/v1/leaderboards/${board.id}`, body), 409, "leaderboardActionNotAllowed", notAllowed("edit", "finished"))
  })

  it("404 for an unknown id; the body is validated like create", async () => {
    const { tenant, api } = await newTenant("p110-lb-update-404")
    const inboxId = await zaloInbox(tenant)
    expectNotFound(await api.put(`/v1/leaderboards/${UNKNOWN_ID}`, zaloBody(inboxId)), NOT_FOUND)
    const board = await createLeaderboard(api, zaloBody(inboxId))
    expectInvalid(await api.put(`/v1/leaderboards/${board.id}`, zaloBody(inboxId, { name: "" })), "name")
    expectValidation(
      await api.put(`/v1/leaderboards/${board.id}`, zaloBody(inboxId, { rules: { comments: { enabled: true, points: 1 } } })),
      'Rule "comments" is not available on zalo',
    )
  })
})

describe("leaderboards status machine", () => {
  it("publish -> pause -> start -> finish, with the allowed actions of each status", async () => {
    const { tenant, api } = await newTenant("p110-lb-status")
    const board = await createLeaderboard(api, zaloBody(await zaloInbox(tenant)))
    const active = await transition(api, board.id, "publish")
    expect(active).toMatchObject({ status: "active", availableActions: ACTIONS.active, finishedAt: null })
    const paused = await transition(api, board.id, "pause")
    expect(paused).toMatchObject({ status: "paused", availableActions: ACTIONS.paused })
    const resumed = await transition(api, board.id, "start")
    expect(resumed).toMatchObject({ status: "active", availableActions: ACTIONS.active })
    const finished = await transition(api, board.id, "finish")
    expect(finished).toMatchObject({ status: "finished", availableActions: ACTIONS.finished, rewardsTriggeredAt: null })
    expect(finished.finishedAt).toMatch(ISO)
    expect(Object.keys(finished).sort()).toEqual(LEADERBOARD_KEYS)
    expect(await getLeaderboard(api, board.id)).toEqual(finished)
  })

  it("refuses transitions the status does not allow (409 leaderboardActionNotAllowed)", async () => {
    const { tenant, api } = await newTenant("p110-lb-status-409")
    const draft = await createLeaderboard(api, zaloBody(await zaloInbox(tenant)))
    const post = (id: string, action: string) => api.post(`/v1/leaderboards/${id}/${action}`, {})
    expectError(await post(draft.id, "pause"), 409, "leaderboardActionNotAllowed", notAllowed("pause", "draft"))
    expectError(await post(draft.id, "start"), 409, "leaderboardActionNotAllowed", notAllowed("start", "draft"))
    expectError(await post(draft.id, "finish"), 409, "leaderboardActionNotAllowed", notAllowed("finish", "draft"))
    await transition(api, draft.id, "publish")
    expectError(await post(draft.id, "publish"), 409, "leaderboardActionNotAllowed", notAllowed("publish", "active"))
    expectError(await post(draft.id, "start"), 409, "leaderboardActionNotAllowed", notAllowed("start", "active"))
    await transition(api, draft.id, "finish")
    for (const action of ["publish", "pause", "start", "finish"]) {
      expectError(await post(draft.id, action), 409, "leaderboardActionNotAllowed", notAllowed(action, "finished"))
    }
    for (const action of ["publish", "pause", "start", "finish"]) {
      expectNotFound(await post(UNKNOWN_ID, action), NOT_FOUND)
    }
  })

  it("publish needs an enabled rule and an end time in the future (422 validation)", async () => {
    const { tenant, api } = await newTenant("p110-lb-publish-biz")
    const inboxId = await zaloInbox(tenant)
    const noRule = await createLeaderboard(api, zaloBody(inboxId, { rules: { message: { enabled: false } } }))
    expectValidation(await api.post(`/v1/leaderboards/${noRule.id}/publish`, {}), "Enable at least one scoring rule")
    const past = await createLeaderboard(api, zaloBody(inboxId, { startsAt: isoFromNow(-2 * HOUR), endsAt: isoFromNow(-HOUR) }))
    expectValidation(await api.post(`/v1/leaderboards/${past.id}/publish`, {}), "The end time has already passed")
    expect((await getLeaderboard(api, past.id)).status).toBe("draft")
  })

  it(
    "the scheduler finishes an active leaderboard once its end time passed",
    async () => {
      const { tenant, api } = await newTenant("p110-lb-overdue")
      const board = await activeLeaderboard(api, tenant, { startsAt: isoFromNow(-HOUR), endsAt: isoFromNow(15_000) })
      expect(board.status).toBe("active")
      const finished = await wait(async () => {
        const current = await getLeaderboard(api, board.id)
        expect(current.status).toBe("finished")
        return current
      }, 120_000)
      expect(finished.finishedAt).toMatch(ISO)
      expect(Date.parse(finished.finishedAt as string)).toBeGreaterThanOrEqual(Date.parse(board.endsAt))
      expect(finished.availableActions).toEqual(ACTIONS.finished)
    },
    150_000,
  )
})

describe("leaderboards.delete", () => {
  it("deletes draft, paused and finished leaderboards; an active one must be paused first", async () => {
    const { tenant, api } = await newTenant("p110-lb-delete")
    const inboxId = await zaloInbox(tenant)
    const draft = await createLeaderboard(api, zaloBody(inboxId))
    const response = await api.delete(`/v1/leaderboards/${draft.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectNotFound(await api.get(`/v1/leaderboards/${draft.id}`), NOT_FOUND)
    expectNotFound(await api.delete(`/v1/leaderboards/${draft.id}`), NOT_FOUND)

    const active = await transition(api, (await createLeaderboard(api, zaloBody(inboxId))).id, "publish")
    expectError(await api.delete(`/v1/leaderboards/${active.id}`), 409, "leaderboardActionNotAllowed", notAllowed("delete", "active"))
    await transition(api, active.id, "pause")
    expect((await api.delete(`/v1/leaderboards/${active.id}`)).status).toBe(204)

    const finished = await transition(api, (await createLeaderboard(api, zaloBody(inboxId))).id, "publish")
    await transition(api, finished.id, "finish")
    expect((await api.delete(`/v1/leaderboards/${finished.id}`)).status).toBe(204)

    const list = await api.get<{ data: unknown[]; total: number; summary: unknown }>("/v1/leaderboards")
    expect(list.json).toEqual({ data: [], pageCount: 0, total: 0, summary: { total: 0, active: 0, paused: 0, finished: 0, draft: 0 } })
    expectNotFound(await api.delete(`/v1/leaderboards/${UNKNOWN_ID}`), NOT_FOUND)
  })
})
