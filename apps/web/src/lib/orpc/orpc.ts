// The oRPC client of apps/builder/src/lib/orpc/orpc.ts, pointed at the Rust
// `/rpc`. One request per call (no BatchLinkPlugin: the Rust server has no
// batch endpoint). Typed loosely: the builder's router type would pull the
// whole server graph into the SPA type check.
import { createORPCClient } from "@orpc/client"
import { RPCLink } from "@orpc/client/fetch"

export const RPC_ENDPOINT_PATH = "/rpc"

// biome-ignore lint/suspicious/noExplicitAny: see the note above
export type RouterClientAny = any

const link = new RPCLink({
  url: `${window.location.origin}${RPC_ENDPOINT_PATH}`,
})

export const client: RouterClientAny = createORPCClient(link)
