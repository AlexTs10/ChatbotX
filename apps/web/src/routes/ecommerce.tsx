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
// crates/api/src/rpc_procedures/products.rs).
import { isNumericId } from "@chatbotx.io/utils"
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet, useParams } from "@tanstack/react-router"
import { useSearchParams } from "next/navigation"
import { Suspense } from "react"
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
import { NotAvailablePage, NotFoundPage } from "@/routes/root"
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

/** The e-commerce tabs whose domain is not in the Rust server yet (Q.D8). */
function ecommerceNotAvailable(title: string) {
  return function EcommerceNotAvailablePage() {
    return (
      <NotAvailablePage
        detail={`${title} is not available yet in this version.`}
        title={title}
      />
    )
  }
}
export const OrdersPage = ecommerceNotAvailable("Orders")
export const DiscountCodesPage = ecommerceNotAvailable("Discount codes")
export const EcommerceSettingsPage = ecommerceNotAvailable(
  "E-commerce settings",
)
export const EcommerceAnalyticsPage = ecommerceNotAvailable(
  "E-commerce analytics",
)
