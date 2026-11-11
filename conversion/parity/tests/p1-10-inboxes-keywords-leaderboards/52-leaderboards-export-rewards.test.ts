// leaderboards.export / getExport (the worker writes a CSV of every
// participant to storage; the test downloads it through the presigned URL) and
// leaderboards.triggerRewards (starts the reward flow for the top N of a
// finished leaderboard; the observable effect is the API channel callback of
// each winner who has a conversation).

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  type Api,
  callbackTexts,
  createApiChannel,
  createPublishedFlow,
  expectError,
  expectNotFound,
  extraToken,
  inboundContact,
  ISO,
  newTenant,
  UNKNOWN_ID,
  uniqueName,
  wait,
} from "./helpers.ts"
import {
  ACTIONS,
  activeLeaderboard,
  adjust,
  contact,
  createLeaderboard,
  getLeaderboard,
  NOT_FOUND,
  transition,
  zaloBody,
  zaloInbox,
} from "./helpers-leaderboards.ts"

type ExportFile = {
  fileId: string
  fileName: string | null
  status: string
  totalRecords: number
  downloadUrl: string | null
}

const startExport = async (api: Api, id: string) => {
  const response = await api.post<{ fileId: string; fileName: string; status: string }>(`/v1/leaderboards/${id}/export`, {})
  expect(response.status, response.text).toBe(201)
  return response.json
}

