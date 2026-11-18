// page: app/space/[workspaceId]/(e-commerce)/orders/page.tsx
// page: app/space/[workspaceId]/(e-commerce)/orders/[orderId]/page.tsx
// page: app/space/[workspaceId]/(e-commerce)/discount-codes/page.tsx
// page: app/space/[workspaceId]/(e-commerce)/discount-codes/new/page.tsx
// page: app/space/[workspaceId]/(e-commerce)/discount-codes/[discountCodeId]/page.tsx
// page: app/space/[workspaceId]/(e-commerce)/ecommerce-settings/page.tsx
// page: app/space/[workspaceId]/(e-commerce)/ecommerce-settings/[section]/page.tsx
// page: app/space/[workspaceId]/(e-commerce)/ecommerce-analytics/page.tsx
// page: app/space/[workspaceId]/(e-commerce)/layout.tsx
// page: app/space/[workspaceId]/(e-commerce)/products/page.tsx
// page: app/space/[workspaceId]/(e-commerce)/product-categories/page.tsx
// page: app/space/[workspaceId]/(e-commerce)/product-vendors/page.tsx
// page: app/space/[workspaceId]/products/layout.tsx
// page: app/space/[workspaceId]/products/create/page.tsx
// page: app/space/[workspaceId]/products/[id]/edit/page.tsx
// The `ecommerce` permission of both layouts (404). Server data: the page
// queries `ecommerceShareLinks`, `productsPage`, `productEditPage`,
// `productCategoriesPage`, `productVendorsPage` (Rust:
// crates/api/src/rpc_procedures/products.rs), `ordersPage`, `orderDetailPage`,
// `discountCodesPage`, `discountCodeFormPage`, `ecommerceSettingsPage`,
// `ecommerceAnalyticsPage` (crates/api/src/rpc_procedures/ecommerce.rs).
import { isNumericId } from "@chatbotx.io/utils"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Navigate, Outlet, useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { Suspense } from "react"
import { DiscountCodeForm } from "@/features/discount-codes/components/discount-code-form"
import { DiscountCodesTable } from "@/features/discount-codes/discount-codes-table"
import { listDiscountCodesSearchParams } from "@/features/discount-codes/schema/query"
import { AnalyticsDashboard } from "@/features/ecommerce-analytics/components/analytics-dashboard"
import { CheckoutSettingsForm } from "@/features/ecommerce-settings/components/checkout-settings-form"
import { EcommerceSettingsSidebar } from "@/features/ecommerce-settings/components/ecommerce-settings-sidebar"
import { GeneralSettingsForm } from "@/features/ecommerce-settings/components/general-settings-form"
import { PaymentSettingsForm } from "@/features/ecommerce-settings/components/payment-settings-form"
import { ShippingSettingsForm } from "@/features/ecommerce-settings/components/shipping-settings-form"
import {
  type EcommerceSettingsSection,
  ecommerceSettingsSections,
  externalStoreHiddenSections,
} from "@/features/ecommerce-settings/lib/sections"
import type { EcommerceSettingsPublicResource } from "@/features/ecommerce-settings/schema/public"
import { OrderDetail } from "@/features/orders/components/order-detail"
import { OrdersTable } from "@/features/orders/orders-table"
import { listOrdersSearchParams } from "@/features/orders/schema/query"
import { ShopifySettingsForm } from "@/features/shopify/components/shopify-settings-form"
import { WooCommerceSettingsForm } from "@/features/woocommerce/components/woocommerce-settings-form"
import { CategorySidebar } from "@/features/product-categories/components/category-sidebar"
import { ProductCategoriesTable } from "@/features/product-categories/product-categories-table"
import { listProductCategoriesSearchParams } from "@/features/product-categories/schema/query"
import { ProductVendorsTable } from "@/features/product-vendors/product-vendors-table"
import { listProductVendorsSearchParams } from "@/features/product-vendors/schema/query"
import { CreateProduct } from "@/features/products/components/create-product"
import { EcommerceFooterLayout } from "@/features/products/components/ecommerce-footer"
import { EcommerceTabs } from "@/features/products/components/ecommerce-tabs"
import { EditProduct } from "@/features/products/components/edit-product"
import { ProductsTable } from "@/features/products/products-table"
import { listProductsSearchParams } from "@/features/products/schema/query"
import { LockedEditorGuard } from "@/features/templates/components/locked-editor-guard"
import { ShopLinkSection } from "@/features/webshop/components/shop-link-section"
import { hasWorkspacePermission } from "@/lib/auth/permission-routes"
import { serverQuery } from "@/lib/server-action"
import { NotFoundPage } from "@/routes/root"
import { useWorkspacePage } from "@/routes/workspace"

