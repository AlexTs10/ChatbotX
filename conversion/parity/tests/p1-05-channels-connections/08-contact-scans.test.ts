// contactScans.* — Automatic Customer Scan of a Messenger inbox. create writes
// an `init` run; the worker's scheduler (every minute) claims it, walks the
// page's conversations on Graph (fake upstream) and imports the customers.

import { describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import { fake } from "../../src/fake.ts"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  ext,
  expectError,
  expectInvalid,
  graphPath,
  isoFromNow,
  qs,
  reEscape,
  script,
  tenantApi,
  uniqueDigits,
  upstreamCalls,
} from "./helpers.ts"

const GRAPH = "graph.facebook.com"
const DAY = 86_400_000

type Status = {
  status: string
  latest: Record<string, unknown> | null
  availability: { canScan: boolean; blockedReason?: string; nextScanAt?: string }
}
type RunList = { data: Array<Record<string, unknown> & { id: string; status: string }>; pageCount: number }

const graphTime = (ms: number) => new Date(Date.now() + ms).toISOString().replace(/\.\d{3}Z$/, "+0000")

const messengerInbox = async (workspaceId: string) => {
  const pageId = uniqueDigits()
  const channel = await (await ext()).createMessengerChannel({ workspaceId, pageId, name: `Scan page ${pageId}` })
  return { pageId, ...channel }
}

