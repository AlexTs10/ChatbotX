// A server component's `redirect()` during render: replace the URL after mount.
import { useRouter } from "@tanstack/react-router"
import { useEffect } from "react"

export function Redirect({ href }: { href: string }) {
  const router = useRouter()
  useEffect(() => {
    router.history.replace(href)
  }, [router, href])
  return null
}
