// Unique test data. Every test makes its own users, workspaces and upstream
// host names, so tests never depend on each other or on order.

import { randomBytes } from "node:crypto"

/** Time-ordered, random suffix: lowercase letters and digits only. */
export const uniqueId = (): string =>
  `${Date.now().toString(36)}${randomBytes(4).toString("hex")}`

export const uniqueEmail = (label = "u"): string =>
  `${label}-${uniqueId()}@parity.test`

/** A host under `*.parity.test`, which the fake upstream certificate covers. */
export const uniqueHost = (label = "h"): string => `${label}-${uniqueId()}.parity.test`

/**
 * A client address for `X-Forwarded-For`. Auth endpoints allow 3 calls per
 * 10 s per client address, so each simulated client uses its own address.
 */
export const uniqueClientIp = (): string => {
  const bytes = randomBytes(3)
  return `10.${bytes[0]}.${bytes[1]}.${(bytes[2] ?? 0) % 254 + 1}`
}
