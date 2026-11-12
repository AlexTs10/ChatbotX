// orders.create / get / list / export / accept / ship / conclude / cancel /
// delete — cash-on-delivery orders placed through the API (source "api",
// actor "apiToken"). Prices come from the products (`products.create`), the
// store settings are the defaults (USD, delivery, delivery cost 0, no service
// fee). No order here has a contact, so no customer message is sent. Online
// payment (Stripe) is in 31-orders-online.

import { describe, expect, it } from "vitest"
import {
  type Api,
  createProduct,
  expectError,
  expectInvalid,
  expectNotFound,
  ISO,
  keysOf,
  newContact,
  newTenant,
  SNOWFLAKE,
  UNKNOWN_ID,
  uniqueName,
} from "./helpers.ts"

export const ORDER_KEYS = [
  "acceptedAt",
  "cancelReason",
  "cancelledAt",
  "channel",
  "concludedAt",
  "contactId",
  "conversationId",
  "couponDiscount",
  "createdAt",
  "currency",
  "customerEmail",
  "customerName",
  "customerPhone",
  "discountCode",
  "discountTotal",
  "externalId",
  "externalName",
  "fulfillmentStatus",
  "id",
  "inboxId",
  "note",
  "number",
  "paidAt",
  "paymentExpiresAt",
  "paymentMethod",
  "paymentStatus",
  "ratingComment",
  "rating",
  "serviceFeeTotal",
  "shippedAt",
  "shippingAddress",
  "shippingTotal",
  "shippingType",
  "source",
  "status",
  "subtotal",
  "taxTotal",
  "total",
  "trackingCompany",
  "trackingNumber",
  "trackingUrl",
  "updatedAt",
].sort()

const ITEM_KEYS = [
  "discountPercent",
  "discountTotal",
  "id",
  "imageUrl",
  "lineTotal",
  "name",
  "productId",
  "quantity",
  "retailerId",
  "sku",
  "taxPercent",
  "taxTotal",
  "unitPrice",
  "variantId",
  "variantLabel",
  "vendorId",
  "vendorName",
].sort()

type History = {
  id: string
  fromStatus: string | null
  toStatus: string
  actorType: string
  actorUser?: unknown
  note: string | null
  createdAt: string
}

export type Order = Record<string, unknown> & {
  id: string
  number: number
  status: string
  paymentMethod: string
  paymentStatus: string
  paymentExpiresAt: string | null
  total: number
  stockless?: never
  items: Array<Record<string, unknown> & { productId: string | null; quantity: number }>
  history: History[]
  payments: Array<Record<string, unknown>>
}

export const placeOrder = (api: Api, body: Record<string, unknown>) => api.post<Order>("/v1/orders", body)

const placeOk = async (api: Api, body: Record<string, unknown>): Promise<Order> => {
  const response = await placeOrder(api, body)
  expect(response.status, response.text).toBe(200)
  return response.json
}

const stockOf = async (api: Api, productId: string): Promise<number> => {
  const response = await api.get<{ inventoryQuantity: number }>(`/v1/products/${productId}`)
  expect(response.status, response.text).toBe(200)
  return response.json.inventoryQuantity
}

const transition = (api: Api, id: string, action: string, message?: string) =>
  api.post<Order>(`/v1/orders/${id}/${action}`, message === undefined ? {} : { message })

