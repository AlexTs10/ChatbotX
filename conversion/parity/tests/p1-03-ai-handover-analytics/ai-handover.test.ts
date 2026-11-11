// aiHandover.*: Meta Business AI hand-over settings of a Messenger Page and
// the "apply to all customers" bulk runs.
//
// The Page is an `Inbox` row (channel messenger) without a Messenger
// integration row, so a run the worker picks up cannot reach Meta: it ends
// `failed` with `integrationUnavailable` and counts the eligible threads as
// skipped. That keeps the async path deterministic and offline.

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { Normalizer } from "../../src/normalize.ts"
import { type Api, expectError, expectInvalid, ext, tenantApi, uniqueName } from "./helpers.ts"

type Settings = {
  enabled: boolean
  scheduleEnabled: boolean
  timeRanges: Array<{ from: number; to: number }>
  gotoFlowId: string | null
  returnMessage: string | null
  pauseBotWaitingForStaff: boolean
}
type Run = {
  id: string
  action: "enable" | "disable"
  status: string
  message: string | null
  requestedAt: string
  startedAt: string | null
  finishedAt: string | null
  processedCount: number
  skippedCount: number
  failedCount: number
  totalCount: number | null
  currentError: string | null
  requestedByName: string | null
  pausedUntil: string | null
}
type ApplyToAll = { status: string; applyToAllCustomers: boolean; isAutomationActive: boolean; run: Run | null }
type SettingsWithStatus = Settings & { applyToAll: ApplyToAll }
type ApplyResponse = { dryRun: boolean; isChanged: boolean; eligibleCount: number | null; run: Run | null }

const DEFAULTS: Settings = {
  enabled: false,
  scheduleEnabled: false,
  timeRanges: [],
  gotoFlowId: null,
  returnMessage: null,
  pauseBotWaitingForStaff: false,
}

/** Always-on automation: enabled, a schedule that covers every hour. */
const ALWAYS_ON: Settings = {
  enabled: true,
  scheduleEnabled: true,
  timeRanges: [{ from: 0, to: 24 }],
  gotoFlowId: null,
  returnMessage: "Back to the team",
  pauseBotWaitingForStaff: true,
}

const page = async (label: string, options: { channel?: string; status?: "connected" | "disconnected" } = {}) => {
  const { tenant, api } = await tenantApi(label)
  const { inboxId } = await (await ext()).createInbox({
    workspaceId: tenant.workspaceId,
    name: uniqueName("Page"),
    channel: options.channel ?? "messenger",
    status: options.status,
  })
  const base = `/v1/inboxes/${inboxId}/ai-handover`
  return { tenant, api, inboxId, base }
}

/** A customer thread that an enable run would hand over (active in the last 30 days). */
const eligibleThread = async (workspaceId: string, inboxId: string) =>
  (await ext()).createContact({
    workspaceId,
    inboxId,
    channel: "messenger",
    firstName: "Eli",
    lastIncomingMessageAt: new Date(Date.now() - 24 * 3600_000).toISOString(),
  })

const waitForRunEnd = (api: Api, base: string, runId: string) =>
  eventually(
    async () => {
      const response = await api.get<SettingsWithStatus>(`${base}/settings`)
      expect(response.status).toBe(200)
      const run = response.json.applyToAll.run
      expect(run?.id).toBe(runId)
      expect(["completed", "failed", "cancelled"]).toContain(run?.status)
      return response.json
    },
    { timeoutMs: 25_000, intervalMs: 250, description: `run ${runId} ends` },
  )

