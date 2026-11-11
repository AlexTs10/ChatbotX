// facebookMarketingMessages.pages.list and .adAccounts.list, plus the
// Marketing Messages grant check every Graph-backed MM operation runs first
// (`requireValidGrant`, features/facebook-marketing-messages/lib/grant.ts).

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { CONNECT_FIRST, graphAdAccount, graphCalls, mmTenant, scriptAdAccounts, scriptDebugToken, uniqueAct } from "./helpers-mm.ts"
import { expectError, GRAPH, messengerPage, tenantApi } from "./helpers.ts"

const byPageId = (a: { pageId: string }, b: { pageId: string }) => a.pageId.localeCompare(b.pageId)

describe("facebookMarketingMessages.pages.list", () => {
  it("lists the workspace's connected Messenger Pages only", async () => {
    const { tenant, api } = await tenantApi("p108-mm-pages")
    const { tenant: other } = await tenantApi("p108-mm-pages-other")
    const pageA = await messengerPage(tenant.workspaceId, "MM Page A")
    const pageB = await messengerPage(tenant.workspaceId, "MM Page B")
    await messengerPage(other.workspaceId, "Foreign page")
    const since = await fake.cursor()

    const response = await api.get<{ pages: Array<{ pageId: string; name: string }> }>("/v1/marketing-messages/pages")
    expect(response.status, response.text).toBe(200)
    expect(Object.keys(response.json)).toEqual(["pages"])
    expect([...response.json.pages].sort(byPageId)).toEqual(
      [
        { pageId: pageA.pageId, name: "MM Page A" },
        { pageId: pageB.pageId, name: "MM Page B" },
      ].sort(byPageId),
    )
    // A database read only: no grant needed, no Graph call.
    expect(await fake.requests({ kind: "http", host: GRAPH, since })).toHaveLength(0)
  })

  it("returns an empty list without pages", async () => {
    const { api } = await tenantApi("p108-mm-pages-empty")
    const response = await api.get("/v1/marketing-messages/pages")
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ pages: [] })
  })
})

describe("facebookMarketingMessages.adAccounts.list", () => {
  it("lists the grant's ad accounts with the custom-audience ToS state", async () => {
    const mm = await mmTenant("p108-mm-accounts")
    const actA = uniqueAct()
    const actB = uniqueAct()
    const actC = uniqueAct()
    await scriptAdAccounts([
      graphAdAccount(actA, { name: "Shop A", currency: "USD" }),
      { ...graphAdAccount(actB, { name: null, currency: "VND" }), tos_accepted: { web_custom_audience_tos: 1 } },
      graphAdAccount(actC, { name: "No ToS", currency: "EUR", tos: false }),
    ])
    const since = await fake.cursor()

    const response = await mm.api.get<{ accounts: unknown[] }>("/v1/marketing-messages/ad-accounts")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      accounts: [
        {
          id: actA,
          accountId: actA.slice(4),
          name: "Shop A",
          currency: "USD",
          tosAccepted: true,
          tosUrl: `https://business.facebook.com/ads/manage/customaudiences/tos/?act=${actA.slice(4)}`,
        },
        {
          id: actB,
          accountId: actB.slice(4),
          name: actB,
          currency: "VND",
          tosAccepted: true,
          tosUrl: `https://business.facebook.com/ads/manage/customaudiences/tos/?act=${actB.slice(4)}`,
        },
        {
          id: actC,
          accountId: actC.slice(4),
          name: "No ToS",
          currency: "EUR",
          tosAccepted: false,
          tosUrl: `https://business.facebook.com/ads/manage/customaudiences/tos/?act=${actC.slice(4)}`,
        },
      ],
    })

    // The grant is checked with the owner's Messenger app token, then the
    // ad accounts are read with the grant's user token.
    const debug = (await graphCalls(since, "debug_token", "GET")).filter(
      (record) => record.query.input_token === mm.token,
    )
    expect(debug).toHaveLength(1)
    expect(debug[0]?.path).toBe("/v23.0/debug_token")
    expect(debug[0]?.headers.authorization).toBe(`Bearer ${mm.appId}|${mm.appSecret}`)
    expect(debug[0]?.query.access_token).toBeUndefined()

    const accounts = (await graphCalls(since, "me/adaccounts", "GET")).filter(
      (record) => record.query.access_token === mm.token,
    )
    expect(accounts).toHaveLength(1)
    expect(accounts[0]?.path).toBe("/v23.0/me/adaccounts")
    expect(accounts[0]?.query).toEqual({
      fields: "name,account_id,currency,timezone_name,tos_accepted",
      limit: "499",
      access_token: mm.token,
    })
  })

  it("is 400 mmGrantInvalid without a usable grant, with no Graph call", async () => {
    for (const grant of ["none", "invalid", "expired"] as const) {
      const mm = await mmTenant(`p108-mm-accounts-${grant}`, { grant, scriptDebugToken: false })
      const since = await fake.cursor()
      const response = await mm.api.get("/v1/marketing-messages/ad-accounts")
      expectError(response, 400, "mmGrantInvalid", { defined: false, message: CONNECT_FIRST })
      expect(await fake.requests({ kind: "http", host: GRAPH, since }), grant).toHaveLength(0)
    }
  })

  it("marks the grant invalid when Graph reports the token invalid or without the scope", async () => {
    for (const answer of [{ isValid: false, scopes: ["marketing_messages_messenger"] }, { isValid: true, scopes: ["ads_read"] }]) {
      const mm = await mmTenant("p108-mm-accounts-dead", { scriptDebugToken: false })
      await scriptDebugToken({ ...answer, times: 1 })
      const since = await fake.cursor()
      expectError(await mm.api.get("/v1/marketing-messages/ad-accounts"), 400, "mmGrantInvalid", {
        message: CONNECT_FIRST,
      })
      const calls = await fake.requests({ kind: "http", host: GRAPH, since })
      expect(calls.map((record) => record.path)).toEqual(["/v23.0/debug_token"])

      // The grant row is now `invalid`: the next call refuses without asking Graph.
      await scriptDebugToken({ isValid: true, scopes: ["marketing_messages_messenger"] })
      const again = await fake.cursor()
      expectError(await mm.api.get("/v1/marketing-messages/ad-accounts"), 400, "mmGrantInvalid")
      expect(await fake.requests({ kind: "http", host: GRAPH, since: again })).toHaveLength(0)
    }
  })

  it("treats a failed /debug_token as expired without marking the grant invalid", async () => {
    const mm = await mmTenant("p108-mm-accounts-outage", { scriptDebugToken: false })
    await scriptDebugToken({ status: 400, times: 1 })
    expectError(await mm.api.get("/v1/marketing-messages/ad-accounts"), 400, "mmGrantInvalid")

    await scriptDebugToken({ isValid: true, scopes: ["marketing_messages_messenger"] })
    const act = uniqueAct()
    await scriptAdAccounts([graphAdAccount(act)])
    const response = await mm.api.get<{ accounts: Array<{ id: string }> }>("/v1/marketing-messages/ad-accounts")
    expect(response.status, response.text).toBe(200)
    expect(response.json.accounts.map((account) => account.id)).toEqual([act])
  })

  it("passes a Graph error of the ad account read through as 400 BAD_REQUEST", async () => {
    const mm = await mmTenant("p108-mm-accounts-graph-error")
    await scriptAdAccounts([], { status: 400, times: 1 })
    const response = await mm.api.get("/v1/marketing-messages/ad-accounts")
    expectError(response, 400, "BAD_REQUEST")
  })
})
