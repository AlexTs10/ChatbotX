// ecommerceSettings.get / update — the store settings in four groups. A
// workspace that never saved any gets the defaults; a PATCH merges the sent
// fields into the stored (or default) settings and returns the full result.

import { describe, expect, it } from "vitest"
import { type Api, expectError, expectInvalid, tenantApi, uniqueName } from "./helpers.ts"

const DEFAULTS = {
  general: {
    storeSource: "chatbotx",
    orderingAvailable: true,
    webshopProductView: "list",
    openHoursId: null,
    searchProducts: false,
    facebookPixelId: null,
    abandonedCartDelayMinutes: null,
    abandonedCartFlowId: null,
    productButtons: ["addToCart", "share", "similar"],
    productImageSize: "bigSquare",
    allowRateOrders: false,
    notifyOnCartStatusChange: true,
  },
  payments: {
    paymentMethods: ["cod"],
    currency: "USD",
    currencyPosition: "left",
    currencySymbol: "$",
    serviceFeePercent: 0,
  },
  shipping: {
    shippingType: "delivery",
    storeAddress: null,
    shippingCountries: [],
    defaultShippingCountry: null,
    deliveryCost: 0,
  },
  checkout: {
    checkoutMode: "default",
    checkoutLink: null,
    checkoutFlowId: null,
    minimumOrderValue: 0,
    orderConfirmationMode: "default",
    orderConfirmationFlowId: null,
    requireName: true,
    requireEmail: false,
    requirePhone: true,
    requireState: false,
    requireZip: false,
    externalCheckoutMethod: "webCheckout",
    externalAutoApplyDiscounts: true,
  },
  proFeatures: { openHours: true, searchProducts: true, facebookPixel: true, abandonedCart: true },
  productsInOtherCurrency: 0,
}

type Settings = typeof DEFAULTS

const getSettings = async (api: Api): Promise<Settings> => {
  const response = await api.get<Settings>("/v1/ecommerce/settings")
  expect(response.status, response.text).toBe(200)
  return response.json
}

const patch = (api: Api, body: unknown) => api.patch<Settings>("/v1/ecommerce/settings", body)

/** A flow of the workspace (any flow counts for `flowService.exists`). */
const createFlow = async (api: Api): Promise<string> => {
  const name = uniqueName("Checkout flow")
  const response = await api.post<{ id: string }>("/v1/flows", {
    name,
    folderId: null,
    spec: { formatVersion: 1, name, steps: [{ type: "send", text: "Thanks" }] },
    publish: true,
  })
  expect(response.status, response.text).toBe(201)
  return response.json.id
}

describe("ecommerceSettings.get", () => {
  it("returns the defaults for a workspace that never saved settings", async () => {
    const { api } = await tenantApi("p108-es-defaults")
    expect(await getSettings(api)).toEqual(DEFAULTS)
  })
})

