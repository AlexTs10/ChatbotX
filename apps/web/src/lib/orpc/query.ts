import { createTanstackQueryUtils } from "@orpc/tanstack-query"
import type {
  QueryKey,
  UseInfiniteQueryOptions,
  UseMutationOptions,
  UseQueryOptions,
} from "@tanstack/react-query"
import { client } from "./orpc"

// The TanStack Query utils of every procedure, typed loosely like `client`
// (see orpc.ts) but so that `useQuery(orpc.x.y.queryOptions(...))` gives
// `any` data instead of `unknown`.
/* biome-ignore-start lint/suspicious/noExplicitAny: the procedure types live on the server */
type ProcedureUtilsAny = {
  queryOptions: (
    ...args: any[]
  ) => UseQueryOptions<any, any, any, any> & { queryKey: any }
  infiniteOptions: (...args: any[]) => UseInfiniteQueryOptions<
    any,
    any,
    any,
    any,
    any
  > & {
    queryKey: any
  }
  mutationOptions: (...args: any[]) => UseMutationOptions<any, any, any, any>
  key: (...args: any[]) => QueryKey
  queryKey: (...args: any[]) => QueryKey
  call: (...args: any[]) => Promise<any>
}
export type OrpcUtilsAny = ProcedureUtilsAny & { [key: string]: OrpcUtilsAny }
/* biome-ignore-end lint/suspicious/noExplicitAny: see above */

// Kept separate from `orpc.ts` so importers of `client` don't pull in react-query.
export const orpc = createTanstackQueryUtils(client) as unknown as OrpcUtilsAny
