// Orders paid online with Stripe (connected with a secret key): orders.create
// with paymentMethod "stripe", orders.getPaymentLink (Stripe Checkout
// Session), orders.accept while unpaid, orders.cancel (expires the session),
// and paymentProviders.disconnect refused while such an order is open.
//
// Stripe calls go to the fake upstream with per-test ids: the Checkout
// Session create matches the order id in its form body, the expire matches
// the per-test session id.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  type Api,
  createProduct,
  expectError,
  ISO,
  newContact,
  newTenant,
  parseForm,
  SNOWFLAKE,
  STRIPE,
  upstreamCalls,
} from "./helpers.ts"
import type { Order } from "./30-orders.test.ts"

type Store = { tenant: Tenant; api: Api; secretKey: string; webhookId: string; productId: string; contactId: string }

/** A workspace with Stripe connected by key, offered at checkout, one product (10 USD) and one contact. */
const stripeStore = async (label: string): Promise<Store> => {
  const { tenant, api } = await newTenant(label)
  const secretKey = `sk_test_${uniqueId()}`
  const webhookId = `we_${uniqueId()}`
  await fake.script(
    {
      match: { host: STRIPE, method: "GET", path: "/v1/account" },
      respond: {
        status: 200,
        json: { id: `acct_${uniqueId()}`, object: "account", business_profile: { name: "Parity Shop" }, charges_enabled: true },
      },
      times: 1,
    },
    {
      match: { host: STRIPE, method: "POST", path: "/v1/webhook_endpoints", bodyIncludes: tenant.workspaceId },
      respond: { status: 200, json: { id: webhookId, object: "webhook_endpoint", secret: `whsec_${uniqueId()}` } },
      times: 1,
    },
  )
  const connected = await api.post("/v1/payment-providers/stripe/api-key", { secretKey })
  expect(connected.status, connected.text).toBe(200)
  const settings = await api.patch("/v1/ecommerce/settings", { payments: { paymentMethods: ["cod", "stripe"] } })
  expect(settings.status, settings.text).toBe(200)
  const product = await createProduct(api, { price: 10 })
  const contactId = await newContact(api)
  return { tenant, api, secretKey, webhookId, productId: product.id, contactId }
}

const placeStripeOrder = async (store: Store, quantity = 1): Promise<Order> => {
  const response = await store.api.post<Order>("/v1/orders", {
    items: [{ productId: store.productId, quantity }],
    paymentMethod: "stripe",
    contact: `id:${store.contactId}`,
    customerEmail: "payer@parity.test",
  })
  expect(response.status, response.text).toBe(200)
  return response.json
}

const scriptSession = async (order: Order, sessionId: string) => {
  await fake.script({
    match: { host: STRIPE, method: "POST", path: "/v1/checkout/sessions", bodyIncludes: order.id },
    respond: {
      status: 200,
      json: {
        id: sessionId,
        object: "checkout.session",
        url: `https://checkout.stripe.com/c/pay/${sessionId}`,
        status: "open",
        payment_status: "unpaid",
      },
    },
    times: 1,
  })
}

