// Repeating broadcasts: previewRecurrence, create (recurring), skipOccurrence,
// restoreOccurrence, listOccurrences, listRunGroups, and stop / resume /
// moveToDraft / schedule on a series. The last test waits for the scheduler
// to create a run (worker effect).

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  type Broadcast,
  createPublishedFlow,
  errorShape,
  expectError,
  expectInvalid,
  newTenant,
} from "./helpers.ts"

const KEEP = ["schedulesAt", "nextRunAt", "nextSendAt", "upcomingRuns", "occurrenceAt", "startsOn"]

const dailyRule = (overrides: Record<string, unknown> = {}) => ({
  frequency: "daily",
  interval: 1,
  timeOfDay: "09:00",
  timezone: "UTC",
  startsOn: "2030-01-01",
  end: { kind: "never" },
  ...overrides,
})

const createSeries = async (api: Api, flowId: string, rule = dailyRule(), extra: Record<string, unknown> = {}) => {
  const response = await api.post<Broadcast>("/v1/broadcasts", {
    channel: "omnichannel",
    subaction: "allContacts",
    flowId,
    contactFilter: { operator: "and", conditions: [] },
    schedulesType: "recurring",
    schedulesAt: null,
    recurrence: rule,
    ...extra,
  })
  expect(response.status, response.text).toBe(201)
  return response.json
}

