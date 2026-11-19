// page: app/space/[workspaceId]/(coupons)/layout.tsx
// page: app/space/[workspaceId]/(coupons)/coupons/page.tsx
// page: app/space/[workspaceId]/(coupons)/topic-coupons/page.tsx
// Membership is the workspace layout's check (no permission guard in the
// builder); data comes from `couponsAPI.*` (`rpc_procedures/coupons.rs`).
import { Outlet, useParams } from "@tanstack/react-router"
import { useTranslations } from "next-intl"
import { Suspense } from "react"
import { AppBreadcrumb } from "@/components/app-breadcrumb"
import { CouponList } from "@/features/coupons/components/coupon-list"
import { CouponTabs } from "@/features/coupons/components/coupon-tabs"
import { TopicCouponContent } from "@/features/coupons/components/topic-coupon-content"

function useWorkspaceId() {
  return (useParams({ strict: false }) as { workspaceId: string }).workspaceId
}

export function CouponsLayout() {
  return (
    <div className="flex flex-col gap-4">
      <CouponTabs />
      <Suspense>
        <Outlet />
      </Suspense>
    </div>
  )
}

export function CouponsPage() {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb
        items={[
          { label: t("tools.title"), href: `/space/${workspaceId}/tools` },
          { label: t("coupons.title"), href: "" },
        ]}
      />
      <CouponList workspaceId={workspaceId} />
    </div>
  )
}

export function TopicCouponsPage() {
  const t = useTranslations()
  const workspaceId = useWorkspaceId()
  return (
    <div className="flex flex-col gap-4">
      <AppBreadcrumb
        items={[
          { label: t("tools.title"), href: `/space/${workspaceId}/tools` },
          { label: t("coupons.topicCouponsTitle"), href: "" },
        ]}
      />
      <TopicCouponContent workspaceId={workspaceId} />
    </div>
  )
}
