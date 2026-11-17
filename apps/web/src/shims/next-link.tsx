// `next/link`: an anchor with client-side navigation.
import { useRouter } from "@tanstack/react-router"
import type { AnchorHTMLAttributes, MouseEvent, Ref } from "react"

type LinkProps = Omit<AnchorHTMLAttributes<HTMLAnchorElement>, "href"> & {
  href: string | { pathname?: string; query?: Record<string, string> }
  replace?: boolean
  prefetch?: boolean | null
  scroll?: boolean
  ref?: Ref<HTMLAnchorElement>
}

/** A URL with a scheme (`https:`, `mailto:`, ...): the browser opens it. */
const SCHEME_RE = /^[a-z][a-z0-9+.-]*:/i

const toHref = (href: LinkProps["href"]) =>
  typeof href === "string"
    ? href
    : `${href.pathname ?? ""}${href.query ? `?${new URLSearchParams(href.query)}` : ""}`

export default function Link({
  href,
  replace,
  prefetch: _prefetch,
  scroll: _scroll,
  onClick,
  target,
  ...props
}: LinkProps) {
  const router = useRouter()
  const url = toHref(href)
  const handleClick = (event: MouseEvent<HTMLAnchorElement>) => {
    onClick?.(event)
    const external = SCHEME_RE.test(url)
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey ||
      (target && target !== "_self") ||
      external
    ) {
      return
    }
    event.preventDefault()
    if (replace) {
      router.history.replace(url)
    } else {
      router.history.push(url)
    }
  }
  return <a href={url} onClick={handleClick} target={target} {...props} />
}
