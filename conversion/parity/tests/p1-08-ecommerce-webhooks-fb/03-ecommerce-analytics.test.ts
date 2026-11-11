// ecommerceAnalytics.get — the sales dashboard over placed orders: lifetime
// sales, the period's sales/orders/items against the period of the same
// length just before, a per-hour (one day) or per-day series, and the top
// products and vendors. Orders are fixture rows with chosen `createdAt`
// (the public `orders.create` always stamps "now").

import { describe, expect, it } from "vitest"
import type { OrderRow } from "../../src/fixtures/ext/p1-08.ts"
import { fixtures } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { uniqueEmail, uniqueId } from "../../src/unique.ts"
import { type Api, expectInvalid, ext, keysOf, qs, tenantApi } from "./helpers.ts"

const RESOURCE_KEYS = [
  "currency",
  "from",
  "granularity",
  "lifetime",
  "orders",
  "otherCurrencyOrders",
  "products",
  "sales",
  "series",
  "timezone",
  "to",
  "topProducts",
  "topVendors",
]

type Dashboard = {
  currency: string
  otherCurrencyOrders: number
  from: string
  to: string
  timezone: string
  granularity: "hour" | "day"
  lifetime: { value: number; change: number | null }
  sales: { value: number; change: number | null }
  orders: { value: number; change: number | null }
  products: { value: number; change: number | null }
  series: Array<{ key: string; sales: number; orders: number }>
  topProducts: Array<{ productId: string | null; name: string; quantity: number; sales: number }>
  topVendors: Array<{ vendorId: string | null; name: string; sales: number }>
}

const order = (
  number: number,
  createdAt: string,
  total: number,
  items: Array<{ name: string; quantity: number; lineTotal: number; vendorName?: string | null }>,
  options: { status?: OrderRow["status"]; currency?: string } = {},
): OrderRow => ({
  number,
  status: options.status ?? "pending",
  currency: options.currency ?? "USD",
  total,
  createdAt: new Date(createdAt),
  items: items.map((item) => ({
    productId: null,
    name: item.name,
    quantity: item.quantity,
    unitPrice: item.lineTotal / item.quantity,
    lineTotal: item.lineTotal,
    vendorName: item.vendorName ?? null,
  })),
})

/** Orders around the period 2025-03-10..2025-03-12 (previous period: 03-07..03-09). */
const SHOP_ORDERS: OrderRow[] = [
  order(1, "2025-03-10T10:00:00.000Z", 100, [
    { name: "Widget", quantity: 2, lineTotal: 60, vendorName: "Acme" },
    { name: "Gadget", quantity: 1, lineTotal: 40 },
  ]),
  order(2, "2025-03-12T23:30:00.000Z", 50.5, [{ name: "Widget", quantity: 1, lineTotal: 50.5, vendorName: "Acme" }]),
  // Cancelled orders never count.
  order(3, "2025-03-11T09:00:00.000Z", 999, [{ name: "Widget", quantity: 9, lineTotal: 999, vendorName: "Acme" }], {
    status: "cancelled",
  }),
  // Another currency: only counted in otherCurrencyOrders.
  order(4, "2025-03-11T12:00:00.000Z", 70, [{ name: "Euro thing", quantity: 1, lineTotal: 70, vendorName: "EuroCo" }], {
    currency: "EUR",
    status: "concluded",
  }),
  // The previous period.
  order(5, "2025-03-08T08:00:00.000Z", 75, [{ name: "Gizmo", quantity: 3, lineTotal: 75, vendorName: "Beta" }], {
    status: "accepted",
  }),
  // After the period: lifetime only.
  order(6, "2025-03-20T08:00:00.000Z", 25, [{ name: "Widget", quantity: 1, lineTotal: 25, vendorName: "Acme" }]),
]

const dashboard = async (api: Api, params: Record<string, string | undefined> = {}): Promise<Dashboard> => {
  const response = await api.get<Dashboard>(`/v1/ecommerce/analytics${qs(params)}`)
  expect(response.status, response.text).toBe(200)
  expect(keysOf(response.json)).toEqual(RESOURCE_KEYS)
  return response.json
}

const shop = async (label: string) => {
  const { tenant, api } = await tenantApi(label)
  await (await ext()).insertOrders({ workspaceId: tenant.workspaceId, orders: SHOP_ORDERS })
  return { tenant, api }
}

