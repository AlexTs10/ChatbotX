// page: app/shop/[workspaceId]/page.tsx
// page: app/shop/[workspaceId]/categories/page.tsx
// page: app/shop/[workspaceId]/products/[productId]/page.tsx
// page: app/shop/[workspaceId]/cart/page.tsx
// page: app/shop/[workspaceId]/orders/[orderId]/page.tsx
// The public web shop (no session). The signed shop token is the authority:
// the Rust loaders `pages.shop*` (crates/api shop_public.rs) verify it and
// answer `null` for a bad, expired or foreign link; every price comes from
// the server.
import { Badge } from "@chatbotx.io/ui/components/ui/badge"
import { useSuspenseQuery } from "@tanstack/react-query"
import {
  ClockIcon,
  EyeIcon,
  LinkIcon,
  ShoppingCartIcon,
  StoreIcon,
} from "lucide-react"
import Link from "next/link"
import { useParams, useSearchParams } from "next/navigation"
import Script from "next/script"
import { useTranslations } from "next-intl"
import type { ReactNode } from "react"
import { CartView } from "@/features/webshop/components/cart-view"
import { CategoryGrid } from "@/features/webshop/components/category-grid"
import { FacebookPixel } from "@/features/webshop/components/facebook-pixel"
import { OrderView } from "@/features/webshop/components/order-view"
import { ProductDetail } from "@/features/webshop/components/product-detail"
import { ProductFilters } from "@/features/webshop/components/product-filters"
import { ProductList } from "@/features/webshop/components/product-list"
import { RelatedProducts } from "@/features/webshop/components/related-products"
import { ShopPagination } from "@/features/webshop/components/shop-pagination"
import { ShopProvider } from "@/features/webshop/components/shop-provider"
import type { ShopView } from "@/features/webshop/lib/load-shop"
import { shopBrandStyle } from "@/features/webshop/lib/shop-brand-style"
import { shopHref } from "@/features/webshop/lib/shop-href"
import type {
  ShopProduct,
  ShopProductDetail,
} from "@/features/webshop/lib/shop-product"
import { LocaleProvider } from "@/i18n/provider"
import { loadPage } from "@/lib/server-action"

type ShopPage = {
  shop: ShopView
  session: { mode: "customer" | "preview" }
  cartCount: number
}

/**
 * The signed shop token of the URL. A payment provider sends the customer
 * back with `shopToken`: PayPal appends its own `token` to the return URL.
 */
function useShopToken() {
  const search = useSearchParams()
  return search.get("shopToken") ?? search.get("token") ?? ""
}

function useWorkspaceId() {
  return useParams<{ workspaceId: string }>().workspaceId ?? ""
}

/** The page data, `null` for an invalid link. */
function useShopPage<T>(name: string, input: Record<string, unknown>) {
  const workspaceId = useWorkspaceId()
  const token = useShopToken()
  return useSuspenseQuery({
    queryKey: ["spa", `pages.${name}`, workspaceId, token, input],
    queryFn: () =>
      token
        ? loadPage<(ShopPage & T) | null>(name, {
            workspaceId,
            token,
            ...input,
          })
        : Promise.resolve(null),
  }).data
}

function ShopInvalidLink() {
  const t = useTranslations("webshop")
  return (
    <div className="flex min-h-screen items-center justify-center p-6 text-center">
      <div className="fade-in zoom-in-95 max-w-md animate-in space-y-3 duration-500">
        <span className="mx-auto flex size-14 items-center justify-center rounded-full bg-muted text-muted-foreground">
          <LinkIcon className="size-6" />
        </span>
        <h1 className="font-semibold text-2xl">{t("invalidLinkTitle")}</h1>
        <p className="text-muted-foreground text-sm">
          {t("invalidLinkDescription")}
        </p>
      </div>
    </div>
  )
}

