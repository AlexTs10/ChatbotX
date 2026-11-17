// `next/dynamic`: React.lazy + Suspense.
import type React from "react"
import { type ComponentType, lazy, type ReactNode, Suspense } from "react"

type Loaded<P> = ComponentType<P> | { default: ComponentType<P> }

export default function dynamic<P extends object>(
  load: () => Promise<Loaded<P>>,
  options?: { loading?: () => ReactNode; ssr?: boolean },
) {
  const Lazy = lazy(async () => {
    const loaded = await load()
    return { default: "default" in loaded ? loaded.default : loaded }
  })
  return function DynamicComponent(props: P) {
    return (
      <Suspense fallback={options?.loading?.() ?? null}>
        <Lazy {...(props as P & React.JSX.IntrinsicAttributes)} />
      </Suspense>
    )
  }
}