describe("aiHandover.getSettings", () => {
  it("returns the defaults and an idle status for a Page that never saved settings", async () => {
    const { api, base } = await page("aih-get")
    const response = await api.get(`${base}/settings`)
    expect(response.status).toBe(200)
    expect(response.json).toEqual({
      ...DEFAULTS,
      applyToAll: { status: "idle", applyToAllCustomers: false, isAutomationActive: false, run: null },
    })
  })

  it("is 404 notFound for an unknown inbox, a non-messenger inbox and another workspace's inbox", async () => {
    const { api } = await page("aih-get-404")
    const other = await page("aih-get-404-other")
    const telegram = await page("aih-get-404-tg", { channel: "telegram" })
    const unknown = await api.get("/v1/inboxes/1/ai-handover/settings")
    expectError(unknown, 404, "notFound", { defined: true })
    expect((unknown.json as { message?: string }).message).toBe("Inbox not found")
    expectError(await telegram.api.get(`${telegram.base}/settings`), 404, "notFound")
    expectError(await api.get(`${other.base}/settings`), 404, "notFound")
  })

  it("rejects a non-numeric inbox id (422)", async () => {
    const { api } = await page("aih-get-bad")
    expectInvalid(await api.get("/v1/inboxes/abc/ai-handover/settings"), "inboxId")
  })
})

describe("aiHandover.saveSettings", () => {
  it("replaces the settings, trims the return message, and reports the automation active", async () => {
    const { api, base } = await page("aih-save")
    const flow = await api.post<{ id: string }>("/v1/flows", { name: uniqueName("Return flow"), folderId: null })
    expect(flow.status).toBe(201)
    const saved = await api.put<Settings>(`${base}/settings`, {
      ...ALWAYS_ON,
      gotoFlowId: flow.json.id,
      returnMessage: "   Back to the team   ",
    })
    expect(saved.status).toBe(200)
    expect(saved.json).toEqual({ ...ALWAYS_ON, gotoFlowId: flow.json.id })

    const read = await api.get(`${base}/settings`)
    expect(read.json).toEqual({
      ...ALWAYS_ON,
      gotoFlowId: flow.json.id,
      applyToAll: { status: "idle", applyToAllCustomers: false, isAutomationActive: true, run: null },
    })
  })

  it("stores a wrapping schedule and turns an empty return message into null", async () => {
    const { api, base } = await page("aih-save-wrap")
    const body = { ...ALWAYS_ON, timeRanges: [{ from: 22, to: 6 }, { from: 12, to: 13 }], returnMessage: "   " }
    const saved = await api.put<Settings>(`${base}/settings`, body)
    expect(saved.status).toBe(200)
    expect(saved.json).toEqual({ ...body, returnMessage: null })
    // A GET result can be sent back to PUT unchanged.
    const echoed = await api.put<Settings>(`${base}/settings`, { ...saved.json, returnMessage: null })
    expect(echoed.json).toEqual(saved.json)
  })

  it("is 404 for an unknown or inactive goto flow and an unknown inbox", async () => {
    const { api, base } = await page("aih-save-flow")
    const unknownFlow = await api.put(`${base}/settings`, { ...ALWAYS_ON, gotoFlowId: "1" })
    expectError(unknownFlow, 404, "notFound", { defined: true })
    expect((unknownFlow.json as { message?: string }).message).toBe("Flow not found")
    const inactive = await api.post<{ id: string }>("/v1/flows", { name: uniqueName("Inactive"), folderId: null })
    expect((await api.patch(`/v1/flows/${inactive.json.id}`, { active: false })).status).toBe(204)
    expectError(await api.put(`${base}/settings`, { ...ALWAYS_ON, gotoFlowId: inactive.json.id }), 404, "notFound")
    expectError(await api.put("/v1/inboxes/1/ai-handover/settings", ALWAYS_ON), 404, "notFound")
    // Nothing was saved.
    expect((await api.get<Settings>(`${base}/settings`)).json.enabled).toBe(false)
  })

  it.each([
    ["a schedule without time ranges", { scheduleEnabled: true, timeRanges: [] }, "timeRanges"],
    ["a range with equal bounds", { timeRanges: [{ from: 5, to: 5 }] }, "timeRanges.0"],
    ["an hour above 24", { timeRanges: [{ from: 0, to: 25 }] }, "timeRanges.0.to"],
    ["a fractional hour", { timeRanges: [{ from: 1.5, to: 3 }] }, "timeRanges.0.from"],
    ["more than 24 ranges", { timeRanges: Array.from({ length: 25 }, (_, i) => ({ from: i % 24, to: (i % 24) + 1 })) }, "timeRanges"],
    ["a return message over 2000 characters", { returnMessage: "x".repeat(2001) }, "returnMessage"],
    ["a missing field", { enabled: undefined }, "enabled"],
    ["a non-numeric goto flow id", { gotoFlowId: "flow-1" }, "gotoFlowId"],
  ])("rejects %s (422)", async (_label, overrides, path) => {
    const { api, base } = await page("aih-save-bad")
    expectInvalid(await api.put(`${base}/settings`, { ...ALWAYS_ON, ...overrides }), path)
  })
})