function ShopHeader({ page }: { page: ShopPage }) {
  const t = useTranslations("webshop")
  const workspaceId = useWorkspaceId()
  const token = useShopToken()
  const { shop, cartCount } = page
  const closedReason = shop.ordering.open ? null : shop.ordering.reason
  return (
    <>
      <header className="sticky top-0 z-10 border-b bg-background/80 shadow-xs backdrop-blur-md">
        <div className="mx-auto flex h-16 max-w-5xl items-center justify-between gap-3 px-4">
          <Link
            className="group flex min-w-0 items-center gap-2.5 font-semibold text-lg"
            href={shopHref(workspaceId, token)}
          >
            <span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-primary text-primary-foreground shadow-sm transition-transform duration-200 group-hover:scale-105">
              <StoreIcon className="size-5" />
            </span>
            <span className="truncate">{shop.storeName}</span>
          </Link>
          <Link
            aria-label={t("cart")}
            className="relative flex items-center gap-2 rounded-full border bg-background px-3.5 py-2 font-medium text-sm shadow-xs transition-colors hover:bg-muted active:scale-95"
            href={shopHref(workspaceId, token, "/cart")}
          >
            <ShoppingCartIcon className="size-4" />
            <span className="hidden sm:inline">{t("cart")}</span>
            {cartCount > 0 ? (
              <Badge
                className="zoom-in-50 absolute -top-1.5 -right-1.5 h-5 min-w-5 animate-in px-1.5 tabular-nums shadow-sm duration-300"
                key={cartCount}
              >
                {cartCount}
              </Badge>
            ) : null}
          </Link>
        </div>
      </header>
      {page.session.mode === "preview" ? (
        <div className="border-b bg-amber-50 px-4 py-2 text-amber-900 text-sm dark:bg-amber-950 dark:text-amber-100">
          <div className="mx-auto flex max-w-5xl items-center gap-2">
            <EyeIcon className="size-4 shrink-0" />
            {t("previewBanner")}
          </div>
        </div>
      ) : null}
      {closedReason ? (
        <div className="border-b bg-muted px-4 py-2 text-sm">
          <div className="mx-auto flex max-w-5xl items-center gap-2">
            <ClockIcon className="size-4 shrink-0 text-muted-foreground" />
            {t(`closed.${closedReason}`)}
          </div>
        </div>
      ) : null}
    </>
  )
}

/**
 * Every shop page (`ShopLayout`): the store's name, the cart, a banner when
 * it is a preview or the store is closed, in the workspace language.
 */
function ShopLayout({
  page,
  children,
}: {
  page: ShopPage | null
  children: ReactNode
}) {
  const workspaceId = useWorkspaceId()
  const token = useShopToken()
  if (!page) {
    return <ShopInvalidLink />
  }
  const { shop } = page
  const preview = page.session.mode === "preview"
  const brandColorStyle = shopBrandStyle(shop.brandColor)
  return (
    <LocaleProvider locale={shop.locale}>
      <ShopProvider
        value={{
          workspaceId,
          token,
          money: shop.money,
          preview,
          open: shop.ordering.open,
          checkoutMode: shop.checkout.mode,
          externalWebCheckout: shop.checkout.externalWebCheckout,
          shopifyWebCheckout: shop.checkout.shopifyWebCheckout,
          productButtons: shop.productButtons,
          brandColorStyle,
        }}
      >
        {/* Lets the order page close Messenger's in-app browser. */}
        <Script
          src="https://connect.facebook.net/en_US/messenger.Extensions.js"
          strategy="afterInteractive"
        />
        {/* A merchant's preview is not a customer visit for the Pixel. */}
        <FacebookPixel pixelId={preview ? null : shop.facebookPixelId} />
        <div
          className="min-h-screen bg-gradient-to-b from-muted/60 to-muted/20"
          lang={shop.locale}
          style={brandColorStyle}
        >
          <ShopHeader page={page} />
          <main className="fade-in slide-in-from-bottom-1 mx-auto max-w-5xl animate-in px-4 py-5 duration-300 motion-reduce:animate-none">
            {children}
          </main>
        </div>
      </ShopProvider>
    </LocaleProvider>
  )
}

function NotFoundText({ messageKey }: { messageKey: string }) {
  const t = useTranslations("webshop")
  return (
    <p className="fade-in animate-in py-20 text-center text-muted-foreground duration-500">
      {t(messageKey)}
    </p>
  )
}

type HomeData = {
  products: { data: ShopProduct[]; pageCount: number }
  query: {
    keyword: string
    categoryId: string | null
    vendorId: string | null
    page: number
  }
}