describe("orders.create", () => {
  it("prices the lines from the products and starts pending + unpaid (cash on delivery)", async () => {
    const { api } = await newTenant("p112-or-create")
    const first = await createProduct(api, { price: 12.5 })
    const second = await createProduct(api, { price: 3 })
    const response = await placeOrder(api, {
      items: [
        { productId: first.id, quantity: 2 },
        { productId: second.id, quantity: 1 },
      ],
      customerName: "  Ada Parity  ",
      customerEmail: "ada@parity.test",
      customerPhone: "+15550001111",
      note: "Ring twice",
      shippingAddress: { line1: "1 Main St", city: "Springfield", country: "US" },
    })
    expect(response.status, response.text).toBe(200)
    const order = response.json
    expect(keysOf(order)).toEqual([...ORDER_KEYS, "history", "items", "payments"].sort())
    expect(order.id).toMatch(SNOWFLAKE)
    expect(order).toMatchObject({
      number: 1,
      status: "pending",
      source: "api",
      paymentMethod: "cod",
      paymentStatus: "unpaid",
      paymentExpiresAt: null,
      paidAt: null,
      currency: "USD",
      subtotal: 28,
      discountTotal: 0,
      shippingTotal: 0,
      serviceFeeTotal: 0,
      taxTotal: 0,
      couponDiscount: 0,
      discountCode: null,
      total: 28,
      contactId: null,
      conversationId: null,
      inboxId: null,
      channel: null,
      customerName: "Ada Parity",
      customerEmail: "ada@parity.test",
      customerPhone: "+15550001111",
      shippingType: "delivery",
      shippingAddress: { line1: "1 Main St", city: "Springfield", country: "US" },
      note: "Ring twice",
      rating: null,
      ratingComment: null,
      cancelReason: null,
      acceptedAt: null,
      shippedAt: null,
      concludedAt: null,
      cancelledAt: null,
      externalId: null,
      externalName: null,
      fulfillmentStatus: null,
      trackingCompany: null,
      trackingNumber: null,
      trackingUrl: null,
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
      payments: [],
    })
    expect(order.items).toHaveLength(2)
    for (const item of order.items) {
      expect(keysOf(item)).toEqual(ITEM_KEYS)
    }
    expect(order.items[0]).toMatchObject({
      productId: first.id,
      variantId: null,
      vendorId: null,
      name: first.name,
      variantLabel: null,
      retailerId: null,
      imageUrl: null,
      vendorName: null,
      unitPrice: 12.5,
      quantity: 2,
      discountPercent: 0,
      taxPercent: 0,
      discountTotal: 0,
      taxTotal: 0,
      lineTotal: 25,
    })
    expect(order.items[1]).toMatchObject({ productId: second.id, name: second.name, unitPrice: 3, quantity: 1, lineTotal: 3 })
    expect(order.history).toEqual([
      {
        id: expect.stringMatching(SNOWFLAKE),
        fromStatus: null,
        toStatus: "pending",
        actorType: "apiToken",
        actorUser: null,
        note: null,
        createdAt: expect.stringMatching(ISO),
      },
    ])

    // Numbers count up per workspace; the same product twice is one line.
    const next = await placeOk(api, {
      items: [
        { productId: second.id, quantity: 1 },
        { productId: second.id, quantity: 2 },
      ],
    })
    expect(next.number).toBe(2)
    expect(next.items).toHaveLength(1)
    expect(next.items[0]).toMatchObject({ productId: second.id, quantity: 3, lineTotal: 9 })
    expect(next).toMatchObject({ shippingType: "delivery", shippingAddress: null, total: 9, customerName: null })

    // The default store ships by delivery only.
    expectError(
      await placeOrder(api, { items: [{ productId: second.id, quantity: 1 }], shippingType: "pickup" }),
      400,
      "orderShippingTypeUnavailable",
      { message: "The store does not offer pickup. (shippingType=pickup)" },
    )
  })

  it("takes stock from tracked products and refuses the whole order when one is short", async () => {
    const { api } = await newTenant("p112-or-stock")
    const tracked = await createProduct(api, { price: 5, inventoryPolicy: "track", inventoryQuantity: 3 })
    const untracked = await createProduct(api, { price: 1 })
    await placeOk(api, { items: [{ productId: tracked.id, quantity: 2 }] })
    expect(await stockOf(api, tracked.id)).toBe(1)

    expectError(
      await placeOrder(api, {
        items: [
          { productId: untracked.id, quantity: 1 },
          { productId: tracked.id, quantity: 2 },
        ],
      }),
      400,
      "orderOutOfStock",
      { message: `Not enough stock for "${tracked.name}". (productName=${tracked.name})` },
    )
    expect(await stockOf(api, tracked.id)).toBe(1)
    // Nothing was written: the next order still gets number 2.
    const next = await placeOk(api, { items: [{ productId: untracked.id, quantity: 1 }] })
    expect(next.number).toBe(2)
  })

  it("links a contact given as an identifier", async () => {
    const { api } = await newTenant("p112-or-contact")
    const product = await createProduct(api, { price: 2 })
    const contactId = await newContact(api)
    const order = await placeOk(api, { items: [{ productId: product.id, quantity: 1 }], contact: `id:${contactId}` })
    expect(order.contactId).toBe(contactId)
    expect(order.conversationId).toMatch(SNOWFLAKE)
    expect(order.inboxId).toMatch(SNOWFLAKE)
    expect(order.channel).toBe("webchat")
    expectNotFound(
      await placeOrder(api, { items: [{ productId: product.id, quantity: 1 }], contact: `id:${UNKNOWN_ID}` }),
      "Contact not found",
    )
  })

  it("refuses unknown, disabled and foreign products, online payment that is not offered, and bad input", async () => {
    const { api } = await newTenant("p112-or-refused")
    const { api: other } = await newTenant("p112-or-refused-other")
    const product = await createProduct(api, { price: 4 })
    const disabled = await createProduct(api, { price: 4 })
    expect((await api.patch(`/v1/products/${disabled.id}/active`, { isActive: false })).status).toBe(204)
    const foreign = await createProduct(other, { price: 4 })

    expectError(await placeOrder(api, { items: [{ productId: UNKNOWN_ID, quantity: 1 }] }), 400, "orderProductUnavailable", {
      message: `Product ${UNKNOWN_ID} is not available.`,
    })
    expectError(await placeOrder(api, { items: [{ productId: disabled.id, quantity: 1 }] }), 400, "orderProductUnavailable", {
      message: `Product ${disabled.id} is not available.`,
    })
    expectError(await placeOrder(api, { items: [{ productId: foreign.id, quantity: 1 }] }), 400, "orderProductUnavailable", {
      message: `Product ${foreign.id} is not available.`,
    })
    expectError(
      await placeOrder(api, { items: [{ productId: product.id, quantity: 1 }], paymentMethod: "stripe" }),
      400,
      "paymentMethodUnavailable",
      { message: "The store does not take stripe payments. (method=stripe)" },
    )
    expectInvalid(await placeOrder(api, { items: [] }), "items")
    expectInvalid(await placeOrder(api, {}), "items")
    expectInvalid(await placeOrder(api, { items: [{ productId: product.id, quantity: 0 }] }), "items.0.quantity")
    expectInvalid(await placeOrder(api, { items: [{ productId: product.id, quantity: 1.5 }] }), "items.0.quantity")
    expectInvalid(await placeOrder(api, { items: [{ productId: "abc", quantity: 1 }] }), "items.0.productId")
    expectInvalid(
      await placeOrder(api, { items: [{ productId: product.id, quantity: 1 }], customerEmail: "not-an-email" }),
      "customerEmail",
    )
    expectInvalid(await placeOrder(api, { items: [{ productId: product.id, quantity: 1 }], paymentMethod: "cash" }), "paymentMethod")
    expectInvalid(
      await placeOrder(api, { items: [{ productId: product.id, quantity: 1 }], shippingAddress: { country: "USA" } }),
      "shippingAddress.country",
    )
    const listed = await api.get<{ data: unknown[] }>("/v1/orders")
    expect(listed.json.data).toEqual([])
  })
})

