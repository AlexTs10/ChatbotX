import { defineConfig } from "vitest/config"
import { PathOrderSequencer } from "./src/sequencer.ts"

export default defineConfig({
  test: {
    // Vite sets process.env.BASE_URL to its own base ("/") inside test
    // workers, so hand the stack origin over under another name.
    env: { PARITY_BASE_URL: process.env.PARITY_BASE_URL ?? process.env.BASE_URL ?? "http://app" },
    include: ["tests/**/*.test.ts"],
    globalSetup: ["./src/global-setup.ts"],
    // One file at a time for now; tests use unique data, so this can be raised later.
    fileParallelism: false,
    testTimeout: 30_000,
    hookTimeout: 60_000,
    retry: 0,
    sequence: { sequencer: PathOrderSequencer },
    reporters: ["default", "junit", "json"],
    outputFile: {
      junit: ".state/reports/junit.xml",
      json: ".state/reports/results.json",
    },
  },
})