describe("orders paid online (Stripe)", () => {
  it("an online order waits 30 minutes for its payment and cannot be accepted before it is paid", async () => {
    const store = await stripeStore("p112-on-create")
    const before = Date.now()
    const order = await placeStripeOrder(store, 2)
    expect(order).toMatchObject({
      status: "pending",
      paymentMethod: "stripe",
      paymentStatus: "unpaid",
      paidAt: null,
      total: 20,
      contactId: store.contactId,
      payments: [],
    })
    const expiresAt = Date.parse(order.paymentExpiresAt as string)
    expect(expiresAt - before).toBeGreaterThanOrEqual(30 * 60_000 - 5_000)
    expect(expiresAt - Date.now()).toBeLessThanOrEqual(30 * 60_000 + 1_000)

    expectError(await store.api.post(`/v1/orders/${order.id}/accept`, {}), 409, "orderAwaitingPayment", {
      message: "The order is waiting for its online payment.",
    })
    // Online payment needs a contact.
    expectError(
      await store.api.post("/v1/orders", { items: [{ productId: store.productId, quantity: 1 }], paymentMethod: "stripe" }),
      400,
      "orderPaymentContactRequired",
      { message: "An order paid online needs a contact (the customer)." },
    )
    // PayPal is not offered.
    expectError(
      await store.api.post("/v1/orders", {
        items: [{ productId: store.productId, quantity: 1 }],
        paymentMethod: "paypal",
        contact: `id:${store.contactId}`,
      }),
      400,
      "paymentMethodUnavailable",
    )
  })

  it("getPaymentLink opens a Stripe Checkout Session once and returns the open one again", async () => {
    const store = await stripeStore("p112-on-link")
    const order = await placeStripeOrder(store)
    const sessionId = `cs_test_${uniqueId()}`
    await scriptSession(order, sessionId)
    const since = await fake.cursor()

    const response = await store.api.post<{ url: string; expiresAt: string }>(`/v1/orders/${order.id}/payment-link`, {})
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      url: `https://checkout.stripe.com/c/pay/${sessionId}`,
      expiresAt: order.paymentExpiresAt,
    })

    const calls = await upstreamCalls({ host: STRIPE, since, path: /^\/v1\/checkout\/sessions$/ })
    expect(calls).toHaveLength(1)
    const call = calls[0] as (typeof calls)[number]
    expect(call.method).toBe("POST")
    expect(call.headers.authorization).toBe(`Bearer ${store.secretKey}`)
    expect(call.headers["stripe-account"]).toBeUndefined()
    const form = parseForm(call)
    expect(form).toMatchObject({
      mode: "payment",
      "payment_method_types[0]": "card",
      "line_items[0][quantity]": "1",
      "line_items[0][price_data][currency]": "usd",
      "line_items[0][price_data][unit_amount]": "1000",
      "line_items[0][price_data][product_data][name]": `${store.tenant.workspaceName} #${order.number}`,
      customer_email: "payer@parity.test",
      "metadata[workspaceId]": store.tenant.workspaceId,
      "metadata[orderId]": order.id,
      "payment_intent_data[metadata][orderId]": order.id,
    })
    expect(form["metadata[orderPaymentId]"]).toMatch(SNOWFLAKE)
    expect(form.client_reference_id).toBe(form["metadata[orderPaymentId]"])
    expect(call.headers["idempotency-key"]).toBe(`order-payment-${form["metadata[orderPaymentId]"]}`)
    expect(form.success_url).toMatch(/^http:\/\/app\//)
    expect(form.cancel_url).toMatch(/^http:\/\/app\//)
    expect(Number(form.expires_at) * 1000).toBeGreaterThanOrEqual(Date.parse(order.paymentExpiresAt as string) - 1_000)

    const detail = await store.api.get<Order>(`/v1/orders/${order.id}`)
    expect(detail.json.payments).toEqual([
      {
        id: form["metadata[orderPaymentId]"],
        provider: "stripe",
        status: "open",
        amount: 10,
        currency: "USD",
        livemode: false,
        providerReference: sessionId,
        transactionId: null,
        refundId: null,
        expiresAt: expect.stringMatching(ISO),
        paidAt: null,
        refundedAt: null,
        createdAt: expect.stringMatching(ISO),
      },
    ])

    // The open page is returned again without a new Stripe call.
    const again = await store.api.post<{ url: string }>(`/v1/orders/${order.id}/payment-link`, {})
    expect(again.status, again.text).toBe(200)
    expect(again.json.url).toBe(response.json.url)
    expect(await upstreamCalls({ host: STRIPE, since, path: /^\/v1\/checkout\/sessions$/ })).toHaveLength(1)
  })

  it("the provider cannot be disconnected while the order waits; cancel expires the session, then it can", async () => {
    const store = await stripeStore("p112-on-cancel")
    const order = await placeStripeOrder(store)
    const sessionId = `cs_test_${uniqueId()}`
    await scriptSession(order, sessionId)
    expect((await store.api.post(`/v1/orders/${order.id}/payment-link`, {})).status).toBe(200)

    expectError(await store.api.delete("/v1/payment-providers/stripe"), 409, "paymentProviderInUse", {
      message:
        "1 open order(s) were paid with stripe or are waiting for it. Complete or cancel them first. (provider=stripe, count=1)",
    })
    // Cancelling closes the open Checkout Session (expire), then the order no longer needs Stripe.
    await fake.script({
      match: { host: STRIPE, method: "POST", path: `/v1/checkout/sessions/${sessionId}/expire` },
      respond: {
        status: 200,
        json: { id: sessionId, object: "checkout.session", status: "expired", payment_status: "unpaid", url: null },
      },
      times: 1,
    })
    const since = await fake.cursor()
    const cancelled = await store.api.post<Order>(`/v1/orders/${order.id}/cancel`, { message: "Changed my mind" })
    expect(cancelled.status, cancelled.text).toBe(200)
    expect(cancelled.json).toMatchObject({
      status: "cancelled",
      paymentStatus: "unpaid",
      paymentExpiresAt: null,
      cancelReason: "Changed my mind",
    })
    expect(cancelled.json.payments).toHaveLength(1)
    expect(cancelled.json.payments[0]).toMatchObject({ status: "expired", providerReference: sessionId })
    const expireCalls = await upstreamCalls({ host: STRIPE, since })
    expect(expireCalls.map((call) => `${call.method} ${call.path} ${call.headers.authorization}`)).toEqual([
      `POST /v1/checkout/sessions/${sessionId}/expire Bearer ${store.secretKey}`,
    ])
    expectError(await store.api.post(`/v1/orders/${order.id}/payment-link`, {}), 409, "orderPaymentNotOpen", {
      message: "This order cannot be paid online now. (reason=expired)",
    })

    await fake.script({
      match: { host: STRIPE, method: "DELETE", path: `/v1/webhook_endpoints/${store.webhookId}` },
      respond: { status: 200, json: { id: store.webhookId, object: "webhook_endpoint", deleted: true } },
      times: 1,
    })
    const disconnected = await store.api.delete("/v1/payment-providers/stripe")
    expect(disconnected.status, disconnected.text).toBe(204)
    const settings = await store.api.get<{ payments: { paymentMethods: string[] } }>("/v1/ecommerce/settings")
    expect(settings.json.payments.paymentMethods).toEqual(["cod"])
  })

  it("getPaymentLink reports a Stripe refusal as 400 paymentProviderRejected", async () => {
    const store = await stripeStore("p112-on-refused")
    const order = await placeStripeOrder(store)
    await fake.script({
      match: { host: STRIPE, method: "POST", path: "/v1/checkout/sessions", bodyIncludes: order.id },
      respond: {
        status: 400,
        json: { error: { type: "invalid_request_error", code: "parameter_invalid", message: "Parity refused." } },
      },
      times: 1,
    })
    expectError(await store.api.post(`/v1/orders/${order.id}/payment-link`, {}), 400, "paymentProviderRejected", {
      message: "Parity refused. (provider=stripe, providerCode=parameter_invalid)",
    })
    const detail = await store.api.get<Order>(`/v1/orders/${order.id}`)
    expect(detail.json.payments).toEqual([])
  })
})
