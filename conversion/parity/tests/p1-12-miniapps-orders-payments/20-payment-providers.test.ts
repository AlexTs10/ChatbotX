// paymentProviders.list / getConnectLink / connectStripeKey / disconnect.
//
// Stripe is the fake upstream `api.stripe.com`: `GET /v1/account` checks the
// key, `POST /v1/webhook_endpoints` makes the shop's webhook, `DELETE
// /v1/webhook_endpoints/<id>` removes it. `/v1/account` is a shared path, so
// each script answers once (`times: 1`) and every test consumes its own; the
// webhook scripts match the workspace id in the body or a per-test endpoint id.
// The parity platform has no Stripe Connect credential: `stripeAvailable` is
// false.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  type Api,
  expectError,
  expectInvalid,
  ISO,
  keysOf,
  newTenant,
  parseForm,
  STRIPE,
  upstreamCalls,
} from "./helpers.ts"

type Provider = {
  provider: "stripe" | "paypal"
  connectionType: "oauth" | "apiKey"
  livemode: boolean
  accountLabel: string | null
  accountId: string | null
  connectedAt: string
}

const PROVIDER_KEYS = ["accountId", "accountLabel", "connectedAt", "connectionType", "livemode", "provider"]

/** Scripts one successful Stripe key check + webhook create for this workspace. */
const scriptStripeConnect = async (
  tenant: Tenant,
  input: { accountId: string; label?: string | null; chargesEnabled?: boolean; webhookId: string },
) => {
  await fake.script(
    {
      match: { host: STRIPE, method: "GET", path: "/v1/account" },
      respond: {
        status: 200,
        json: {
          id: input.accountId,
          object: "account",
          business_profile: { name: input.label === undefined ? "Parity Shop" : input.label },
          settings: { dashboard: { display_name: null } },
          email: null,
          charges_enabled: input.chargesEnabled ?? true,
        },
      },
      times: 1,
    },
    {
      match: { host: STRIPE, method: "POST", path: "/v1/webhook_endpoints", bodyIncludes: tenant.workspaceId },
      respond: {
        status: 200,
        json: { id: input.webhookId, object: "webhook_endpoint", secret: `whsec_${uniqueId()}`, status: "enabled" },
      },
      times: 1,
    },
  )
}

const scriptWebhookDelete = async (webhookId: string) => {
  await fake.script({
    match: { host: STRIPE, method: "DELETE", path: `/v1/webhook_endpoints/${webhookId}` },
    respond: { status: 200, json: { id: webhookId, object: "webhook_endpoint", deleted: true } },
    times: 1,
  })
}

const list = async (api: Api) => {
  const response = await api.get<{ data: Provider[]; stripeAvailable: boolean }>("/v1/payment-providers")
  expect(response.status, response.text).toBe(200)
  expect(keysOf(response.json)).toEqual(["data", "stripeAvailable"])
  return response.json
}

const connect = (api: Api, secretKey: string) =>
  api.post<Provider & { chargesEnabled: boolean }>("/v1/payment-providers/stripe/api-key", { secretKey })

describe("paymentProviders.list", () => {
  it("is empty for a new workspace; Stripe Connect is not set up on the parity platform", async () => {
    const { api } = await newTenant("p112-pp-list")
    expect(await list(api)).toEqual({ data: [], stripeAvailable: false })
  })
})

describe("paymentProviders.getConnectLink", () => {
  it("points at the provider row of Settings -> Integrations on the workspace's app URL", async () => {
    const { tenant, api } = await newTenant("p112-pp-link")
    for (const provider of ["stripe", "paypal"] as const) {
      const response = await api.get<{ url: string }>(`/v1/payment-providers/${provider}/connect-link`)
      expect(response.status, response.text).toBe(200)
      expect(response.json).toEqual({
        url: `http://app/space/${tenant.workspaceId}/settings/integrations/${provider}`,
      })
    }
    expectInvalid(await api.get("/v1/payment-providers/cod/connect-link"), "provider")
  })
})

