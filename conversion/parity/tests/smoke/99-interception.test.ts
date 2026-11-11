// Runs last (PathOrderSequencer). Any third-party call that no script or
// default answered, and any TLS name outside the fake certificate, means the
// stack reached for an upstream that the tests did not expect.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"

describe("smoke: interception self-check", () => {
  it("saw no unmatched upstream request and no unknown TLS name", async () => {
    const unmatched = await fake.unmatched()
    expect(
      unmatched.map((record) => ({
        kind: record.kind,
        host: record.host,
        method: record.method,
        path: record.path,
        unknownSni: record.unknownSni,
      })),
    ).toEqual([])
  })
})