function useWorkspaceId() {
  return (useParams({ strict: false }) as { workspaceId: string }).workspaceId
}

/** A page query keyed by the workspace and the search string. */
function usePageQuery<T>(name: string, input: unknown) {
  const workspaceId = useWorkspaceId()
  return useSuspenseQuery({
    queryKey: ["spa", name, workspaceId, JSON.stringify(input)],
    queryFn: () => serverQuery<T>(name, workspaceId, [workspaceId, input]),
  }).data
}

/** A nuqs cache's parse of the URL (`parse` is overloaded: `all` gives the type). */
function useSearch<T>(cache: {
  all: () => T
  parse: (input: Record<string, string>) => unknown
}) {
  const searchParams = useSearchParams()
  return cache.parse(Object.fromEntries(searchParams)) as T
}

function useEcommerceAllowed() {
  const { permissions } = useWorkspacePage()
  return hasWorkspacePermission(permissions, "ecommerce")
}

export function EcommerceLayout() {
  const workspaceId = useWorkspaceId()
  if (!useEcommerceAllowed()) {
    return <NotFoundPage />
  }

  return (
    <EcommerceFooterLayout shopLink={<EcommerceShopLink />}>
      <EcommerceTabs workspaceId={workspaceId} />

      <Suspense fallback={null}>
        <Outlet />
      </Suspense>
    </EcommerceFooterLayout>
  )
}

function EcommerceShopLink() {
  const links = usePageQuery<{
    previewUrl: string
    buttonUrls: { shop: string; cart: string; categories: string }
  }>("ecommerceShareLinks", {})
  return (
    <ShopLinkSection
      buttonUrls={links.buttonUrls}
      previewUrl={links.previewUrl}
    />
  )
}

type ProductsPageData = {
  products: never
  categories: never
  importHistory: never
  metaCatalog: never
  externalStoreLinks: null
}

export function ProductsPage() {
  const workspaceId = useWorkspaceId()
  const search = useSearch(listProductsSearchParams)
  const data = usePageQuery<ProductsPageData>("productsPage", search)

  return (
    <div className="flex min-h-0 flex-1">
      <CategorySidebar categories={data.categories} workspaceId={workspaceId} />
      <div className="min-w-0 flex-1 p-4">
        <ProductsTable
          externalStore={
            data.externalStoreLinks
              ? { store: data.externalStoreLinks }
              : undefined
          }
          importHistoryPromise={Promise.resolve([data.importHistory])}
          initialMetaCatalogState={data.metaCatalog}
          promises={Promise.resolve([data.products])}
          workspaceId={workspaceId}
        />
      </div>
    </div>
  )
}

export function ProductCategoriesPage() {
  const workspaceId = useWorkspaceId()
  const search = useSearch(listProductCategoriesSearchParams)
  const { parent, ...categories } = usePageQuery<{
    data: never
    pageCount: number
    parent: { id: string; name: string } | null
  }>("productCategoriesPage", search)

  return (
    <ProductCategoriesTable
      categories={categories}
      key={parent?.id ?? "root"}
      parent={parent}
      workspaceId={workspaceId}
    />
  )
}