describe("ecommerceAnalytics.get", () => {
  it("reports a day range: totals, changes, daily series and top lists", async () => {
    const { api } = await shop("p108-ea-days")
    expect(await dashboard(api, { from: "2025-03-10", to: "2025-03-12" })).toEqual({
      currency: "USD",
      otherCurrencyOrders: 1,
      from: "2025-03-10",
      to: "2025-03-12",
      timezone: "UTC",
      granularity: "day",
      // 250.5 lifetime; up to 03-12: 225.5, of which 150.5 in the period -> +200.7 %.
      lifetime: { value: 250.5, change: 200.7 },
      sales: { value: 150.5, change: 100.7 },
      orders: { value: 2, change: 100 },
      products: { value: 4, change: 33.3 },
      series: [
        { key: "2025-03-10", sales: 100, orders: 1 },
        { key: "2025-03-11", sales: 0, orders: 0 },
        { key: "2025-03-12", sales: 50.5, orders: 1 },
      ],
      topProducts: [
        { productId: null, name: "Widget", quantity: 3, sales: 110.5 },
        { productId: null, name: "Gadget", quantity: 1, sales: 40 },
      ],
      topVendors: [{ vendorId: null, name: "Acme", sales: 110.5 }],
    })
  })

  it("reports one day by hour; `to` defaults to `from`; null change when the previous day had nothing", async () => {
    const { api } = await shop("p108-ea-hours")
    const result = await dashboard(api, { from: "2025-03-12" })
    expect(result).toMatchObject({
      from: "2025-03-12",
      to: "2025-03-12",
      granularity: "hour",
      otherCurrencyOrders: 0,
      lifetime: { value: 250.5, change: 28.9 },
      sales: { value: 50.5, change: null },
      orders: { value: 1, change: null },
      products: { value: 1, change: null },
      topProducts: [{ productId: null, name: "Widget", quantity: 1, sales: 50.5 }],
      topVendors: [{ vendorId: null, name: "Acme", sales: 50.5 }],
    })
    expect(result.series).toHaveLength(24)
    expect(result.series[0]).toEqual({ key: "2025-03-12T00", sales: 0, orders: 0 })
    expect(result.series[23]).toEqual({ key: "2025-03-12T23", sales: 50.5, orders: 1 })
    expect(result.series.filter((point) => point.orders > 0)).toHaveLength(1)
  })

  it("counts only orders in the store currency", async () => {
    const { api } = await shop("p108-ea-currency")
    const saved = await api.patch("/v1/ecommerce/settings", { payments: { currency: "EUR", currencySymbol: "€" } })
    expect(saved.status, saved.text).toBe(200)
    expect(await dashboard(api, { from: "2025-03-10", to: "2025-03-12" })).toMatchObject({
      currency: "EUR",
      otherCurrencyOrders: 2,
      lifetime: { value: 70, change: null },
      sales: { value: 70, change: null },
      orders: { value: 1, change: null },
      products: { value: 1, change: null },
      topProducts: [{ productId: null, name: "Euro thing", quantity: 1, sales: 70 }],
      topVendors: [{ vendorId: null, name: "EuroCo", sales: 70 }],
    })
  })

  it("reads days in the workspace timezone", async () => {
    const driver = await fixtures()
    const { userId } = await driver.createUser({ email: uniqueEmail("p108-ea-tz"), password: "Parity#12345", name: "Parity tz" })
    const { workspaceId } = await driver.createWorkspace({
      ownerUserId: userId,
      name: `Parity WS ${uniqueId()}`,
      timezone: "Asia/Ho_Chi_Minh",
    })
    const { token } = await driver.createWorkspaceToken({ workspaceId, name: "parity", permission: "full", scopes: null })
    const api = apiClient(token)
    // 20:00 UTC on 03-12 is 03:00 on 03-13 in UTC+7.
    await (await ext()).insertOrders({
      workspaceId,
      orders: [order(1, "2025-03-12T20:00:00.000Z", 12, [{ name: "Late", quantity: 1, lineTotal: 12 }])],
    })
    const next = await dashboard(api, { from: "2025-03-13" })
    expect(next).toMatchObject({ timezone: "Asia/Ho_Chi_Minh", sales: { value: 12, change: null }, orders: { value: 1 } })
    expect(next.series.find((point) => point.orders > 0)?.key).toBe("2025-03-13T03")
    const before = await dashboard(api, { from: "2025-03-12" })
    expect(before).toMatchObject({ sales: { value: 0, change: 0 }, orders: { value: 0, change: 0 } })
  })

  it("reports an empty store with zero changes; defaults to today", async () => {
    const { api } = await tenantApi("p108-ea-empty")
    const before = new Date().toISOString().slice(0, 10)
    const result = await dashboard(api)
    const after = new Date().toISOString().slice(0, 10)
    expect([before, after]).toContain(result.from)
    expect(result).toMatchObject({
      currency: "USD",
      otherCurrencyOrders: 0,
      to: result.from,
      timezone: "UTC",
      granularity: "hour",
      lifetime: { value: 0, change: 0 },
      sales: { value: 0, change: 0 },
      orders: { value: 0, change: 0 },
      products: { value: 0, change: 0 },
      topProducts: [],
      topVendors: [],
    })
    expect(result.series).toHaveLength(24)
    expect(result.series.every((point) => point.sales === 0 && point.orders === 0)).toBe(true)
  })

  it("rejects a day that is not YYYY-MM-DD (422)", async () => {
    const { api } = await tenantApi("p108-ea-invalid")
    expectInvalid(await api.get(`/v1/ecommerce/analytics${qs({ from: "2025/03/10" })}`), "from")
    expectInvalid(await api.get(`/v1/ecommerce/analytics${qs({ from: "2025-03-10", to: "10-03-2025" })}`), "to")
  })
})