const getBroadcast = async (api: Api, id: string) => {
  const response = await api.get<Broadcast & { recurrence: Record<string, unknown> | null }>(`/v1/broadcasts/${id}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("broadcasts.previewRecurrence", () => {
  it("returns the next runs of daily, weekly and monthly rules", async () => {
    const { api } = await newTenant("rec-preview")
    const daily = await api.post("/v1/broadcasts/preview-recurrence", { recurrence: dailyRule() })
    expect(daily.status, daily.text).toBe(200)
    expect(daily.json).toEqual({
      upcomingRuns: ["2030-01-01T09:00:00.000Z", "2030-01-02T09:00:00.000Z", "2030-01-03T09:00:00.000Z"],
      timezone: "UTC",
      weekStartsOn: "monday",
    })

    const weekly = await api.post<{ upcomingRuns: string[] }>("/v1/broadcasts/preview-recurrence", {
      recurrence: dailyRule({ frequency: "weekly", weekdays: [1, 3], timezone: "Asia/Ho_Chi_Minh", timeOfDay: "08:30" }),
      count: 4,
    })
    expect(weekly.status, weekly.text).toBe(200)
    // 2030-01-01 is a Tuesday; Mondays and Wednesdays at 08:30 UTC+7.
    expect(weekly.json.upcomingRuns).toEqual([
      "2030-01-02T01:30:00.000Z",
      "2030-01-07T01:30:00.000Z",
      "2030-01-09T01:30:00.000Z",
      "2030-01-14T01:30:00.000Z",
    ])

    const monthly = await api.post<{ upcomingRuns: string[] }>("/v1/broadcasts/preview-recurrence", {
      recurrence: dailyRule({ frequency: "monthly", monthDay: 31, startsOn: "2030-01-15" }),
      count: 3,
    })
    expect(monthly.json.upcomingRuns).toEqual([
      "2030-01-31T09:00:00.000Z",
      "2030-02-28T09:00:00.000Z",
      "2030-03-31T09:00:00.000Z",
    ])

    const ending = await api.post<{ upcomingRuns: string[] }>("/v1/broadcasts/preview-recurrence", {
      recurrence: dailyRule({ end: { kind: "afterOccurrences", count: 2 }, excludeDates: ["2030-01-01"] }),
      count: 5,
    })
    expect({ upcomingRuns: ending.json.upcomingRuns }).toMatchSnapshot("ending with an excluded date")
  })

  it("validates the rule and the count (422)", async () => {
    const { api } = await newTenant("rec-preview-invalid")
    const path = "/v1/broadcasts/preview-recurrence"
    expectInvalid(await api.post(path, {}), "recurrence")
    expectInvalid(await api.post(path, { recurrence: dailyRule(), count: 11 }), "count")
    expectInvalid(await api.post(path, { recurrence: dailyRule({ frequency: "weekly" }) }), "recurrence.weekdays")
    expectInvalid(await api.post(path, { recurrence: dailyRule({ frequency: "monthly" }) }), "recurrence.monthDay")
    expectInvalid(await api.post(path, { recurrence: dailyRule({ timezone: "Mars/Base" }) }), "recurrence.timezone")
    expectInvalid(await api.post(path, { recurrence: dailyRule({ timeOfDay: "25:00" }) }), "recurrence.timeOfDay")
    expectInvalid(await api.post(path, { recurrence: dailyRule({ startsOn: "2030-02-30" }) }), "recurrence.startsOn")
    expectInvalid(
      await api.post(path, { recurrence: dailyRule({ end: { kind: "onDate", date: "2029-12-31" } }) }),
      "recurrence.end.date",
    )
    expectInvalid(await api.post(path, { recurrence: dailyRule({ interval: 0 }) }), "recurrence.interval")
  })
})

describe("broadcasts.create (recurring)", () => {
  it("creates a running series whose detail shows the schedule", async () => {
    const normalizer = new Normalizer()
    const { api } = await newTenant("rec-create")
    const flowId = await createPublishedFlow(api, "Series flow")
    const created = await createSeries(api, flowId)
    expect(created).toMatchObject({ status: "recurring", schedulesType: "recurring", flowId })
    expect(normalizer.normalize(created, { keep: KEEP })).toMatchSnapshot("created series")

    const detail = await getBroadcast(api, created.id)
    expect(detail.recurrence).toMatchObject({
      rule: dailyRule(),
      nextRunAt: "2030-01-01T09:00:00.000Z",
      nextSendAt: "2030-01-01T09:00:00.000Z",
      occurrenceCount: 0,
      lastOccurrenceAt: null,
      pausedAt: null,
      endedAt: null,
      upcomingRuns: ["2030-01-01T09:00:00.000Z", "2030-01-02T09:00:00.000Z", "2030-01-03T09:00:00.000Z"],
    })
    expect(normalizer.normalize(detail, { keep: KEEP })).toMatchSnapshot("series detail")

    const listed = await api.get<{ data: Broadcast[] }>("/v1/broadcasts", { query: { schedulesType: "recurring" } })
    expect(listed.json.data.map((item) => item.id)).toEqual([created.id])
    expect(listed.json.data[0]?.recurrence).toMatchObject({ nextSendAt: "2030-01-01T09:00:00.000Z" })
    const withoutSeries = await api.get<{ data: Broadcast[] }>("/v1/broadcasts", { query: { excludeTemplates: "true" } })
    expect(withoutSeries.json.data).toEqual([])
  })

  it("saves a repeating draft without arming it", async () => {
    const { api } = await newTenant("rec-create-draft")
    const flowId = await createPublishedFlow(api, "Series flow")
    const draft = await createSeries(api, flowId, dailyRule(), { saveAsDraft: true })
    expect(draft.status).toBe("draft")
    const detail = await getBroadcast(api, draft.id)
    expect(
      new Normalizer().normalize({ status: detail.status, recurrence: detail.recurrence }, { keep: KEEP }),
    ).toMatchSnapshot("repeating draft")
  })

  it("refuses a rule with no upcoming run (422 validation)", async () => {
    const { api } = await newTenant("rec-create-past")
    const flowId = await createPublishedFlow(api, "Series flow")
    const response = await api.post("/v1/broadcasts", {
      channel: "omnichannel",
      subaction: "allContacts",
      flowId,
      contactFilter: { operator: "and", conditions: [] },
      schedulesType: "recurring",
      schedulesAt: null,
      recurrence: dailyRule({ startsOn: "2020-01-01", end: { kind: "onDate", date: "2020-01-02" } }),
    })
    const body = expectError(response, 422, "validation")
    expect(body.message).toBe("The schedule has no upcoming run")
  })
})

describe("broadcasts.skipOccurrence and broadcasts.restoreOccurrence", () => {
  it("skips runs by date or instant and restores them", async () => {
    const { api } = await newTenant("rec-skip")
    const flowId = await createPublishedFlow(api, "Series flow")
    const series = await createSeries(api, flowId)

    const skipped = await api.post(`/v1/broadcasts/${series.id}/skip-occurrence`, { dates: ["2030-01-02"] })
    expect(skipped.status, skipped.text).toBe(200)
    expect(skipped.json).toEqual({
      excludeDates: ["2030-01-02"],
      nextSendAt: "2030-01-01T09:00:00.000Z",
      upcomingRuns: ["2030-01-01T09:00:00.000Z", "2030-01-03T09:00:00.000Z", "2030-01-04T09:00:00.000Z"],
    })
    const both = await api.post(`/v1/broadcasts/${series.id}/skip-occurrence`, {
      occurrenceAts: ["2030-01-01T09:00:00.000Z"],
      dates: ["2030-01-02"],
    })
    expect(both.json).toEqual({
      excludeDates: ["2030-01-01", "2030-01-02"],
      nextSendAt: "2030-01-03T09:00:00.000Z",
      upcomingRuns: ["2030-01-03T09:00:00.000Z", "2030-01-04T09:00:00.000Z", "2030-01-05T09:00:00.000Z"],
    })
    expect((await getBroadcast(api, series.id)).recurrence).toMatchObject({
      rule: { excludeDates: ["2030-01-01", "2030-01-02"] },
      nextRunAt: "2030-01-01T09:00:00.000Z",
      nextSendAt: "2030-01-03T09:00:00.000Z",
    })

    const restored = await api.post(`/v1/broadcasts/${series.id}/restore-occurrence`, { dates: ["2030-01-01"] })
    expect(restored.status, restored.text).toBe(200)
    expect(restored.json).toEqual({
      excludeDates: ["2030-01-02"],
      nextSendAt: "2030-01-01T09:00:00.000Z",
      upcomingRuns: ["2030-01-01T09:00:00.000Z", "2030-01-03T09:00:00.000Z", "2030-01-04T09:00:00.000Z"],
    })
    // Restoring a date that is not skipped is a no-op.
    const noop = await api.post(`/v1/broadcasts/${series.id}/restore-occurrence`, { dates: ["2030-01-05"] })
    expect(noop.json).toEqual(restored.json)
  })

  it("rejects dates without a run, empty requests and non-repeating broadcasts", async () => {
    const { api } = await newTenant("rec-skip-invalid")
    const flowId = await createPublishedFlow(api, "Series flow")
    const series = await createSeries(api, flowId, dailyRule({ frequency: "weekly", weekdays: [1] }))
    const noRun = await api.post(`/v1/broadcasts/${series.id}/skip-occurrence`, { dates: ["2030-01-08"] })
    expect(expectError(noRun, 422, "validation").message).toBe("The schedule has no run on that date")
    expect(errorShape(noRun)).toMatchSnapshot("no run")
    expectInvalid(await api.post(`/v1/broadcasts/${series.id}/skip-occurrence`, {}), "dates")
    expectInvalid(await api.post(`/v1/broadcasts/${series.id}/skip-occurrence`, { dates: ["2030-02-30"] }), "dates.0")
    expectInvalid(await api.post(`/v1/broadcasts/${series.id}/restore-occurrence`, { occurrenceAts: ["x"] }), "occurrenceAts.0")

    const oneOff = await api.post<Broadcast>("/v1/broadcasts", {
      channel: "omnichannel",
      subaction: "allContacts",
      flowId,
      contactFilter: { operator: "and", conditions: [] },
      schedulesType: "future",
      schedulesAt: "2030-01-07T09:00:00.000Z",
    })
    const notSeries = await api.post(`/v1/broadcasts/${oneOff.json.id}/skip-occurrence`, { dates: ["2030-01-07"] })
    expect(expectError(notSeries, 404, "notFound").message).toBe("This broadcast does not repeat")
    expectError(await api.post("/v1/broadcasts/1/restore-occurrence", { dates: ["2030-01-07"] }), 404, "notFound")
  })
})

describe("broadcasts.listOccurrences and broadcasts.listRunGroups", () => {
  it("are empty before the first run", async () => {
    const { api } = await newTenant("rec-ledger-empty")
    const flowId = await createPublishedFlow(api, "Series flow")
    const series = await createSeries(api, flowId)
    const occurrences = await api.get(`/v1/broadcasts/${series.id}/occurrences`)
    expect(occurrences.status, occurrences.text).toBe(200)
    expect(occurrences.json).toEqual({ data: [], nextCursor: null })
    const groups = await api.get(`/v1/broadcasts/${series.id}/run-groups`)
    expect(groups.status, groups.text).toBe(200)
    expect(groups.json).toEqual({
      data: [],
      nextCursor: null,
      totalSlots: 0,
      totalRuns: 0,
      timezone: "UTC",
      weekStartsOn: "monday",
    })
    const tokyo = await api.get<{ timezone: string }>(`/v1/broadcasts/${series.id}/run-groups`, {
      query: { timezone: "Asia/Tokyo", groupBy: "week" },
    })
    expect(tokyo.json.timezone).toBe("Asia/Tokyo")
    const unknownZone = await api.get<{ timezone: string }>(`/v1/broadcasts/${series.id}/run-groups`, {
      query: { timezone: "Mars/Base" },
    })
    expect(unknownZone.json.timezone).toBe("UTC")
    // By name too.
    expect((await api.get(`/v1/broadcasts/${encodeURIComponent(series.name)}/occurrences`)).status).toBe(200)
  })

  it("validate their query and are 404 for non-repeating broadcasts", async () => {
    const { api } = await newTenant("rec-ledger-invalid")
    const flowId = await createPublishedFlow(api, "Series flow")
    const series = await createSeries(api, flowId)
    const bad = await api.get(`/v1/broadcasts/${series.id}/occurrences`, { query: { cursor: "nope" } })
    expect(expectError(bad, 422, "validation").message).toBe("Invalid cursor")
    expectInvalid(await api.get(`/v1/broadcasts/${series.id}/occurrences`, { query: { limit: 101 } }), "limit")
    expectInvalid(await api.get(`/v1/broadcasts/${series.id}/occurrences`, { query: { from: "soon" } }), "from")
    expectInvalid(await api.get(`/v1/broadcasts/${series.id}/run-groups`, { query: { groupBy: "year" } }), "groupBy")
    expectInvalid(await api.get(`/v1/broadcasts/${series.id}/run-groups`, { query: { limit: 32 } }), "limit")
    expectError(await api.get("/v1/broadcasts/Unknown/occurrences"), 404, "notFound")
    expectError(await api.get("/v1/broadcasts/Unknown/run-groups"), 404, "notFound")
  })
})

describe("stop, resume, moveToDraft and schedule on a series", () => {
  it("pauses and restarts a series; moves it to draft and arms it again", async () => {
    const normalizer = new Normalizer()
    const { api } = await newTenant("rec-pause")
    const flowId = await createPublishedFlow(api, "Series flow")
    const series = await createSeries(api, flowId)

    const stopped = await api.post(`/v1/broadcasts/${series.id}/stop`, {})
    expect(stopped.status, stopped.text).toBe(200)
    expect(stopped.json).toEqual({ id: series.id })
    const paused = await getBroadcast(api, series.id)
    expect(paused.status).toBe("cancelled")
    expect(paused.recurrence).toMatchObject({ nextRunAt: null, nextSendAt: null, upcomingRuns: [] })
    expect(Date.parse(String(paused.recurrence?.pausedAt))).not.toBeNaN()
    expect(normalizer.normalize(paused, { keep: KEEP })).toMatchSnapshot("paused")

    const resumed = await api.post(`/v1/broadcasts/${series.id}/resume`, {})
    expect(resumed.status, resumed.text).toBe(200)
    expect(resumed.json).toEqual({ id: series.id, nextRunAt: "2030-01-01T09:00:00.000Z" })
    expect((await getBroadcast(api, series.id)).status).toBe("recurring")

    const drafted = await api.post(`/v1/broadcasts/${series.id}/move-to-draft`, {})
    expect(drafted.status, drafted.text).toBe(200)
    expect(drafted.json).toEqual({ id: series.id })
    expect((await getBroadcast(api, series.id)).status).toBe("draft")

    const rearmed = await api.post(`/v1/broadcasts/${series.id}/schedule`, {
      schedulesType: "recurring",
      schedulesAt: null,
      recurrence: dailyRule({ timeOfDay: "10:00" }),
    })
    expect(rearmed.status, rearmed.text).toBe(200)
    expect(rearmed.json).toEqual({ id: series.id })
    expect((await getBroadcast(api, series.id)).recurrence).toMatchObject({
      nextRunAt: "2030-01-01T10:00:00.000Z",
      rule: { timeOfDay: "10:00" },
    })
  })

  it("refuses stop and resume on broadcasts in another state", async () => {
    const { api } = await newTenant("rec-pause-invalid")
    const flowId = await createPublishedFlow(api, "Series flow")
    const draft = await api.post<Broadcast>("/v1/broadcasts", {
      channel: "omnichannel",
      subaction: "allContacts",
      flowId,
      contactFilter: { operator: "and", conditions: [] },
      schedulesType: "future",
      schedulesAt: "2030-01-07T09:00:00.000Z",
      saveAsDraft: true,
    })
    const stop = await api.post(`/v1/broadcasts/${draft.json.id}/stop`, {})
    expect(stop.status).toBeGreaterThanOrEqual(400)
    expect(errorShape(stop)).toMatchSnapshot("stop a draft")
    const resume = await api.post(`/v1/broadcasts/${draft.json.id}/resume`, {})
    expect(resume.status).toBeGreaterThanOrEqual(400)
    expect(errorShape(resume)).toMatchSnapshot("resume a draft")
    expectInvalid(await api.post(`/v1/broadcasts/${draft.json.id}/resume`, { sendRatePerMinute: 0 }), "sendRatePerMinute")
    expectInvalid(await api.post("/v1/broadcasts/x/stop", {}), "id")
  })
})

describe("a series run created by the scheduler", () => {
  it(
    "materializes the next slot into a run that listOccurrences, listRunGroups and list show",
    async () => {
      const { api } = await newTenant("rec-run")
      const flowId = await createPublishedFlow(api, "Series flow")
      const MINUTE = 60_000
      const slot = new Date(Math.ceil((Date.now() + 25_000) / MINUTE) * MINUTE)
      const iso = slot.toISOString()
      const series = await createSeries(
        api,
        flowId,
        dailyRule({
          timeOfDay: iso.slice(11, 16),
          startsOn: iso.slice(0, 10),
          end: { kind: "afterOccurrences", count: 1 },
        }),
      )
      expect((await getBroadcast(api, series.id)).recurrence).toMatchObject({ nextRunAt: iso })

      const occurrences = await eventually(
        async () => {
          const response = await api.get<{
            data: Array<{ localSlot: string; occurrenceAt: string; outcome: string; run: { id: string } | null }>
          }>(`/v1/broadcasts/${series.id}/occurrences`)
          expect(response.json.data).toHaveLength(1)
          return response.json
        },
        { timeoutMs: 120_000, intervalMs: 2_000, description: "series run" },
      )
      const [row] = occurrences.data
      expect(row).toMatchObject({
        localSlot: `${iso.slice(0, 10)} ${iso.slice(11, 16)}`,
        occurrenceAt: iso,
        outcome: "created",
        reason: null,
        supersededCount: 1,
      })
      const runId = row?.run?.id as string
      expect(runId).toMatch(/^\d+$/)
      expect(occurrences.nextCursor).toBeNull()

      const run = await getBroadcast(api, runId)
      expect(run).toMatchObject({ parentBroadcastId: series.id, occurrenceAt: iso, flowId })
      expect(run.parent).toEqual({ id: series.id, name: series.name })
      const runs = await api.get<{ data: Broadcast[] }>("/v1/broadcasts", { query: { parentBroadcastId: series.id } })
      expect(runs.json.data.map((item) => item.id)).toEqual([runId])
      const noRuns = await api.get<{ data: Broadcast[] }>("/v1/broadcasts", { query: { includeRuns: "false" } })
      expect(noRuns.json.data.map((item) => item.id)).toEqual([series.id])

      const groups = await api.get<{
        data: Array<Record<string, unknown>>
        totalSlots: number
        totalRuns: number
      }>(`/v1/broadcasts/${series.id}/run-groups`)
      expect(groups.status, groups.text).toBe(200)
      expect(groups.json).toMatchObject({ totalSlots: 1, totalRuns: 1, nextCursor: null })
      expect(groups.json.data).toHaveLength(1)
      expect(groups.json.data[0]).toMatchObject({
        periodStart: iso.slice(0, 10),
        periodStartAt: `${iso.slice(0, 10)}T00:00:00.000Z`,
        slotCount: 1,
        createdCount: 1,
        skippedCount: 0,
        missedCount: 0,
      })

      // The only occurrence was used: the series has ended.
      await eventually(
        async () => {
          const detail = await getBroadcast(api, series.id)
          expect(detail.recurrence).toMatchObject({ occurrenceCount: 1, nextRunAt: null })
        },
        { timeoutMs: 30_000, intervalMs: 1_000, description: "series ended" },
      )
    },
    200_000,
  )
})