export function ProductVendorsPage() {
  const workspaceId = useWorkspaceId()
  const search = useSearch(listProductVendorsSearchParams)
  const data = usePageQuery<{
    vendors: never
    categories: never
    openHours: never
  }>("productVendorsPage", search)

  return (
    <ProductVendorsTable
      categories={data.categories}
      openHours={data.openHours}
      vendors={data.vendors}
      workspaceId={workspaceId}
    />
  )
}

/** `products/layout.tsx`: the `ecommerce` guard. */
export function ProductsLayout() {
  if (!useEcommerceAllowed()) {
    return <NotFoundPage />
  }
  return <Outlet />
}

export function CreateProductPage() {
  return <CreateProduct workspaceId={useWorkspaceId()} />
}

export function EditProductPage() {
  const { workspaceId, id } = useParams({ strict: false }) as {
    workspaceId: string
    id: string
  }
  if (!isNumericId(id)) {
    return <NotFoundPage />
  }
  return <EditProductForm id={id} workspaceId={workspaceId} />
}

function EditProductForm({
  workspaceId,
  id,
}: {
  workspaceId: string
  id: string
}) {
  const data = usePageQuery<{
    product: never
    metaCatalogId: string | null
    publicUrl: string
  }>("productEditPage", { id })

  return (
    <LockedEditorGuard
      resourceId={id}
      resourceKind="product"
      workspaceId={workspaceId}
    >
      <EditProduct
        metaCatalogId={data.metaCatalogId}
        product={data.product}
        publicUrl={data.publicUrl}
        workspaceId={workspaceId}
      />
    </LockedEditorGuard>
  )
}

type Store = {
  currency: string
  currencySymbol: string | null
  currencyPosition: never
}

/** `orders/page.tsx` and `orders/[orderId]/page.tsx`. */
export function OrdersPage() {
  const { orderId } = useParams({ strict: false }) as { orderId?: string }
  if (orderId === undefined) {
    return <OrdersList />
  }
  if (!isNumericId(orderId)) {
    return <NotFoundPage />
  }
  return <OrderDetailView orderId={orderId} />
}

function OrdersList() {
  const workspaceId = useWorkspaceId()
  const search = useSearch(listOrdersSearchParams)
  const data = usePageQuery<{
    orders: never
    store: Store
    externalStore: never
  }>("ordersPage", search)
  return (
    <OrdersTable
      externalStore={data.externalStore ?? undefined}
      orders={data.orders}
      store={data.store}
      workspaceId={workspaceId}
    />
  )
}

function OrderDetailView({ orderId }: { orderId: string }) {
  const workspaceId = useWorkspaceId()
  const data = usePageQuery<{ order: never; store: Store }>(
    "orderDetailPage",
    { orderId },
  )
  return (
    <OrderDetail
      order={data.order}
      store={data.store}
      workspaceId={workspaceId}
    />
  )
}

/** `discount-codes/page.tsx`, `new/page.tsx` and `[discountCodeId]/page.tsx`. */
export function DiscountCodesPage() {
  const { discountCodeId } = useParams({ strict: false }) as {
    discountCodeId?: string
  }
  if (discountCodeId === undefined) {
    return <DiscountCodesList />
  }
  if (discountCodeId === "new") {
    return <DiscountCodeFormView />
  }
  if (!/^\d{1,20}$/.test(discountCodeId)) {
    return <NotFoundPage />
  }
  return <DiscountCodeFormView id={discountCodeId} />
}

function DiscountCodesList() {
  const workspaceId = useWorkspaceId()
  const search = useSearch(listDiscountCodesSearchParams)
  const data = usePageQuery<{ discountCodes: never; store: Store }>(
    "discountCodesPage",
    search,
  )
  return (
    <DiscountCodesTable
      discountCodes={data.discountCodes}
      store={data.store}
      workspaceId={workspaceId}
    />
  )
}

function DiscountCodeFormView({ id }: { id?: string }) {
  const workspaceId = useWorkspaceId()
  const data = usePageQuery<{
    categories: never
    products: never
    currency: string
    discountCode: never
  }>("discountCodeFormPage", { id })
  return (
    <DiscountCodeForm
      categories={data.categories}
      currency={data.currency}
      discountCode={data.discountCode ?? undefined}
      key={id ?? "new"}
      products={data.products}
      workspaceId={workspaceId}
    />
  )
}

