// Per-unit fixture extensions, so parallel Phase 1 units never edit the same file.
//
// A unit that needs setup with no public HTTP path (for example a channel row
// or platform credentials) adds:
//   src/fixtures/ext/<unit>.ts            — the interface (stack-neutral types only)
//   stacks/<stack>/ext/<unit>.ts          — `export const create = (): <Interface>` for that stack
// and loads it in tests with `await loadFixtureExt<MyExt>("<unit>")`.
// The core driver in `driver.ts` / `stacks/<stack>/fixtures.ts` stays frozen after P1.1.

import path from "node:path"
import { fileURLToPath } from "node:url"
import { config } from "../config.ts"

const here = path.dirname(fileURLToPath(import.meta.url))
const cache = new Map<string, Promise<unknown>>()

export const loadFixtureExt = <T>(unit: string): Promise<T> => {
  if (!/^[a-z0-9-]+$/.test(unit)) throw new Error(`bad fixture ext name: ${unit}`)
  let p = cache.get(unit)
  if (!p) {
    const modulePath = path.resolve(here, "../../stacks", config.stack, "ext", `${unit}.ts`)
    p = (import(modulePath) as Promise<{ create: () => T | Promise<T> }>).then((m) => m.create())
    cache.set(unit, p)
  }
  return p as Promise<T>
}
