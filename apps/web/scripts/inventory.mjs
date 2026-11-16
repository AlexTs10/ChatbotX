#!/usr/bin/env node
// biome-ignore-all lint/performance/useTopLevelRegex: a one-shot dev script
// Writes the P4 inventories (conversion/specs/P4-*.tsv) from apps/builder,
// apps/web and crates/api:
//   P4-routes.tsv      Next.js page -> SPA path -> ported in apps/web?
//   P4-actions.tsv     "use server" export -> /rpc/actions/<name> -> in Rust?
//   P4-procedures.tsv  oRPC procedure the UI calls -> in Rust?
// Usage (repo root): node apps/web/scripts/inventory.mjs
import { execFileSync } from "node:child_process"
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs"
import { dirname, join, relative, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../../..")
const builderSrc = join(repo, "apps/builder/src")
const specs = join(repo, "conversion/specs")

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    const full = join(dir, name)
    if (name === "node_modules" || name === "__tests__") {
      continue
    }
    if (statSync(full).isDirectory()) {
      walk(full, out)
    } else if (/\.(ts|tsx)$/.test(name) && !/\.test\.tsx?$/.test(name)) {
      out.push(full)
    }
  }
  return out
}

const builderFiles = walk(builderSrc)
const webFiles = walk(join(repo, "apps/web/src"))
const webText = webFiles.map((file) => readFileSync(file, "utf8")).join("\n")
// The generated `rpc/table.rs` lists every procedure, ported or not: skip it.
const rustText = execFileSync(
  "grep",
  [
    "-rhoE",
    "--exclude=table.rs",
    '"[A-Za-z0-9_.-]+"',
    join(repo, "crates/api/src"),
  ],
  { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
)
const rustStrings = new Set(rustText.split("\n").map((s) => s.slice(1, -1)))

// Routes: `// page: app/...` markers in apps/web/src name the ported pages.
const ported = new Set(
  [...webText.matchAll(/\/\/ page: (app\/\S+)/g)].map((match) => match[1]),
)
const routeRows = builderFiles
  .filter((file) => file.endsWith("/page.tsx"))
  .map((file) => relative(builderSrc, file))
  .sort()
  .map((page) => {
    const spa =
      page
        .replace(/^app/, "")
        .replace(/\/page\.tsx$/, "")
        .replace(/\/\([^)]*\)/g, "")
        .replace(/\[\[?\.\.\.\w+\]\]?/g, "$")
        .replace(/\[(\w+)\]/g, "$$$1") || "/"
    return [page, spa, ported.has(page) ? "ported" : "todo"]
  })

// Actions: every export of a "use server" file.
const actionRows = []
for (const file of builderFiles) {
  const source = readFileSync(file, "utf8")
  if (!/^\s*["']use server["']/.test(source)) {
    continue
  }
  const rel = relative(builderSrc, file)
  const matches = [
    ...source.matchAll(
      /^export\s+(?:(?:async\s+)?function\s+(\w+)|const\s+(\w+))/gm,
    ),
  ]
  matches.forEach((match, index) => {
    const name = match[1] ?? match[2]
    const body = source.slice(match.index, matches[index + 1]?.index)
    let kind = "function"
    if (/\.action\(/.test(body)) {
      kind =
        /workspaceActionClient|calling\w*ActionClient|callHistoryActionClient/.test(
          body,
        )
          ? "workspace-action"
          : "auth-action"
    }
    const rpcName =
      name === "updateHandoverResumeFlowAction"
        ? `${rel.split("/")[1]}.${name}`
        : name
    actionRows.push([
      rel.split("/").slice(0, 2).join("/"),
      rel,
      name,
      kind,
      `/rpc/actions/${rpcName}`,
      rustStrings.has(rpcName) ? "rust" : "missing",
    ])
  })
}
actionRows.sort((a, b) => a[0].localeCompare(b[0]) || a[1].localeCompare(b[1]))

// oRPC procedures called from UI code (orpc.X.Y / client.X.Y).
const uses = new Map()
for (const file of builderFiles) {
  const source = readFileSync(file, "utf8")
  if (/^\s*["']use server["']/.test(source) || file.includes("/api/")) {
    continue
  }
  for (const match of source.matchAll(
    /\b(?:orpc|client)\.(\w+(?:API|APIs|Routes))\.(\w+)/g,
  )) {
    const key = `${match[1]}.${match[2]}`
    uses.set(key, (uses.get(key) ?? 0) + 1)
  }
}
const procedureRows = [...uses]
  .sort(([a], [b]) => a.localeCompare(b))
  .map(([key, count]) => {
    const inWeb = webText.includes(key) ? "web" : ""
    return [
      key,
      String(count),
      inWeb,
      rustStrings.has(key) ? "rust" : "missing",
    ]
  })

const tsv = (header, rows) =>
  `${[header, ...rows].map((row) => row.join("\t")).join("\n")}\n`
writeFileSync(
  join(specs, "P4-routes.tsv"),
  tsv(["page", "spa_path", "status"], routeRows),
)
writeFileSync(
  join(specs, "P4-actions.tsv"),
  tsv(["feature", "file", "export", "kind", "rpc_path", "rust"], actionRows),
)
writeFileSync(
  join(specs, "P4-procedures.tsv"),
  tsv(["procedure", "ui_uses", "used_in_web", "rust"], procedureRows),
)
console.log(
  `routes ${routeRows.length} (ported ${routeRows.filter((r) => r[2] === "ported").length}), ` +
    `actions ${actionRows.length} (rust ${actionRows.filter((r) => r[5] === "rust").length}), ` +
    `procedures ${procedureRows.length} (rust ${procedureRows.filter((r) => r[3] === "rust").length})`,
)
