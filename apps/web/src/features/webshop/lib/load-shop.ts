// The builder's `loadShop` runs in the Rust server (`pages.shop*`, crates/api
// shop_public.rs, `webshop::storefront`); this is the view it answers.
import type {
  CheckoutMode,
  PaymentMethod,
  ProductButtonType,
  ProductImageSize,
  ShippingMode,
  WebshopProductView,
} from "@chatbotx.io/database/partials"
import type { ShopMoneyFormat } from "./shop-money"

export type ShopView = {
  storeName: string
  brandColor: string
  locale: string
  ordering:
    | { open: true }
    | {
        open: false
        reason: "orderingUnavailable" | "outsideOpenHours"
        vendorName?: string
      }
  money: ShopMoneyFormat
  productView: WebshopProductView
  productImageSize: ProductImageSize
  productButtons: ProductButtonType[]
  allowRateOrders: boolean
  paymentMethods: PaymentMethod[]
  facebookPixelId: string | null
  checkout: {
    mode: CheckoutMode
    externalLink: string | null
    minimumOrderValue: number
    requireName: boolean
    requireEmail: boolean
    requirePhone: boolean
    requireState: boolean
    requireZip: boolean
    externalWebCheckout: boolean
    shopifyWebCheckout: boolean
    showDiscountCodes: boolean
  }
  shipping: {
    mode: ShippingMode
    storeAddress: string | null
    countries: string[]
    defaultCountry: string | null
    deliveryCost: number
  }
  categories: Array<{
    id: string
    parentId: string | null
    name: string
    imageUrl: string | null
  }>
  vendors: Array<{
    id: string
    name: string
    shortDescription: string | null
    imageUrl: string | null
  }>
}