describe("paymentProviders.connectStripeKey", () => {
  it("checks the key with Stripe, creates the webhook and lists the connection (never the key)", async () => {
    const { tenant, api } = await newTenant("p112-pp-connect")
    const accountId = `acct_${uniqueId()}`
    const webhookId = `we_${uniqueId()}`
    const secretKey = `sk_test_${uniqueId()}`
    await scriptStripeConnect(tenant, { accountId, webhookId, chargesEnabled: false })
    const since = await fake.cursor()

    const response = await connect(api, `  ${secretKey}  `)
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual([...PROVIDER_KEYS, "chargesEnabled"].sort())
    expect(response.json).toEqual({
      provider: "stripe",
      connectionType: "apiKey",
      livemode: false,
      accountLabel: "Parity Shop",
      accountId,
      connectedAt: expect.stringMatching(ISO),
      chargesEnabled: false,
    })
    expect(response.text).not.toContain(secretKey)

    const calls = await upstreamCalls({ host: STRIPE, since, bearer: secretKey })
    expect(calls.map((call) => `${call.method} ${call.path}`)).toEqual(["GET /v1/account", "POST /v1/webhook_endpoints"])
    const webhook = parseForm(calls[1] as (typeof calls)[number])
    expect(webhook).toMatchObject({
      url: `http://app/api/payments/stripe/workspace/${tenant.workspaceId}/webhook`,
      description: "Web shop payments",
      "enabled_events[0]": "checkout.session.completed",
      "enabled_events[1]": "checkout.session.async_payment_succeeded",
      "enabled_events[2]": "checkout.session.async_payment_failed",
      "enabled_events[3]": "checkout.session.expired",
    })
    expect(Object.keys(webhook).filter((key) => key.startsWith("enabled_events"))).toHaveLength(4)

    const listed = await list(api)
    expect(listed).toEqual({
      data: [
        {
          provider: "stripe",
          connectionType: "apiKey",
          livemode: false,
          accountLabel: "Parity Shop",
          accountId,
          connectedAt: response.json.connectedAt,
        },
      ],
      stripeAvailable: false,
    })
    expect(JSON.stringify(listed)).not.toContain(secretKey)

    // Connecting does not offer Stripe at checkout by itself.
    const settings = await api.get<{ payments: { paymentMethods: string[] } }>("/v1/ecommerce/settings")
    expect(settings.json.payments.paymentMethods).toEqual(["cod"])
  })

  it("a live key connects in live mode; the label falls back to null", async () => {
    const { tenant, api } = await newTenant("p112-pp-live")
    const accountId = `acct_${uniqueId()}`
    await scriptStripeConnect(tenant, { accountId, webhookId: `we_${uniqueId()}`, label: null })
    const response = await connect(api, `rk_live_${uniqueId()}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ livemode: true, accountLabel: null, accountId, chargesEnabled: true })
  })

  it("reconnecting the same account removes the old webhook with the new key", async () => {
    const { tenant, api } = await newTenant("p112-pp-reconnect")
    const accountId = `acct_${uniqueId()}`
    const firstWebhook = `we_${uniqueId()}`
    const secondWebhook = `we_${uniqueId()}`
    const firstKey = `sk_test_${uniqueId()}`
    const secondKey = `sk_test_${uniqueId()}`
    await scriptStripeConnect(tenant, { accountId, webhookId: firstWebhook })
    expect((await connect(api, firstKey)).status).toBe(200)

    await scriptStripeConnect(tenant, { accountId, webhookId: secondWebhook, label: "Renamed Shop" })
    await scriptWebhookDelete(firstWebhook)
    const since = await fake.cursor()
    const second = await connect(api, secondKey)
    expect(second.status, second.text).toBe(200)
    expect(second.json).toMatchObject({ accountId, accountLabel: "Renamed Shop" })
    const calls = await upstreamCalls({ host: STRIPE, since })
    expect(calls.map((call) => `${call.method} ${call.path} ${call.headers.authorization}`)).toEqual([
      `GET /v1/account Bearer ${secondKey}`,
      `POST /v1/webhook_endpoints Bearer ${secondKey}`,
      `DELETE /v1/webhook_endpoints/${firstWebhook} Bearer ${secondKey}`,
    ])
    const listed = await list(api)
    expect(listed.data).toHaveLength(1)
    expect(listed.data[0]).toMatchObject({ accountId, accountLabel: "Renamed Shop" })
  })

  it("refuses a publishable key without calling Stripe; 422 for an empty key", async () => {
    const { api } = await newTenant("p112-pp-pk")
    const since = await fake.cursor()
    expectError(await connect(api, `pk_test_${uniqueId()}`), 400, "stripeSecretKeyInvalid", {
      message:
        "Paste a Stripe secret key (sk_live_… or sk_test_…) or a restricted key (rk_…), not a publishable key (pk_…).",
    })
    expectError(await connect(api, "sk_test_has spaces"), 400, "stripeSecretKeyInvalid")
    expectInvalid(await connect(api, "   "), "secretKey")
    expectInvalid(await api.post("/v1/payment-providers/stripe/api-key", {}), "secretKey")
    expectInvalid(await connect(api, `sk_test_${"a".repeat(512)}`), "secretKey")
    expect(await upstreamCalls({ host: STRIPE, since, path: /^\/v1\/account$/ })).toHaveLength(0)
    expect((await list(api)).data).toEqual([])
  })

  it("a key Stripe refuses -> 400 paymentProviderCredentialsRejected and nothing is stored", async () => {
    const { api } = await newTenant("p112-pp-rejected")
    const secretKey = `sk_test_${uniqueId()}`
    await fake.script({
      match: { host: STRIPE, method: "GET", path: "/v1/account" },
      respond: {
        status: 401,
        json: { error: { message: "Invalid API Key provided: sk_test_****", type: "invalid_request_error" } },
      },
      times: 1,
    })
    const since = await fake.cursor()
    expectError(await connect(api, secretKey), 400, "paymentProviderCredentialsRejected", {
      message: "stripe refused the credentials. (provider=stripe, providerCode=)",
    })
    // No retry on a 401, and no webhook.
    expect(await upstreamCalls({ host: STRIPE, since, bearer: secretKey })).toHaveLength(1)
    expect((await list(api)).data).toEqual([])
  })
})

describe("paymentProviders.disconnect", () => {
  it("removes the webhook, the connection and the payment method; 204 with no body", async () => {
    const { tenant, api } = await newTenant("p112-pp-disconnect")
    const accountId = `acct_${uniqueId()}`
    const webhookId = `we_${uniqueId()}`
    const secretKey = `sk_test_${uniqueId()}`
    await scriptStripeConnect(tenant, { accountId, webhookId })
    expect((await connect(api, secretKey)).status).toBe(200)
    const offered = await api.patch<{ payments: { paymentMethods: string[] } }>("/v1/ecommerce/settings", {
      payments: { paymentMethods: ["cod", "stripe"] },
    })
    expect(offered.status, offered.text).toBe(200)
    expect(offered.json.payments.paymentMethods).toEqual(["cod", "stripe"])

    await scriptWebhookDelete(webhookId)
    const since = await fake.cursor()
    const response = await api.delete("/v1/payment-providers/stripe")
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    const calls = await upstreamCalls({ host: STRIPE, since })
    expect(calls.map((call) => `${call.method} ${call.path} ${call.headers.authorization}`)).toEqual([
      `DELETE /v1/webhook_endpoints/${webhookId} Bearer ${secretKey}`,
    ])
    expect((await list(api)).data).toEqual([])
    const settings = await api.get<{ payments: { paymentMethods: string[] } }>("/v1/ecommerce/settings")
    expect(settings.json.payments.paymentMethods).toEqual(["cod"])
  })

  it("does nothing (204) when the provider is not connected; 422 for an unknown provider", async () => {
    const { api } = await newTenant("p112-pp-disconnect-none")
    const since = await fake.cursor()
    for (const provider of ["stripe", "paypal"]) {
      const response = await api.delete(`/v1/payment-providers/${provider}`)
      expect(response.status, response.text).toBe(204)
    }
    expectInvalid(await api.delete("/v1/payment-providers/cod"), "provider")
    expect(await upstreamCalls({ host: STRIPE, since, path: /^\/v1\/webhook_endpoints/ })).toHaveLength(0)
  })

  it("still disconnects when Stripe refuses the webhook delete (best effort)", async () => {
    const { tenant, api } = await newTenant("p112-pp-disconnect-fail")
    const webhookId = `we_${uniqueId()}`
    await scriptStripeConnect(tenant, { accountId: `acct_${uniqueId()}`, webhookId })
    expect((await connect(api, `sk_test_${uniqueId()}`)).status).toBe(200)
    await fake.script({
      match: { host: STRIPE, method: "DELETE", path: `/v1/webhook_endpoints/${webhookId}` },
      respond: {
        status: 401,
        json: { error: { message: "Expired API Key provided", type: "invalid_request_error" } },
      },
      times: 1,
    })
    const response = await api.delete("/v1/payment-providers/stripe")
    expect(response.status, response.text).toBe(204)
    expect((await list(api)).data).toEqual([])
  })
})
