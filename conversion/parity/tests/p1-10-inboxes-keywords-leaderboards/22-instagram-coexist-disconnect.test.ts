// instagramChannels.getCoexistStatus / setCoexist (history sync) and
// instagramChannels.disconnect.
//
// setCoexist {enabled:true} switches coexist on and creates (or reuses) a sync
// run; the worker's `scanCoexistRuns` schedule (every minute) picks it up and
// pulls `GET /<igId>/conversations` from graph.instagram.com, scripted here to
// an empty page, so the run ends `succeeded`.
// disconnect unsubscribes the account at Meta (`DELETE /<pageId>/subscribed_apps`;
// graph.instagram.com with the account token for Instagram Login,
// graph.facebook.com with the app token `<clientId>|<clientSecret>` for
// Facebook Login, skipped when the page is still a Messenger page), then
// deletes the integration row and disconnects the inbox.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import {
  type Api,
  connectInstagram,
  expectError,
  expectInvalid,
  expectNotFound,
  ext,
  graphPath,
  type IgAccount,
  ISO,
  newTenant,
  SNOWFLAKE,
  UNKNOWN_ID,
  upstreamCalls,
  wait,
} from "./helpers.ts"

const IG_HOST = "graph.instagram.com"
const FB_HOST = "graph.facebook.com"

type CoexistRun = {
  id: string
  status: string
  startedAt: string | null
  finishedAt: string | null
  totalScan: number
  currentScan: number
  currentStep: string | null
  syncProgress: number
  importedContactCount: number
  importedMessageCount: number
  skippedCount: number
  failedCount: number
}

const RUN_KEYS = [
  "currentScan",
  "currentStep",
  "failedCount",
  "finishedAt",
  "id",
  "importedContactCount",
  "importedMessageCount",
  "skippedCount",
  "startedAt",
  "status",
  "syncProgress",
  "totalScan",
]

const coexistPath = (account: { integrationId: string }) => `/v1/instagram-channels/${account.integrationId}/coexist`

const status = async (api: Api, account: IgAccount): Promise<CoexistRun | null> => {
  const response = await api.get<{ run: CoexistRun | null }>(coexistPath(account))
  expect(response.status, response.text).toBe(200)
  return response.json.run
}

const coexistEnabled = async (api: Api, account: IgAccount): Promise<boolean> => {
  const response = await api.get<{ coexistEnabled: boolean }>(`/v1/instagram-channels/${account.integrationId}`)
  expect(response.status, response.text).toBe(200)
  return response.json.coexistEnabled
}

/** The history pull of `account` answers one empty page. */
const answerEmptyHistory = (account: IgAccount) =>
  fake.script({
    match: { host: IG_HOST, method: "GET", pathRegex: graphPath(`${account.igId}/conversations`) },
    respond: { status: 200, json: { data: [] } },
  })

describe("instagramChannels.getCoexistStatus / setCoexist", () => {
  it("run is null before any sync; 404 for an unknown or foreign account", async () => {
    const { tenant, api } = await newTenant("p110-coexist-none")
    const account = await connectInstagram(tenant, "instagram")
    expect(await status(api, account)).toBeNull()
    expectNotFound(await api.get(`/v1/instagram-channels/${UNKNOWN_ID}/coexist`), "Channel not found")
    const { api: otherApi } = await newTenant("p110-coexist-none-o")
    expectNotFound(await otherApi.get(coexistPath(account)), "Channel not found")
    expectInvalid(await api.get("/v1/instagram-channels/abc/coexist"), "id")
  })

  it(
    "enable turns coexist on and returns the run id (reused while live); the scheduler runs the sync to succeeded",
    async () => {
      const { tenant, api } = await newTenant("p110-coexist-on")
      const account = await connectInstagram(tenant, "instagram")
      await answerEmptyHistory(account)
      const since = await fake.cursor()
      const response = await api.put<{ success: boolean; runId: string }>(coexistPath(account), { enabled: true })
      expect(response.status, response.text).toBe(200)
      expect(response.json).toEqual({ success: true, runId: expect.stringMatching(SNOWFLAKE) })
      const again = await api.put<{ success: boolean; runId: string }>(coexistPath(account), {
        enabled: true,
        aiReadsSyncedHistory: true,
      })
      expect(again.json).toEqual({ success: true, runId: response.json.runId })
      expect(await coexistEnabled(api, account)).toBe(true)

      const first = await status(api, account)
      expect(Object.keys(first ?? {}).sort()).toEqual(RUN_KEYS)
      expect(first?.id).toBe(response.json.runId)

      const done = await wait(async () => {
        const run = await status(api, account)
        expect(run?.status).toBe("succeeded")
        return run as CoexistRun
      }, 110_000)
      expect(done).toMatchObject({
        id: response.json.runId,
        importedContactCount: 0,
        importedMessageCount: 0,
        skippedCount: 0,
        failedCount: 0,
        currentStep: "instagram page 1 processed",
      })
      expect(done.finishedAt).toMatch(ISO)

      const pulls = await upstreamCalls({ host: IG_HOST, since, path: new RegExp(graphPath(`${account.igId}/conversations`)) })
      expect(pulls.length).toBeGreaterThanOrEqual(1)
      expect(pulls[0]?.method).toBe("GET")
      expect(pulls[0]?.headers.authorization).toBe(`Bearer ig-token-${account.igId}`)
      expect(pulls[0]?.query).toMatchObject({ fields: "id,participants,updated_time" })
    },
    150_000,
  )

  it("disable turns coexist off (no run id); unknown accounts are 404; the body is validated", async () => {
    const { tenant, api } = await newTenant("p110-coexist-off")
    const account = await connectInstagram(tenant, "instagram")
    const response = await api.put(coexistPath(account), { enabled: false })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ success: true })
    expect(await coexistEnabled(api, account)).toBe(false)
    expect(await status(api, account)).toBeNull()

    expectNotFound(await api.put(`/v1/instagram-channels/${UNKNOWN_ID}/coexist`, { enabled: true }), "Channel not found")
    expectNotFound(await api.put(`/v1/instagram-channels/${UNKNOWN_ID}/coexist`, { enabled: false }), "Channel not found")
    expectInvalid(await api.put(coexistPath(account), {}), "enabled")
    expectInvalid(await api.put(coexistPath(account), { enabled: "yes" }), "enabled")
  })
})