describe("aiHandover.patchSettings", () => {
  it("changes only the given fields, starting from the defaults", async () => {
    const { api, base } = await page("aih-patch")
    const first = await api.patch<Settings>(`${base}/settings`, { enabled: true })
    expect(first.status).toBe(200)
    expect(first.json).toEqual({ ...DEFAULTS, enabled: true })
    const second = await api.patch<Settings>(`${base}/settings`, { returnMessage: " See you ", pauseBotWaitingForStaff: true })
    expect(second.json).toEqual({ ...DEFAULTS, enabled: true, returnMessage: "See you", pauseBotWaitingForStaff: true })
    const unchanged = await api.patch<Settings>(`${base}/settings`, {})
    expect(unchanged.json).toEqual(second.json)
    const cleared = await api.patch<Settings>(`${base}/settings`, { returnMessage: null })
    expect(cleared.json).toEqual({ ...second.json, returnMessage: null })
    expect((await api.get<SettingsWithStatus>(`${base}/settings`)).json).toMatchObject({
      ...cleared.json,
      applyToAll: { status: "idle", isAutomationActive: true },
    })
  })

  it("refuses a schedule without saved time ranges (422 validation)", async () => {
    const { api, base } = await page("aih-patch-schedule")
    const response = await api.patch(`${base}/settings`, { scheduleEnabled: true })
    expectError(response, 422, "validation", { defined: true })
    expect((response.json as { message?: string }).message).toBe("Add at least one time range")
  })

  it("is 404 for an unknown inbox and rejects invalid fields (422)", async () => {
    const { api, base } = await page("aih-patch-bad")
    expectError(await api.patch("/v1/inboxes/1/ai-handover/settings", { enabled: true }), 404, "notFound")
    expectInvalid(await api.patch(`${base}/settings`, { enabled: "yes" }), "enabled")
    expectInvalid(await api.patch(`${base}/settings`, { timeRanges: [{ from: -1, to: 2 }] }), "timeRanges.0.from")
  })
})