describe("orders.get", () => {
  it("returns the detail; 404 for an unknown id or another workspace's order", async () => {
    const { api } = await newTenant("p112-or-get")
    const { api: other } = await newTenant("p112-or-get-other")
    const product = await createProduct(api, { price: 1 })
    const order = await placeOk(api, { items: [{ productId: product.id, quantity: 1 }] })
    const response = await api.get<Order>(`/v1/orders/${order.id}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual(order)
    expectNotFound(await other.get(`/v1/orders/${order.id}`), "Order does not exist.")
    expectNotFound(await api.get(`/v1/orders/${UNKNOWN_ID}`), "Order does not exist.")
    expectInvalid(await api.get("/v1/orders/abc"), "id")
  })
})

describe("orders.list", () => {
  it("is newest first (by number) with itemCount and pages; filters by status and keyword", async () => {
    const { api } = await newTenant("p112-or-list")
    const product = await createProduct(api, { price: 2 })
    const placed: Order[] = []
    for (let index = 0; index < 3; index += 1) {
      placed.push(
        await placeOk(api, {
          items: [{ productId: product.id, quantity: index + 1 }],
          customerName: index === 1 ? "Grace Hopper" : `Buyer ${index}`,
          customerEmail: index === 2 ? "lin@parity.test" : null,
        }),
      )
    }
    const [one, two, three] = placed as [Order, Order, Order]

    const page1 = await api.get<{ data: Array<Order & { itemCount: number }>; pageCount: number }>("/v1/orders", {
      query: { perPage: 2 },
    })
    expect(page1.status, page1.text).toBe(200)
    expect(keysOf(page1.json)).toEqual(["data", "pageCount"])
    expect(page1.json.pageCount).toBe(2)
    expect(page1.json.data.map((order) => order.id)).toEqual([three.id, two.id])
    expect(keysOf(page1.json.data[0])).toEqual([...ORDER_KEYS, "itemCount"].sort())
    expect(page1.json.data.map((order) => order.itemCount)).toEqual([3, 2])
    const page2 = await api.get<{ data: Order[]; pageCount: number }>("/v1/orders", { query: { perPage: 2, page: 2 } })
    expect(page2.json.data.map((order) => order.id)).toEqual([one.id])

    expect((await transition(api, two.id, "accept")).status).toBe(200)
    const accepted = await api.get<{ data: Order[] }>("/v1/orders?status[]=accepted")
    expect(accepted.status, accepted.text).toBe(200)
    expect(accepted.json.data.map((order) => order.id)).toEqual([two.id])
    const pendingOrAccepted = await api.get<{ data: Order[] }>("/v1/orders?status[]=pending&status[]=accepted")
    expect(pendingOrAccepted.json.data.map((order) => order.id)).toEqual([three.id, two.id, one.id])

    const byName = await api.get<{ data: Order[] }>("/v1/orders", { query: { keyword: "hopper" } })
    expect(byName.json.data.map((order) => order.id)).toEqual([two.id])
    const byEmail = await api.get<{ data: Order[] }>("/v1/orders", { query: { keyword: "lin@parity" } })
    expect(byEmail.json.data.map((order) => order.id)).toEqual([three.id])
    const byNumber = await api.get<{ data: Order[] }>("/v1/orders", { query: { keyword: `#${one.number}` } })
    expect(byNumber.json.data.map((order) => order.id)).toEqual([one.id])

    const future = await api.get<{ data: Order[] }>("/v1/orders", { query: { from: "2999-01-01T00:00:00.000Z" } })
    expect(future.json.data).toEqual([])
    const past = await api.get<{ data: Order[] }>("/v1/orders", { query: { to: "2000-01-01T00:00:00.000Z" } })
    expect(past.json.data).toEqual([])

    expectInvalid(await api.get("/v1/orders", { query: { perPage: 101 } }), "perPage")
    expectInvalid(await api.get("/v1/orders", { query: { page: 0 } }), "page")
    expectInvalid(await api.get("/v1/orders?status[]=done"), "status.0")
  })

  it("perPage up to 100 is accepted but a page holds at most 50 orders", async () => {
    const { api } = await newTenant("p112-or-list-clamp")
    const product = await createProduct(api, { price: 1 })
    for (let index = 0; index < 51; index += 1) {
      await placeOk(api, { items: [{ productId: product.id, quantity: 1 }] })
    }
    const page1 = await api.get<{ data: Order[]; pageCount: number }>("/v1/orders", { query: { perPage: 100 } })
    expect(page1.status, page1.text).toBe(200)
    expect(page1.json.data).toHaveLength(50)
    expect(page1.json.pageCount).toBe(2)
    expect(page1.json.data[0]?.number).toBe(51)
    const page2 = await api.get<{ data: Order[]; pageCount: number }>("/v1/orders", { query: { perPage: 100, page: 2 } })
    expect(page2.json.data.map((order) => order.number)).toEqual([1])
    const exported = await api.get<{ rowCount: number; truncated: boolean }>("/v1/orders/export")
    expect(exported.json).toMatchObject({ rowCount: 51, truncated: false })
  }, 60_000)

  it("filters by contact and never shows another workspace's orders", async () => {
    const { api } = await newTenant("p112-or-list-contact")
    const { api: other } = await newTenant("p112-or-list-contact-other")
    const product = await createProduct(api, { price: 2 })
    const contactId = await newContact(api)
    const withContact = await placeOk(api, { items: [{ productId: product.id, quantity: 1 }], contact: `id:${contactId}` })
    await placeOk(api, { items: [{ productId: product.id, quantity: 1 }] })
    const filtered = await api.get<{ data: Order[] }>("/v1/orders", { query: { contactId } })
    expect(filtered.json.data.map((order) => order.id)).toEqual([withContact.id])
    const foreign = await other.get<{ data: Order[]; pageCount: number }>("/v1/orders")
    expect(foreign.json).toEqual({ data: [], pageCount: 0 })
  })
})

describe("orders.export", () => {
  it("returns the list as CSV rows, newest first, with the same filters", async () => {
    const { api } = await newTenant("p112-or-export")
    const product = await createProduct(api, { price: 2.5 })
    const empty = await api.get<{ csv: string; rowCount: number; truncated: boolean }>("/v1/orders/export")
    expect(empty.status, empty.text).toBe(200)
    expect(empty.json).toEqual({
      csv: "Order,Created at,Status,Payment,Fulfillment,Customer,Email,Phone,Items,Currency,Total",
      rowCount: 0,
      truncated: false,
    })

    const first = await placeOk(api, {
      items: [{ productId: product.id, quantity: 2 }],
      customerName: "=SUM(A1)",
      customerEmail: "first@parity.test",
    })
    const second = await placeOk(api, {
      items: [{ productId: product.id, quantity: 1 }],
      customerName: "Lovelace, Ada",
      customerPhone: "+15550002222",
    })
    const response = await api.get<{ csv: string; rowCount: number; truncated: boolean }>("/v1/orders/export")
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(["csv", "rowCount", "truncated"])
    expect(response.json.rowCount).toBe(2)
    expect(response.json.truncated).toBe(false)
    const lines = response.json.csv.split("\r\n")
    expect(lines).toHaveLength(3)
    expect(lines[1]).toBe(
      `#2,${second.createdAt as string},pending,unpaid,,"Lovelace, Ada",,'+15550002222,1,USD,2.5`,
    )
    expect(lines[2]).toBe(`#1,${first.createdAt as string},pending,unpaid,,'=SUM(A1),first@parity.test,,2,USD,5`)

    const filtered = await api.get<{ rowCount: number }>("/v1/orders/export", { query: { keyword: "lovelace" } })
    expect(filtered.json.rowCount).toBe(1)
  })
})

describe("order status moves", () => {
  it("pending -> accepted -> shipped -> concluded; cash on delivery becomes paid on completion", async () => {
    const { api } = await newTenant("p112-or-flow")
    const product = await createProduct(api, { price: 6 })
    const order = await placeOk(api, { items: [{ productId: product.id, quantity: 1 }] })

    const accepted = await transition(api, order.id, "accept", "  On it  ")
    expect(accepted.status, accepted.text).toBe(200)
    expect(accepted.json).toMatchObject({ status: "accepted", acceptedAt: expect.stringMatching(ISO), paymentStatus: "unpaid" })
    const shipped = await transition(api, order.id, "ship")
    expect(shipped.status, shipped.text).toBe(200)
    expect(shipped.json).toMatchObject({ status: "shipped", shippedAt: expect.stringMatching(ISO) })
    const concluded = await transition(api, order.id, "conclude", "")
    expect(concluded.status, concluded.text).toBe(200)
    expect(concluded.json).toMatchObject({
      status: "concluded",
      concludedAt: expect.stringMatching(ISO),
      paymentStatus: "paid",
      cancelledAt: null,
    })
    expect(concluded.json.history.map((entry) => [entry.fromStatus, entry.toStatus, entry.actorType, entry.note])).toEqual([
      [null, "pending", "apiToken", null],
      ["pending", "accepted", "apiToken", "On it"],
      ["accepted", "shipped", "apiToken", null],
      ["shipped", "concluded", "apiToken", null],
    ])

    // Concluded is final.
    for (const action of ["accept", "ship", "conclude", "cancel"]) {
      const target = { accept: "accepted", ship: "shipped", conclude: "concluded", cancel: "cancelled" }[action]
      expectError(await transition(api, order.id, action), 409, "orderInvalidStatusTransition", {
        message: `An order that is concluded cannot become ${target}. (from=concluded, to=${target})`,
      })
    }
  })

  it("an accepted order can be concluded without shipping; a pending one cannot be shipped or concluded", async () => {
    const { api } = await newTenant("p112-or-skip")
    const product = await createProduct(api, { price: 6 })
    const order = await placeOk(api, { items: [{ productId: product.id, quantity: 1 }] })
    expectError(await transition(api, order.id, "ship"), 409, "orderInvalidStatusTransition", {
      message: "An order that is pending cannot become shipped. (from=pending, to=shipped)",
    })
    expectError(await transition(api, order.id, "conclude"), 409, "orderInvalidStatusTransition")
    expect((await transition(api, order.id, "accept")).status).toBe(200)
    expectError(await transition(api, order.id, "accept"), 409, "orderInvalidStatusTransition", {
      message: "An order that is accepted cannot become accepted. (from=accepted, to=accepted)",
    })
    const concluded = await transition(api, order.id, "conclude")
    expect(concluded.status, concluded.text).toBe(200)
    expect(concluded.json.shippedAt).toBeNull()
  })

  it("cancel keeps the reason, puts the stock back and is final", async () => {
    const { api } = await newTenant("p112-or-cancel")
    const product = await createProduct(api, { price: 3, inventoryPolicy: "track", inventoryQuantity: 5 })
    const order = await placeOk(api, { items: [{ productId: product.id, quantity: 4 }] })
    expect(await stockOf(api, product.id)).toBe(1)
    expect((await transition(api, order.id, "accept")).status).toBe(200)

    const cancelled = await transition(api, order.id, "cancel", "Out of delivery range")
    expect(cancelled.status, cancelled.text).toBe(200)
    expect(cancelled.json).toMatchObject({
      status: "cancelled",
      cancelReason: "Out of delivery range",
      cancelledAt: expect.stringMatching(ISO),
      paymentStatus: "unpaid",
      paymentExpiresAt: null,
    })
    expect(cancelled.json.history.at(-1)).toMatchObject({
      fromStatus: "accepted",
      toStatus: "cancelled",
      note: "Out of delivery range",
    })
    expect(await stockOf(api, product.id)).toBe(5)
    expectError(await transition(api, order.id, "cancel"), 409, "orderInvalidStatusTransition")
    expectInvalid(await transition(api, order.id, "cancel", "x".repeat(1001)), "message")
  })

  it("404 for an unknown order or another workspace's order", async () => {
    const { api } = await newTenant("p112-or-move-404")
    const { api: other } = await newTenant("p112-or-move-404-other")
    const product = await createProduct(other, { price: 1 })
    const foreign = await placeOk(other, { items: [{ productId: product.id, quantity: 1 }] })
    for (const action of ["accept", "ship", "conclude", "cancel"]) {
      expectNotFound(await transition(api, UNKNOWN_ID, action), "Order does not exist.")
      expectNotFound(await transition(api, foreign.id, action), "Order does not exist.")
    }
    const still = await other.get<Order>(`/v1/orders/${foreign.id}`)
    expect(still.json.status).toBe("pending")
  })
})

describe("orders.delete", () => {
  it("204; an open order puts its stock back, a cancelled one does not take it twice", async () => {
    const { api } = await newTenant("p112-or-delete")
    const product = await createProduct(api, { price: 3, inventoryPolicy: "track", inventoryQuantity: 4 })
    const open = await placeOk(api, { items: [{ productId: product.id, quantity: 3 }] })
    expect(await stockOf(api, product.id)).toBe(1)
    const response = await api.delete(`/v1/orders/${open.id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expect(await stockOf(api, product.id)).toBe(4)
    expectNotFound(await api.get(`/v1/orders/${open.id}`), "Order does not exist.")

    const cancelled = await placeOk(api, { items: [{ productId: product.id, quantity: 2 }] })
    expect((await transition(api, cancelled.id, "cancel")).status).toBe(200)
    expect(await stockOf(api, product.id)).toBe(4)
    expect((await api.delete(`/v1/orders/${cancelled.id}`)).status).toBe(204)
    expect(await stockOf(api, product.id)).toBe(4)

    const concluded = await placeOk(api, { items: [{ productId: product.id, quantity: 1 }] })
    expect((await transition(api, concluded.id, "accept")).status).toBe(200)
    expect((await transition(api, concluded.id, "conclude")).status).toBe(200)
    expect((await api.delete(`/v1/orders/${concluded.id}`)).status).toBe(204)
    expect(await stockOf(api, product.id)).toBe(3)
    const listed = await api.get<{ data: Order[] }>("/v1/orders")
    expect(listed.json.data).toEqual([])
  })

  it("404 for an unknown order or another workspace's order", async () => {
    const { api } = await newTenant("p112-or-delete-404")
    const { api: other } = await newTenant("p112-or-delete-404-other")
    const product = await createProduct(other, { price: 1 })
    const foreign = await placeOk(other, { items: [{ productId: product.id, quantity: 1 }] })
    expectNotFound(await api.delete(`/v1/orders/${UNKNOWN_ID}`), "Order does not exist.")
    expectNotFound(await api.delete(`/v1/orders/${foreign.id}`), "Order does not exist.")
    expect((await other.get(`/v1/orders/${foreign.id}`)).status).toBe(200)
  })
})

describe("orders.getPaymentLink (cash on delivery)", () => {
  it("409 orderPaymentNotOpen for an order that is not paid online; 404 for an unknown order", async () => {
    const { api } = await newTenant("p112-or-link-cod")
    const product = await createProduct(api, { price: 1 })
    const order = await placeOk(api, { items: [{ productId: product.id, quantity: 1 }], customerName: uniqueName("C") })
    expectError(await api.post(`/v1/orders/${order.id}/payment-link`, {}), 409, "orderPaymentNotOpen", {
      message: "This order cannot be paid online now. (reason=notPaidOnline)",
    })
    expectNotFound(await api.post(`/v1/orders/${UNKNOWN_ID}/payment-link`, {}), "Order does not exist.")
  })
})