/** The shop's front page: products, searchable, by category or vendor. */
export function ShopHomePage() {
  const search = useSearchParams()
  const page = useShopPage<HomeData>("shopHome", {
    q: search.get("q") ?? "",
    category: search.get("category"),
    vendor: search.get("vendor"),
    products: search.get("products"),
    page: search.get("page"),
  })
  if (!page) {
    return <ShopLayout page={null}>{null}</ShopLayout>
  }
  const { shop, products, query } = page
  const vendor = query.vendorId
    ? shop.vendors.find((candidate) => candidate.id === query.vendorId)
    : undefined
  const category = query.categoryId
    ? shop.categories.find((candidate) => candidate.id === query.categoryId)
    : undefined
  const heading = vendor ?? category
  return (
    <ShopLayout page={page}>
      <div className="space-y-5">
        {heading ? (
          <div className="flex items-center gap-4 rounded-2xl border bg-gradient-to-br from-primary/10 via-card to-card p-5 shadow-xs">
            {heading.imageUrl ? (
              // biome-ignore lint/performance/noImgElement: merchant images come from any host
              <img
                alt=""
                className="size-16 shrink-0 rounded-xl object-cover shadow-sm ring-2 ring-background"
                height={64}
                src={heading.imageUrl}
                width={64}
              />
            ) : null}
            <div className="min-w-0 space-y-1">
              <h1 className="truncate font-bold text-xl tracking-tight">
                {heading.name}
              </h1>
              {vendor?.shortDescription ? (
                <p className="line-clamp-2 text-muted-foreground text-sm">
                  {vendor.shortDescription}
                </p>
              ) : null}
            </div>
          </div>
        ) : null}
        <ProductFilters
          categories={shop.categories}
          categoryId={query.categoryId}
          keyword={query.keyword}
          vendorId={query.vendorId}
          vendors={shop.vendors}
        />
        <ProductList products={products.data} view={shop.productView} />
        <ShopPagination
          page={query.page}
          pageCount={products.pageCount}
          params={{
            q: query.keyword || null,
            category: query.categoryId,
            vendor: query.vendorId,
          }}
        />
      </div>
    </ShopLayout>
  )
}

/** The shop's visible categories; each card opens the shop on its products. */
export function ShopCategoriesPage() {
  const page = useShopPage<Record<string, never>>("shopCategories", {})
  return (
    <ShopLayout page={page}>
      {page ? <CategoryGrid categories={page.shop.categories} /> : null}
    </ShopLayout>
  )
}

export function ShopProductPage() {
  const { productId } = useParams<{ productId: string }>()
  const page = useShopPage<{
    product: ShopProductDetail | null
    related: ShopProduct[]
  }>("shopProduct", { productId })
  return (
    <ShopLayout page={page}>
      {page?.product ? (
        <ProductDetail
          imageSize={page.shop.productImageSize}
          product={page.product}
        >
          <RelatedProducts
            products={page.related}
            view={page.shop.productView}
          />
        </ProductDetail>
      ) : (
        <NotFoundText messageKey="productNotFound" />
      )}
    </ShopLayout>
  )
}

/**
 * The cart and its checkout; prices follow the delivery choice in the URL.
 * A preview shows all of it too — only placing the order is off.
 */
export function ShopCartPage() {
  const search = useSearchParams()
  const page = useShopPage<{ cart: Parameters<typeof CartView>[0]["cart"] }>(
    "shopCart",
    { shipping: search.get("shipping") },
  )
  return (
    <ShopLayout page={page}>
      {page ? <CartView cart={page.cart} shop={page.shop} /> : null}
    </ShopLayout>
  )
}

/** Only the customer's own order; anything else reads as not found. */
export function ShopOrderPage() {
  const { orderId } = useParams<{ orderId: string }>()
  const search = useSearchParams()
  const payment = search.get("payment")
  const page = useShopPage<{
    order: Parameters<typeof OrderView>[0]["order"] | null
  }>("shopOrder", { orderId, payment })
  const order = page?.order
  return (
    <ShopLayout page={page}>
      {page && order ? (
        <OrderView
          canRate={page.shop.allowRateOrders && order.status === "concluded"}
          order={order}
          paymentReturn={
            payment === "return" || payment === "cancelled" ? payment : null
          }
          placed={search.get("placed") === "1"}
        />
      ) : (
        <NotFoundText messageKey="orderNotFound" />
      )}
    </ShopLayout>
  )
}
