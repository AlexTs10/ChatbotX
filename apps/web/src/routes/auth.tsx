// page: app/auth/layout.tsx
// page: app/auth/sign-in/page.tsx
// page: app/auth/sign-up/page.tsx
// page: app/auth/forgot-password/page.tsx
// page: app/auth/reset-password/page.tsx
// page: app/auth/magic-link-sent/page.tsx
// page: app/auth/change-password/page.tsx
import { useSuspenseQuery } from "@tanstack/react-query"
import { Outlet } from "@tanstack/react-router"
import { ThemeSwitcher } from "@/components/theme-switcher"
import { SignInForm } from "@/features/auth/sign-in"
import { SignUpForm } from "@/features/auth/sign-up"
import { resolveAuthPageOptions } from "@/lib/auth/auth-page-options"

export function AuthLayout() {
  return (
    <div className="flex min-h-svh flex-col items-center justify-center gap-6 bg-muted p-6 md:p-10">
      <div className="flex w-full max-w-sm flex-col gap-6">
        <Outlet />
      </div>

      <div className="fixed inset-e-2 bottom-2">
        <ThemeSwitcher />
      </div>
    </div>
  )
}

const authOptionsQuery = {
  queryKey: ["spa", "authOptions"],
  queryFn: resolveAuthPageOptions,
}

export function SignInPage() {
  const { data } = useSuspenseQuery(authOptionsQuery)
  return <SignInForm {...data} />
}

export function SignUpPage() {
  const { data } = useSuspenseQuery(authOptionsQuery)
  return <SignUpForm {...data} />
}

export { ChangePassword as ChangePasswordPage } from "@/features/auth/change-password"
export { ForgotPassword as ForgotPasswordPage } from "@/features/auth/forgot-password"
export { default as MagicLinkSentPage } from "@/features/auth/magic-link-sent"
export { ResetPassword as ResetPasswordPage } from "@/features/auth/reset-password"