/** Answers every Graph call about this page; the conversation list returns `customers`. */
const scriptConversations = async (pageId: string, customers: Array<{ psid: string; name: string }>) => {
  await script({
    match: { host: GRAPH, pathRegex: `^/v[0-9.]+/[^?]*${reEscape(pageId)}` },
    respond: { status: 200, json: { data: [] } },
  })
  for (const customer of customers) {
    await script({
      match: { host: GRAPH, pathRegex: `^/v[0-9.]+/[^?]*${reEscape(customer.psid)}` },
      respond: { status: 200, json: { id: customer.psid, name: customer.name } },
    })
  }
  await script({
    match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${reEscape(pageId)}/conversations`) },
    respond: {
      status: 200,
      json: {
        data: customers.map((customer, index) => ({
          id: `t_${uniqueDigits()}`,
          updated_time: graphTime(-(index + 1) * 60_000),
          participants: {
            data: [
              { id: customer.psid, name: customer.name, email: `${customer.psid}@facebook.com` },
              { id: pageId, name: "Scan page", email: `${pageId}@facebook.com` },
            ],
          },
        })),
      },
    },
  })
}

describe("contactScans.getStatus", () => {
  it("is idle for an inbox that was never scanned, and for an unknown inbox", async () => {
    const { tenant, api } = await tenantApi("p105-scan-idle")
    const inbox = await messengerInbox(tenant.workspaceId)
    const idle = { status: "idle", latest: null, availability: { canScan: true } }
    const response = await api.get(`/v1/contact-scans/status${qs({ inboxId: inbox.inboxId })}`)
    expect(response.status).toBe(200)
    expect(response.json).toEqual(idle)
    expect((await api.get(`/v1/contact-scans/status${qs({ inboxId: "1" })}`)).json).toEqual(idle)
  })

  it("requires a numeric inboxId (422)", async () => {
    const { api } = await tenantApi("p105-scan-status-bad")
    expectInvalid(await api.get("/v1/contact-scans/status"), "inboxId")
    expectInvalid(await api.get(`/v1/contact-scans/status${qs({ inboxId: "abc" })}`), "inboxId")
  })
})

describe("contactScans.create", () => {
  it("rejects a scan the inbox cannot run", async () => {
    const { tenant, api } = await tenantApi("p105-scan-refuse")
    const fixtures = await ext()
    const past = isoFromNow(-DAY)
    const messenger = await messengerInbox(tenant.workspaceId)
    expectError(
      await api.post("/v1/contact-scans", { inboxId: messenger.inboxId, scanFromAt: isoFromNow(DAY) }),
      400,
      "contactScanFromTimeInvalid",
      { defined: true, message: "Scan-from time must be in the past." },
    )
    expectError(await api.post("/v1/contact-scans", { inboxId: "1", scanFromAt: past }), 404, "contactScanInboxNotFound", {
      defined: true,
      message: "Inbox not found.",
    })
    const telegram = await fixtures.createInbox({
      workspaceId: tenant.workspaceId,
      channel: "telegram",
      name: "Telegram",
      sourceId: uniqueDigits(),
    })
    expectError(
      await api.post("/v1/contact-scans", { inboxId: telegram.inboxId, scanFromAt: past }),
      400,
      "contactScanChannelUnsupported",
      { defined: true, message: "This channel does not support Automatic Customer Scan." },
    )
    // A Messenger inbox with no integration row behind it.
    const bare = await fixtures.createInbox({
      workspaceId: tenant.workspaceId,
      channel: "messenger",
      name: "Bare",
      sourceId: uniqueDigits(),
    })
    expectError(
      await api.post("/v1/contact-scans", { inboxId: bare.inboxId, scanFromAt: past }),
      400,
      "contactScanIntegrationDisconnected",
      { defined: true, message: "This inbox is not connected." },
    )
    // Another workspace's inbox is not found.
    const { api: other } = await tenantApi("p105-scan-refuse-other")
    expectError(
      await other.post("/v1/contact-scans", { inboxId: messenger.inboxId, scanFromAt: past }),
      404,
      "contactScanInboxNotFound",
    )
    expect((await api.get<RunList>("/v1/contact-scans")).json).toEqual({ data: [], pageCount: 0 })
  })

  it("validates the body (422)", async () => {
    const { api } = await tenantApi("p105-scan-body")
    expectInvalid(await api.post("/v1/contact-scans", {}), "inboxId")
    expectInvalid(await api.post("/v1/contact-scans", { inboxId: "1" }), "scanFromAt")
    expectInvalid(await api.post("/v1/contact-scans", { inboxId: "1", scanFromAt: "not a date" }), "scanFromAt")
    expectInvalid(await api.post("/v1/contact-scans", { inboxId: "x1", scanFromAt: isoFromNow(-DAY) }), "inboxId")
  })

  it(
    "schedules a run (202); the worker walks the conversations and imports the customers",
    async () => {
      const { tenant, api } = await tenantApi("p105-scan-run")
      const inbox = await messengerInbox(tenant.workspaceId)
      const customer = { psid: uniqueDigits(), name: `Ana Scan${uniqueId()}` }
      await scriptConversations(inbox.pageId, [customer])
      const since = await fake.cursor()
      const scanFromAt = isoFromNow(-7 * DAY)

      const created = await api.post<{ runId: string }>("/v1/contact-scans", { inboxId: inbox.inboxId, scanFromAt })
      expect(created.status).toBe(202)
      expect(created.json).toEqual({ runId: expect.stringMatching(/^\d+$/) })
      const { runId } = created.json

      // While the run is active a second scan is refused.
      const status = await api.get<Status>(`/v1/contact-scans/status${qs({ inboxId: inbox.inboxId })}`)
      expect(status.status).toBe(200)
      if (status.json.status === "init" || status.json.status === "running") {
        expect(status.json.availability).toEqual({
          canScan: false,
          blockedReason: "running",
          nextScanAt: expect.any(String),
        })
        expectError(
          await api.post("/v1/contact-scans", { inboxId: inbox.inboxId, scanFromAt }),
          409,
          "contactScanAlreadyRunning",
          { defined: true, message: "A scan is already running for this inbox." },
        )
      }
      expect(status.json.latest).toMatchObject({ id: runId, scanFromAt: new Date(scanFromAt).toISOString() })

      const listed = await api.get<RunList>("/v1/contact-scans")
      expect(listed.status).toBe(200)
      expect(listed.json.pageCount).toBe(1)
      expect(listed.json.data.map((run) => run.id)).toEqual([runId])
      expect(Object.keys(listed.json.data[0] as object).sort()).toEqual([
        "channel",
        "createdAt",
        "currentError",
        "currentScan",
        "finishedAt",
        "id",
        "importedContactCount",
        "requestedByUserId",
        "scanFromAt",
        "startedAt",
        "status",
      ])
      expect(listed.json.data[0]).toMatchObject({ channel: "messenger", requestedByUserId: null })

      const settled = await eventually(
        async () => {
          const response = await api.get<Status>(`/v1/contact-scans/status${qs({ inboxId: inbox.inboxId })}`)
          expect(response.json.status).toBe("succeeded")
          return response.json
        },
        { timeoutMs: 150_000, intervalMs: 1_000, description: "contact scan run finishes" },
      )
      const normalizer = new Normalizer()
      expect(normalizer.normalize(settled, { replace: { [runId]: "<run>" }, checkTimestamps: false })).toEqual({
        status: "succeeded",
        latest: {
          id: "<run>",
          status: "succeeded",
          scanFromAt: "<ts>",
          createdAt: "<ts>",
          startedAt: "<ts>",
          finishedAt: "<ts>",
          importedContactCount: 1,
          currentScan: expect.any(Number),
          currentError: null,
        },
        availability: { canScan: false, blockedReason: "cooldown", nextScanAt: "<ts>" },
      })
      const createdAt = Date.parse(settled.latest?.createdAt as string)
      expect(Date.parse(settled.availability.nextScanAt as string)).toBe(createdAt + DAY)

      const calls = await upstreamCalls({
        host: GRAPH,
        since,
        path: new RegExp(`^/v[0-9.]+/${reEscape(inbox.pageId)}/conversations$`),
      })
      expect(calls.length).toBeGreaterThanOrEqual(1)
      expect(calls[0]?.headers.authorization).toBe(`Bearer page-token-${inbox.pageId}`)
      expect(calls[0]?.query).toMatchObject({ platform: "MESSENGER" })

      const contacts = await api.get<{ data: Array<Record<string, unknown>> }>("/v1/contacts")
      expect(contacts.status).toBe(200)
      const [firstName] = customer.name.split(" ")
      expect(contacts.json.data.some((contact) => contact.firstName === firstName)).toBe(true)

      // The 24 h cooldown runs from the request time.
      expectError(
        await api.post("/v1/contact-scans", { inboxId: inbox.inboxId, scanFromAt }),
        409,
        "contactScanCooldown",
        { defined: true, message: "This inbox was scanned recently. Please wait before scanning again." },
      )
    },
    200_000,
  )
})

describe("contactScans.list", () => {
  it("lists the workspace's runs newest first, filters by integration and pages", async () => {
    const { tenant, api } = await tenantApi("p105-scan-list")
    const first = await messengerInbox(tenant.workspaceId)
    const second = await messengerInbox(tenant.workspaceId)
    await scriptConversations(first.pageId, [])
    await scriptConversations(second.pageId, [])
    const scanFromAt = isoFromNow(-DAY)
    const runA = (await api.post<{ runId: string }>("/v1/contact-scans", { inboxId: first.inboxId, scanFromAt })).json.runId
    const runB = (await api.post<{ runId: string }>("/v1/contact-scans", { inboxId: second.inboxId, scanFromAt })).json.runId

    const all = await api.get<RunList>("/v1/contact-scans")
    expect(all.json.data.map((run) => run.id)).toEqual([runB, runA])
    expect(all.json.pageCount).toBe(1)
    const filtered = await api.get<RunList>(`/v1/contact-scans${qs({ integrationId: first.integrationId })}`)
    expect(filtered.json.data.map((run) => run.id)).toEqual([runA])
    const paged = await api.get<RunList>(`/v1/contact-scans${qs({ perPage: 1, page: 2 })}`)
    expect(paged.json).toMatchObject({ pageCount: 2 })
    expect(paged.json.data.map((run) => run.id)).toEqual([runA])
    expectInvalid(await api.get(`/v1/contact-scans${qs({ perPage: 51 })}`), "perPage")
    expectInvalid(await api.get(`/v1/contact-scans${qs({ integrationId: "abc" })}`), "integrationId")

    const { api: other } = await tenantApi("p105-scan-list-other")
    expect((await other.get<RunList>("/v1/contact-scans")).json).toEqual({ data: [], pageCount: 0 })
  })
})