describe("aiHandover.setApplyToAll", () => {
  it("dry run counts the eligible threads without changing anything", async () => {
    const { tenant, api, base, inboxId } = await page("aih-dry")
    expect((await api.put(`${base}/settings`, ALWAYS_ON)).status).toBe(200)
    await eligibleThread(tenant.workspaceId, inboxId)
    await eligibleThread(tenant.workspaceId, inboxId)
    // Inactive for 40 days: outside the 30-day activity window.
    await (await ext()).createContact({
      workspaceId: tenant.workspaceId,
      inboxId,
      channel: "messenger",
      lastIncomingMessageAt: new Date(Date.now() - 40 * 24 * 3600_000).toISOString(),
    })

    const dry = await api.post(`${base}/apply-to-all`, { applyToAllCustomers: true, dryRun: true })
    expect(dry.status).toBe(200)
    expect(dry.json).toEqual({ dryRun: true, run: null, isChanged: true, eligibleCount: 2 })
    // OFF while the Page already is OFF: nothing would change.
    const offDry = await api.post(`${base}/apply-to-all`, { applyToAllCustomers: false, message: "Back", dryRun: true })
    expect(offDry.json).toEqual({ dryRun: true, run: null, isChanged: false, eligibleCount: 0 })
    expect((await api.get(`${base}/settings`)).json).toMatchObject({
      applyToAll: { status: "idle", applyToAllCustomers: false, run: null },
    })
    expect((await api.get(`${base}/history`)).json).toEqual({ data: [], pageCount: 0 })
  })

  it("refuses ON while the automation is not running, and on a disconnected Page (422)", async () => {
    const { api, base } = await page("aih-refuse")
    const off = await api.post(`${base}/apply-to-all`, { applyToAllCustomers: true, dryRun: true })
    expectError(off, 422, "aiHandoverBulkAutomationNotActive", { defined: true })
    const offReal = await api.post(`${base}/apply-to-all`, { applyToAllCustomers: true, confirmCount: 10 })
    expectError(offReal, 422, "aiHandoverBulkAutomationNotActive", { defined: true })

    const disconnected = await page("aih-refuse-dc", { status: "disconnected" })
    expect((await disconnected.api.put(`${disconnected.base}/settings`, ALWAYS_ON)).status).toBe(200)
    const dc = await disconnected.api.post(`${disconnected.base}/apply-to-all`, { applyToAllCustomers: true, dryRun: true })
    expectError(dc, 422, "aiHandoverBulkPageNotConnected", { defined: true })
  })

  it("refuses a change that touches more threads than confirmed (422)", async () => {
    const { tenant, api, base, inboxId } = await page("aih-confirm")
    expect((await api.put(`${base}/settings`, ALWAYS_ON)).status).toBe(200)
    await eligibleThread(tenant.workspaceId, inboxId)
    const response = await api.post(`${base}/apply-to-all`, { applyToAllCustomers: true, confirmCount: 0 })
    expectError(response, 422, "aiHandoverBulkConfirmCountExceeded", { defined: true })
    expect((response.json as { message?: string }).message).toBe("1 threads are eligible, more than the 0 confirmed")
    expect((await api.get(`${base}/history`)).json).toEqual({ data: [], pageCount: 0 })
  })

  it("starts an enable run; the worker ends it failed (no Messenger integration) with the threads skipped", async () => {
    const normalizer = new Normalizer()
    const { tenant, api, base, inboxId } = await page("aih-run")
    expect((await api.put(`${base}/settings`, ALWAYS_ON)).status).toBe(200)
    await eligibleThread(tenant.workspaceId, inboxId)

    const started = await api.post<ApplyResponse>(`${base}/apply-to-all`, { applyToAllCustomers: true, confirmCount: 1 })
    expect(started.status).toBe(200)
    expect(normalizer.normalize(started.json)).toEqual({
      dryRun: false,
      isChanged: true,
      eligibleCount: null,
      run: {
        id: "<id:1>",
        action: "enable",
        status: "pending",
        message: null,
        requestedAt: "<ts>",
        startedAt: null,
        finishedAt: null,
        processedCount: 0,
        skippedCount: 0,
        failedCount: 0,
        totalCount: null,
        currentError: null,
        requestedByName: null,
        pausedUntil: null,
      },
    })
    const runId = started.json.run?.id as string

    const ended = await waitForRunEnd(api, base, runId)
    expect(normalizer.normalize(ended)).toEqual({
      ...ALWAYS_ON,
      applyToAll: {
        status: "failed",
        applyToAllCustomers: true,
        isAutomationActive: true,
        run: {
          id: "<id:1>",
          action: "enable",
          status: "failed",
          message: null,
          requestedAt: "<ts>",
          startedAt: "<ts>",
          finishedAt: "<ts>",
          processedCount: 0,
          skippedCount: 1,
          failedCount: 0,
          totalCount: 1,
          currentError: "integrationUnavailable",
          requestedByName: null,
          pausedUntil: null,
        },
      },
    })

    // Asking for the state the Page is already in changes nothing.
    const again = await api.post(`${base}/apply-to-all`, { applyToAllCustomers: true, confirmCount: 5 })
    expect(again.json).toEqual({ dryRun: false, isChanged: false, eligibleCount: null, run: null })
    const history = await api.get<{ data: Run[]; pageCount: number }>(`${base}/history`)
    expect(history.json.pageCount).toBe(1)
    expect(history.json.data.map((run) => run.id)).toEqual([runId])
    expect(history.json.data[0]).toEqual(ended.applyToAll.run)
  })

  it("starts a disable run with the trimmed message after an enable", async () => {
    const normalizer = new Normalizer()
    const { tenant, api, base, inboxId } = await page("aih-disable")
    expect((await api.put(`${base}/settings`, ALWAYS_ON)).status).toBe(200)
    await eligibleThread(tenant.workspaceId, inboxId)
    const on = await api.post<ApplyResponse>(`${base}/apply-to-all`, { applyToAllCustomers: true, confirmCount: 1 })
    await waitForRunEnd(api, base, on.json.run?.id as string)

    // No thread is held by the AI, so the take-back touches none.
    const dry = await api.post(`${base}/apply-to-all`, { applyToAllCustomers: false, message: "  We are back  ", dryRun: true })
    expect(dry.json).toEqual({ dryRun: true, run: null, isChanged: true, eligibleCount: 0 })
    const off = await api.post<ApplyResponse>(`${base}/apply-to-all`, {
      applyToAllCustomers: false,
      message: "  We are back  ",
      confirmCount: 0,
    })
    expect(off.status).toBe(200)
    expect(normalizer.normalize(off.json)).toMatchObject({
      dryRun: false,
      isChanged: true,
      eligibleCount: null,
      run: { action: "disable", status: "pending", message: "We are back", totalCount: null },
    })
    const ended = await waitForRunEnd(api, base, off.json.run?.id as string)
    expect(ended.applyToAll).toMatchObject({
      status: "failed",
      applyToAllCustomers: false,
      run: { action: "disable", status: "failed", message: "We are back", currentError: "integrationUnavailable", totalCount: 0, skippedCount: 0 },
    })
    const history = await api.get<{ data: Run[] }>(`${base}/history`)
    expect(history.json.data.map((run) => run.action)).toEqual(["disable", "enable"])
  })

  it.each([
    ["OFF without a message", { applyToAllCustomers: false, confirmCount: 1 }, "message"],
    ["OFF with a blank message", { applyToAllCustomers: false, message: "   ", confirmCount: 1 }, "message"],
    ["no confirmCount without dryRun", { applyToAllCustomers: true }, "confirmCount"],
    ["a negative confirmCount", { applyToAllCustomers: true, confirmCount: -1 }, "confirmCount"],
    ["a message over 2000 characters", { applyToAllCustomers: false, message: "x".repeat(2001), dryRun: true }, "message"],
    ["applyToAllCustomers missing", { dryRun: true }, "applyToAllCustomers"],
  ])("rejects %s (422)", async (_label, body, path) => {
    const { api, base } = await page("aih-apply-bad")
    expectInvalid(await api.post(`${base}/apply-to-all`, body), path)
  })

  it("is 404 for an unknown inbox", async () => {
    const { api } = await page("aih-apply-404")
    expectError(
      await api.post("/v1/inboxes/1/ai-handover/apply-to-all", { applyToAllCustomers: true, dryRun: true }),
      404,
      "notFound",
      { defined: true },
    )
  })
})