describe("ecommerceSettings.update", () => {
  it("merges one group, normalizes codes and keeps the other groups", async () => {
    const { api } = await tenantApi("p108-es-merge")
    const response = await patch(api, {
      payments: { currency: " vnd ", currencySymbol: "₫", currencyPosition: "rightSpace", serviceFeePercent: 2.5 },
    })
    expect(response.status, response.text).toBe(200)
    const expected = {
      ...DEFAULTS,
      payments: { paymentMethods: ["cod"], currency: "VND", currencySymbol: "₫", currencyPosition: "rightSpace", serviceFeePercent: 2.5 },
    }
    expect(response.json).toEqual(expected)
    expect(await getSettings(api)).toEqual(expected)

    // A second PATCH of another group keeps the saved payments.
    const second = await patch(api, {
      general: { productButtons: ["save", "addToCart"], webshopProductView: "cardSquare", abandonedCartDelayMinutes: 30 },
      shipping: { shippingCountries: ["vn", "us"], defaultShippingCountry: "us", deliveryCost: 3 },
    })
    expect(second.status, second.text).toBe(200)
    const merged = {
      ...expected,
      general: { ...DEFAULTS.general, productButtons: ["addToCart", "save"], webshopProductView: "cardSquare", abandonedCartDelayMinutes: 30 },
      shipping: { ...DEFAULTS.shipping, shippingCountries: ["VN", "US"], defaultShippingCountry: "US", deliveryCost: 3 },
    }
    expect(second.json).toEqual(merged)
    expect(await getSettings(api)).toEqual(merged)

    // An empty body changes nothing.
    const empty = await patch(api, {})
    expect(empty.status, empty.text).toBe(200)
    expect(empty.json).toEqual(merged)
  })

  it("accepts the deprecated Shopify checkout keys; the new names win when both are sent", async () => {
    const { api } = await tenantApi("p108-es-deprecated")
    const old = await patch(api, { checkout: { shopifyCheckoutMethod: "inApp", shopifyAutoApplyDiscounts: false } })
    expect(old.status, old.text).toBe(200)
    expect(old.json.checkout).toEqual({ ...DEFAULTS.checkout, externalCheckoutMethod: "inApp", externalAutoApplyDiscounts: false })
    expect(old.json.checkout).not.toHaveProperty("shopifyCheckoutMethod")

    const both = await patch(api, {
      checkout: {
        shopifyCheckoutMethod: "inApp",
        externalCheckoutMethod: "webCheckout",
        shopifyAutoApplyDiscounts: false,
        externalAutoApplyDiscounts: true,
      },
    })
    expect(both.json.checkout).toEqual(DEFAULTS.checkout)
  })

  it("saves the flow and external link checkout modes and pickup with a store address", async () => {
    const { api } = await tenantApi("p108-es-modes")
    const flowId = await createFlow(api)

    const link = await patch(api, { checkout: { checkoutMode: "externalLink", checkoutLink: "https://shop.example.com/pay" } })
    expect(link.status, link.text).toBe(200)
    expect(link.json.checkout).toMatchObject({ checkoutMode: "externalLink", checkoutLink: "https://shop.example.com/pay" })

    const flow = await patch(api, {
      checkout: { checkoutMode: "flow", checkoutFlowId: flowId, orderConfirmationMode: "flow", orderConfirmationFlowId: flowId },
      general: { abandonedCartFlowId: flowId },
    })
    expect(flow.status, flow.text).toBe(200)
    expect(flow.json.checkout).toMatchObject({
      checkoutMode: "flow",
      checkoutFlowId: flowId,
      checkoutLink: "https://shop.example.com/pay",
      orderConfirmationMode: "flow",
      orderConfirmationFlowId: flowId,
    })
    expect(flow.json.general.abandonedCartFlowId).toBe(flowId)

    const pickup = await patch(api, { shipping: { shippingType: "deliveryAndPickup", storeAddress: "  1 Main St  " } })
    expect(pickup.status, pickup.text).toBe(200)
    expect(pickup.json.shipping).toEqual({ ...DEFAULTS.shipping, shippingType: "deliveryAndPickup", storeAddress: "1 Main St" })
  })

  it("rejects incoherent settings with 422 validation and the field's message", async () => {
    const { api } = await tenantApi("p108-es-coherent")
    const cases: Array<[unknown, string]> = [
      [{ checkout: { checkoutMode: "externalLink" } }, "A checkout link is required for the external link mode."],
      [{ checkout: { checkoutMode: "flow" } }, "A checkout flow is required for the flow mode."],
      [{ checkout: { orderConfirmationMode: "flow" } }, "An order confirmation flow is required for the flow mode."],
      [
        { shipping: { shippingCountries: ["VN"], defaultShippingCountry: "US" } },
        "The default shipping country must be one of the shipping countries.",
      ],
      [{ shipping: { shippingType: "pickup" } }, "A store address is required for pickup."],
    ]
    for (const [body, message] of cases) {
      expectError(await patch(api, body), 422, "validation", { defined: true, message })
    }
    // An unknown flow id is refused by the template reference guard first
    // (404 "Flow not found"), before the service's own "does not exist" check.
    for (const body of [
      { checkout: { checkoutMode: "flow", checkoutFlowId: "1" } },
      { checkout: { orderConfirmationMode: "flow", orderConfirmationFlowId: "1" } },
      { general: { abandonedCartFlowId: "1" } },
    ]) {
      expectError(await patch(api, body), 404, "notFound", { defined: true, message: "Flow not found" })
    }
    // Nothing was saved.
    expect(await getSettings(api)).toEqual(DEFAULTS)
  })

  it("rejects an unknown open-hours schedule (404) and unconnected payment providers or stores (400)", async () => {
    const { api } = await tenantApi("p108-es-refs")
    expectError(await patch(api, { general: { openHoursId: "1" } }), 404, "notFound", {
      defined: true,
      message: "Open-hours schedule does not exist.",
    })
    expectError(await patch(api, { payments: { paymentMethods: ["cod", "stripe"] } }), 400, "paymentProviderNotConnected", {
      defined: true,
      message: "Connect stripe in Settings → Integrations first. (provider=stripe)",
    })
    expectError(await patch(api, { payments: { paymentMethods: ["paypal"] } }), 400, "paymentProviderNotConnected", {
      message: "Connect paypal in Settings → Integrations first. (provider=paypal)",
    })
    expectError(await patch(api, { general: { storeSource: "shopify" } }), 400, "externalStoreNotConnected", {
      defined: true,
      message: "Connect a Shopify store in Ecommerce → Settings → Shopify before selling from it. (store=shopify)",
    })
    expectError(await patch(api, { general: { storeSource: "woocommerce" } }), 400, "externalStoreNotConnected", {
      message: "Connect a WooCommerce store in Ecommerce → Settings → WooCommerce before selling from it. (store=woocommerce)",
    })
    expect(await getSettings(api)).toEqual(DEFAULTS)
  })

  it("rejects malformed fields (422 invalidRequestData)", async () => {
    const { api } = await tenantApi("p108-es-invalid")
    expectInvalid(await patch(api, { general: { abandonedCartDelayMinutes: 7 } }), "general.abandonedCartDelayMinutes")
    expectInvalid(await patch(api, { general: { productButtons: [] } }), "general.productButtons")
    expectInvalid(await patch(api, { general: { storeSource: "magento" } }), "general.storeSource")
    expectInvalid(await patch(api, { general: { openHoursId: "abc" } }), "general.openHoursId")
    expectInvalid(await patch(api, { payments: { currency: "US" } }), "payments.currency")
    expectInvalid(await patch(api, { payments: { paymentMethods: [] } }), "payments.paymentMethods")
    expectInvalid(await patch(api, { payments: { serviceFeePercent: 101 } }), "payments.serviceFeePercent")
    expectInvalid(await patch(api, { shipping: { deliveryCost: -1 } }), "shipping.deliveryCost")
    expectInvalid(await patch(api, { shipping: { defaultShippingCountry: "USA" } }), "shipping.defaultShippingCountry")
    expectInvalid(await patch(api, { checkout: { checkoutLink: "ftp://shop.example.com" } }), "checkout.checkoutLink")
    expectInvalid(await patch(api, { checkout: { minimumOrderValue: -5 } }), "checkout.minimumOrderValue")
    expectInvalid(await patch(api, { checkout: { checkoutMode: "popup" } }), "checkout.checkoutMode")
    expect(await getSettings(api)).toEqual(DEFAULTS)
  })
})