/** `ecommerce-settings/page.tsx` (to `general`) and `[section]/page.tsx`. */
export function EcommerceSettingsPage() {
  const { workspaceId, section: rawSection } = useParams({
    strict: false,
  }) as { workspaceId: string; section?: string }
  if (rawSection === undefined) {
    return (
      <Navigate replace to={`/space/${workspaceId}/ecommerce-settings/general`} />
    )
  }
  const { data: section } = ecommerceSettingsSections.safeParse(rawSection)
  if (!section) {
    return <NotFoundPage />
  }
  return <EcommerceSettingsSection section={section} workspaceId={workspaceId} />
}

function EcommerceSettingsSection({
  section,
  workspaceId,
}: {
  section: EcommerceSettingsSection
  workspaceId: string
}) {
  const data = usePageQuery<{
    settings: EcommerceSettingsPublicResource
    flowOptions: never
    openHours: never
    connectedProviders: never
    shopifyStatus: never
    wooCommerceStatus: never
  }>("ecommerceSettingsPage", {})
  const { settings } = data
  const sellsFromConnectedStore =
    (settings.general.storeSource === "shopify" &&
      (data.shopifyStatus as { connected: boolean }).connected) ||
    (settings.general.storeSource === "woocommerce" &&
      (data.wooCommerceStatus as { connected: boolean }).connected)
  // A connected external store owns payment and delivery.
  const hiddenSections = sellsFromConnectedStore
    ? externalStoreHiddenSections
    : []
  if (hiddenSections.includes(section)) {
    return (
      <Navigate replace to={`/space/${workspaceId}/ecommerce-settings/general`} />
    )
  }
  return (
    <div className="flex min-h-0 flex-1">
      <EcommerceSettingsSidebar
        active={section}
        hiddenSections={hiddenSections}
        workspaceId={workspaceId}
      />
      <div className="min-w-0 max-w-5xl flex-1 space-y-4 p-4">
        {section === "general" ? (
          <GeneralSettingsForm
            flowOptions={data.flowOptions}
            openHours={data.openHours}
            settings={settings}
            workspaceId={workspaceId}
          />
        ) : null}
        {section === "payments" ? (
          <PaymentSettingsForm
            connectedProviders={data.connectedProviders}
            settings={settings}
            workspaceId={workspaceId}
          />
        ) : null}
        {section === "shipping" ? (
          <ShippingSettingsForm settings={settings} workspaceId={workspaceId} />
        ) : null}
        {section === "checkout" ? (
          <CheckoutSettingsForm
            flowOptions={data.flowOptions}
            settings={settings}
            workspaceId={workspaceId}
          />
        ) : null}
        {section === "shopify" ? (
          <ShopifySettingsForm
            status={data.shopifyStatus}
            workspaceId={workspaceId}
          />
        ) : null}
        {section === "woocommerce" ? (
          <WooCommerceSettingsForm
            status={data.wooCommerceStatus}
            workspaceId={workspaceId}
          />
        ) : null}
      </div>
    </div>
  )
}

/** `ecommerce-analytics/page.tsx`. */
export function EcommerceAnalyticsPage() {
  const searchParams = useSearchParams()
  const day = (key: string) => {
    const value = searchParams.get(key)
    return value && /^\d{4}-\d{2}-\d{2}$/.test(value) ? value : undefined
  }
  const data = usePageQuery<{
    data: never
    store: Store
    referenceNow: string
    workspaceCreatedAt: string
  }>("ecommerceAnalyticsPage", { from: day("from"), to: day("to") })
  return (
    <AnalyticsDashboard
      data={data.data}
      referenceNow={data.referenceNow}
      store={data.store}
      workspaceCreatedAt={new Date(data.workspaceCreatedAt)}
    />
  )
}