const unsubscribeScript = (host: string, account: IgAccount, respond: { status: number; json: unknown }) =>
  fake.script({
    match: { host, method: "DELETE", pathRegex: graphPath(`${account.pageId}/subscribed_apps`) },
    respond,
  })

const unsubscribeCalls = (host: string, account: IgAccount, since: number) =>
  upstreamCalls({ host, since, path: new RegExp(graphPath(`${account.pageId}/subscribed_apps`)) })

const expectGone = async (api: Api, tenant: Tenant, account: IgAccount) => {
  expectNotFound(await api.get(`/v1/instagram-channels/${account.integrationId}`), "Channel integration not found")
  expect((await api.get("/v1/instagram-channels")).json).toEqual([])
  const inboxes = await api.get<{ data: Array<{ id: string }> }>("/v1/inboxes")
  expect(inboxes.json.data.map((inbox) => inbox.id)).not.toContain(account.inboxId)
  expect(tenant.workspaceId).toMatch(SNOWFLAKE)
}

describe("instagramChannels.disconnect", () => {
  it("Instagram Login: unsubscribes at graph.instagram.com with the account token, then removes the channel", async () => {
    const { tenant, api } = await newTenant("p110-ig-disc")
    const account = await connectInstagram(tenant, "instagram")
    await unsubscribeScript(IG_HOST, account, { status: 200, json: { success: true } })
    const since = await fake.cursor()
    const response = await api.delete(`/v1/instagram-channels/${account.integrationId}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    const calls = await unsubscribeCalls(IG_HOST, account, since)
    expect(calls).toHaveLength(1)
    expect(calls[0]?.headers.authorization).toBe(`Bearer ig-token-${account.igId}`)
    await expectGone(api, tenant, account)
    expectNotFound(await api.delete(`/v1/instagram-channels/${account.integrationId}`), "Instagram channel not found")
  })

  it("Facebook Login: unsubscribes the page at graph.facebook.com with the app token", async () => {
    const { tenant, api } = await newTenant("p110-ig-disc-fb")
    const account = await connectInstagram(tenant, "facebook")
    await unsubscribeScript(FB_HOST, account, { status: 200, json: { success: true } })
    const since = await fake.cursor()
    expect((await api.delete(`/v1/instagram-channels/${account.integrationId}`)).status).toBe(204)
    const calls = await unsubscribeCalls(FB_HOST, account, since)
    expect(calls).toHaveLength(1)
    expect(calls[0]?.headers.authorization).toBe("Bearer parity-client|parity-secret")
    await expectGone(api, tenant, account)
  })

  it("Facebook Login whose page is still a Messenger page: no Meta call", async () => {
    const { tenant, api } = await newTenant("p110-ig-disc-sibling")
    const account = await connectInstagram(tenant, "facebook")
    await (await ext()).createMessengerPage({ workspaceId: tenant.workspaceId, pageId: account.pageId, name: "Page" })
    const since = await fake.cursor()
    expect((await api.delete(`/v1/instagram-channels/${account.integrationId}`)).status).toBe(204)
    expect(await unsubscribeCalls(FB_HOST, account, since)).toEqual([])
    expectNotFound(await api.get(`/v1/instagram-channels/${account.integrationId}`), "Channel integration not found")
  })

  it("a revoked token at Meta still disconnects", async () => {
    const { tenant, api } = await newTenant("p110-ig-disc-revoked")
    const account = await connectInstagram(tenant, "instagram")
    await unsubscribeScript(IG_HOST, account, {
      status: 400,
      json: {
        error: {
          message: "Error validating access token: The user has not authorized application.",
          type: "OAuthException",
          code: 190,
          error_subcode: 458,
          fbtrace_id: "x",
        },
      },
    })
    expect((await api.delete(`/v1/instagram-channels/${account.integrationId}`)).status).toBe(204)
    await expectGone(api, tenant, account)
  })

  it("another Meta error fails the request (400) and keeps the channel", async () => {
    const { tenant, api } = await newTenant("p110-ig-disc-error")
    const account = await connectInstagram(tenant, "instagram")
    await unsubscribeScript(IG_HOST, account, {
      status: 400,
      json: { error: { message: "(#100) Invalid parameter", type: "OAuthException", code: 100, fbtrace_id: "x" } },
    })
    const response = await api.delete(`/v1/instagram-channels/${account.integrationId}`)
    expectError(response, 400, "BAD_REQUEST", { defined: false })
    const still = await api.get(`/v1/instagram-channels/${account.integrationId}`)
    expect(still.status, still.text).toBe(200)
  })

  it("404 for an unknown or foreign id; 422 for a non-numeric id", async () => {
    const { tenant, api } = await newTenant("p110-ig-disc-404")
    const account = await connectInstagram(tenant, "instagram")
    expectNotFound(await api.delete(`/v1/instagram-channels/${UNKNOWN_ID}`), "Instagram channel not found")
    const { api: otherApi } = await newTenant("p110-ig-disc-404-o")
    expectNotFound(await otherApi.delete(`/v1/instagram-channels/${account.integrationId}`), "Instagram channel not found")
    expectInvalid(await api.delete("/v1/instagram-channels/abc"), "id")
  })
})