describe("aiHandover.retryApplyToAll", () => {
  it("restarts the failed latest request as a new run", async () => {
    const normalizer = new Normalizer()
    const { tenant, api, base, inboxId } = await page("aih-retry")
    expect((await api.put(`${base}/settings`, ALWAYS_ON)).status).toBe(200)
    await eligibleThread(tenant.workspaceId, inboxId)
    const first = await api.post<ApplyResponse>(`${base}/apply-to-all`, { applyToAllCustomers: true, confirmCount: 1 })
    const firstId = first.json.run?.id as string
    await waitForRunEnd(api, base, firstId)

    const retried = await api.post<ApplyResponse>(`${base}/apply-to-all/retry`, {})
    expect(retried.status).toBe(200)
    normalizer.idLabel(firstId)
    expect(normalizer.normalize(retried.json)).toEqual({
      dryRun: false,
      isChanged: true,
      eligibleCount: null,
      run: {
        id: "<id:2>",
        action: "enable",
        status: "pending",
        message: null,
        requestedAt: "<ts>",
        startedAt: null,
        finishedAt: null,
        processedCount: 0,
        skippedCount: 0,
        failedCount: 0,
        totalCount: null,
        currentError: null,
        requestedByName: null,
        pausedUntil: null,
      },
    })
    const secondId = retried.json.run?.id as string
    const ended = await waitForRunEnd(api, base, secondId)
    expect(ended.applyToAll.run).toMatchObject({ status: "failed", currentError: "integrationUnavailable", skippedCount: 1 })

    const history = await api.get<{ data: Run[]; pageCount: number }>(`${base}/history`)
    expect(history.json.data.map((run) => run.id)).toEqual([secondId, firstId])
    expect(history.json.pageCount).toBe(1)
  })

  it("refuses when there is nothing to retry (422)", async () => {
    const { api, base } = await page("aih-retry-none")
    const response = await api.post(`${base}/apply-to-all/retry`, {})
    expectError(response, 422, "aiHandoverBulkNothingToRetry", { defined: true })
    expect((response.json as { message?: string }).message).toBe("There is nothing to retry")
  })

  it("is 404 for an unknown inbox", async () => {
    const { api } = await page("aih-retry-404")
    expectError(await api.post("/v1/inboxes/1/ai-handover/apply-to-all/retry", {}), 404, "notFound", { defined: true })
  })
})

