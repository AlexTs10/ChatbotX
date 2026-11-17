// The builder pages passed a server promise to client components that
// `use()` it. Here the data comes from a query (so `router.refresh()`
// refetches it) and the component gets a promise that is already settled.
import { type QueryKey, useSuspenseQuery } from "@tanstack/react-query"
import { useMemo } from "react"

export function useQueryPromise<T>(
  queryKey: QueryKey,
  queryFn: () => Promise<T>,
): Promise<T> {
  const { data } = useSuspenseQuery({ queryKey: ["spa", ...queryKey], queryFn })
  // `use()` reads `status` / `value` of a settled thenable without suspending.
  return useMemo(
    () =>
      Object.assign(Promise.resolve(data), {
        status: "fulfilled",
        value: data,
      }),
    [data],
  )
}