const finishedExport = (api: Api, fileId: string) =>
  wait(async () => {
    const response = await api.get<ExportFile>(`/v1/leaderboards/exports/${fileId}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json.status).not.toBe("pending")
    return response.json
  })

const download = async (url: string) => {
  const response = await fetch(url)
  expect(response.status).toBe(200)
  const bytes = new Uint8Array(await response.arrayBuffer())
  return { contentType: response.headers.get("content-type"), bytes, text: Buffer.from(bytes).toString("utf8") }
}

const ISO_CELL = /^"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z"$/

/** CSV lines (BOM stripped) split into raw cells; the two time cells are checked and replaced. */
const csvRows = (text: string): string[][] =>
  text
    .slice(1)
    .split("\n")
    .filter((line) => line.length > 0)
    .map((line, index) => {
      const cells = line.split(",")
      if (index === 0) {
        return cells
      }
      const times = cells.slice(-2)
      for (const cell of times) {
        expect(cell).toMatch(ISO_CELL)
      }
      return [...cells.slice(0, -2), "<ts>", "<ts>"]
    })

describe("leaderboards.export / getExport", () => {
  it("queues a CSV of every participant in rank order (banned last, no rank) with phone and email", async () => {
    const { tenant, api } = await newTenant("p110-lb-export")
    const board = await activeLeaderboard(api, tenant)
    const tag = uniqueId()
    const ann = await contact(tenant, { firstName: `Ann${tag}`, phoneNumber: "+84901234567" })
    const bob = await contact(tenant, { firstName: `=Bob${tag}` })
    const cat = await contact(tenant, { firstName: `Cat${tag}` })
    await adjust(api, board.id, ann, 20)
    await adjust(api, board.id, bob, 30)
    await adjust(api, board.id, cat, 5)
    expect((await api.post(`/v1/leaderboards/${board.id}/participants/${cat}/ban`, {})).status).toBe(200)

    const started = await startExport(api, board.id)
    expect(started.status).toBe("pending")
    expect(started.fileName).toMatch(new RegExp(`^leaderboard-${board.id}-\\d{4}-\\d{2}-\\d{2}T\\d{2}-\\d{2}-\\d{2}-\\d{3}Z\\.csv$`))
    expect(started.fileId).toMatch(/^\d{15,20}$/)

    const file = await finishedExport(api, started.fileId)
    expect(file).toMatchObject({ fileId: started.fileId, fileName: started.fileName, status: "uploaded", totalRecords: 3 })
    expect(file.downloadUrl).toContain(`/workspaces/${tenant.workspaceId}/exports/${started.fileName}`)
    const csv = await download(file.downloadUrl as string)
    expect(csv.contentType).toBe("text/csv; charset=utf-8")
    expect([...csv.bytes.slice(0, 3)]).toEqual([0xef, 0xbb, 0xbf])
    expect(csvRows(csv.text)).toEqual([
      ['"Rank"', '"Contact ID"', '"Name"', '"Phone"', '"Email"', '"Score"', '"Banned"', '"Breakdown"', '"Joined at"', '"Last activity"'],
      ['"1"', `"${bob}"`, `"'=Bob${tag} Parity"`, '""', '""', '"30"', '"no"', '"manual: 1"', "<ts>", "<ts>"],
      ['"2"', `"${ann}"`, `"Ann${tag} Parity"`, `"'+84901234567"`, '""', '"20"', '"no"', '"manual: 1"', "<ts>", "<ts>"],
      ['""', `"${cat}"`, `"Cat${tag} Parity"`, '""', '""', '"5"', '"yes"', '"manual: 1"', "<ts>", "<ts>"],
    ])
  })

  it("a token without the contacts scope gets no phone or email columns", async () => {
    const { tenant, api } = await newTenant("p110-lb-export-scoped")
    const board = await activeLeaderboard(api, tenant)
    const ann = await contact(tenant, { firstName: "Scoped", phoneNumber: "+84901234567" })
    await adjust(api, board.id, ann, 3)
    const scoped = await extraToken(tenant, { scopes: ["leaderboards"] })
    const started = await startExport(scoped, board.id)
    const file = await finishedExport(scoped, started.fileId)
    expect(file.status).toBe("uploaded")
    const rows = csvRows((await download(file.downloadUrl as string)).text)
    expect(rows[0]).toEqual(['"Rank"', '"Contact ID"', '"Name"', '"Score"', '"Banned"', '"Breakdown"', '"Joined at"', '"Last activity"'])
    expect(rows[1]).toEqual(['"1"', `"${ann}"`, '"Scoped Parity"', '"3"', '"no"', '"manual: 1"', "<ts>", "<ts>"])
  })

  it("an empty paused leaderboard exports a header-only CSV; a draft cannot be exported (409)", async () => {
    const { tenant, api } = await newTenant("p110-lb-export-state")
    const board = await activeLeaderboard(api, tenant)
    await transition(api, board.id, "pause")
    const file = await finishedExport(api, (await startExport(api, board.id)).fileId)
    expect(file).toMatchObject({ status: "uploaded", totalRecords: 0 })
    expect(csvRows((await download(file.downloadUrl as string)).text)).toHaveLength(1)

    const draft = await createLeaderboard(api, zaloBody(await zaloInbox(tenant)))
    expectError(await api.post(`/v1/leaderboards/${draft.id}/export`, {}), 409, "leaderboardActionNotAllowed", {
      defined: true,
      message: "Cannot export a draft leaderboard",
    })
    expectNotFound(await api.post(`/v1/leaderboards/${UNKNOWN_ID}/export`, {}), NOT_FOUND)
  })

  it("getExport is 404 for an unknown or foreign file", async () => {
    const { tenant, api } = await newTenant("p110-lb-export-404")
    const board = await activeLeaderboard(api, tenant)
    const started = await startExport(api, board.id)
    expectNotFound(await api.get(`/v1/leaderboards/exports/${UNKNOWN_ID}`), "Export file not found")
    const { api: otherApi } = await newTenant("p110-lb-export-404-o")
    expectNotFound(await otherApi.get(`/v1/leaderboards/exports/${started.fileId}`), "Export file not found")
    await finishedExport(api, started.fileId)
  })
})

describe("leaderboards.triggerRewards", () => {
  it(
    "starts the reward flow for the top N once; winners without a conversation are skipped",
    async () => {
      const { tenant, api } = await newTenant("p110-lb-rewards")
      const channel = await createApiChannel(api)
      const first = await inboundContact(api, channel)
      const third = await inboundContact(api, channel)
      const noConversation = await contact(tenant, { firstName: "Lonely", conversation: false })
      const rewardText = `You won ${uniqueId()}`
      const rewardFlow = await createPublishedFlow(api, uniqueName("Reward"), rewardText)
      const board = await activeLeaderboard(api, tenant, { rewardSettings: { flowId: rewardFlow, topN: 2 } })
      await adjust(api, board.id, first.contactId, 30)
      await adjust(api, board.id, noConversation, 25)
      await adjust(api, board.id, third.contactId, 20)

      expectError(await api.post(`/v1/leaderboards/${board.id}/trigger-rewards`, {}), 409, "leaderboardNotFinished", {
        defined: true,
        message: "Rewards can only be sent after the leaderboard finishes",
      })
      await transition(api, board.id, "finish")

      const since = await fake.cursor()
      const response = await api.post(`/v1/leaderboards/${board.id}/trigger-rewards`, {})
      expect(response.status, response.text).toBe(200)
      expect(response.json).toEqual({
        winners: [
          { rank: 1, contactId: first.contactId, score: 30 },
          { rank: 2, contactId: noConversation, score: 25 },
        ],
        sent: 1,
        skipped: [noConversation],
        failed: [],
      })
      await wait(async () => {
        expect(await callbackTexts(channel, since)).toEqual([rewardText])
      })
      const records = await fake.requests({ kind: "http", host: channel.callbackHost, since })
      expect(records.map((record) => JSON.parse(record.bodyText).contact?.sourceId)).toEqual([first.sourceId])

      const after = await getLeaderboard(api, board.id)
      expect(after.rewardsTriggeredAt).toMatch(ISO)
      expect(after.availableActions).toEqual(ACTIONS.finished)
      expectError(await api.post(`/v1/leaderboards/${board.id}/trigger-rewards`, {}), 409, "leaderboardRewardsAlreadySent", {
        defined: true,
        message: "Rewards were already sent",
      })
      // A finished leaderboard whose rewards went out keeps its scores locked.
      expectError(
        await api.post(`/v1/leaderboards/${board.id}/participants/${third.contactId}/adjust`, { delta: 1, reason: "late" }),
        409,
        "leaderboardFinished",
      )
    },
    120_000,
  )

  it("needs a reward flow (422) and a known leaderboard (404)", async () => {
    const { tenant, api } = await newTenant("p110-lb-rewards-bad")
    const board = await activeLeaderboard(api, tenant)
    await transition(api, board.id, "finish")
    expectError(await api.post(`/v1/leaderboards/${board.id}/trigger-rewards`, {}), 422, "leaderboardRewardFlowMissing", {
      defined: true,
      message: "Choose a reward flow first",
    })
    expect((await getLeaderboard(api, board.id)).rewardsTriggeredAt).toBeNull()
    expectNotFound(await api.post(`/v1/leaderboards/${UNKNOWN_ID}/trigger-rewards`, {}), NOT_FOUND)
  })

  it("a finished leaderboard with no participants reports no winners", async () => {
    const { tenant, api } = await newTenant("p110-lb-rewards-empty")
    const rewardFlow = await createPublishedFlow(api, uniqueName("Reward"), "nobody")
    const board = await activeLeaderboard(api, tenant, { rewardSettings: { flowId: rewardFlow } })
    await transition(api, board.id, "finish")
    const response = await api.post(`/v1/leaderboards/${board.id}/trigger-rewards`, {})
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ winners: [], sent: 0, skipped: [], failed: [] })
    expectError(await api.post(`/v1/leaderboards/${board.id}/trigger-rewards`, {}), 409, "leaderboardRewardsAlreadySent")
  })
})