describe("aiHandover.listHistory", () => {
  it("pages the runs newest first", async () => {
    const { tenant, api, base, inboxId } = await page("aih-history")
    expect((await api.put(`${base}/settings`, ALWAYS_ON)).status).toBe(200)
    await eligibleThread(tenant.workspaceId, inboxId)
    const first = await api.post<ApplyResponse>(`${base}/apply-to-all`, { applyToAllCustomers: true, confirmCount: 1 })
    await waitForRunEnd(api, base, first.json.run?.id as string)
    const second = await api.post<ApplyResponse>(`${base}/apply-to-all/retry`, {})
    await waitForRunEnd(api, base, second.json.run?.id as string)

    const page1 = await api.get<{ data: Run[]; pageCount: number }>(`${base}/history`, { query: { perPage: 1 } })
    expect(page1.status).toBe(200)
    expect(page1.json.pageCount).toBe(2)
    expect(page1.json.data.map((run) => run.id)).toEqual([second.json.run?.id])
    const page2 = await api.get<{ data: Run[]; pageCount: number }>(`${base}/history`, { query: { perPage: 1, page: 2 } })
    expect(page2.json.data.map((run) => run.id)).toEqual([first.json.run?.id])
    expect(page2.json.data[0]).toMatchObject({ requestedByName: null, status: "failed" })
  })

  it("is 404 for an unknown inbox and rejects bad paging (422)", async () => {
    const { api, base } = await page("aih-history-bad")
    expectError(await api.get("/v1/inboxes/1/ai-handover/history"), 404, "notFound", { defined: true })
    expectInvalid(await api.get(`${base}/history`, { query: { perPage: 51 } }), "perPage")
    expectInvalid(await api.get(`${base}/history`, { query: { page: 0 } }), "page")
  })
})
