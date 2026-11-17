#!/usr/bin/env node
// biome-ignore-all lint/performance/useTopLevelRegex: a one-shot dev script
// Copies builder files into the SPA with their `@/` and relative imports
// (transitive closure). Files already in apps/web are kept (they may be
// edited) but their imports are still followed.
//
// "use server" files become client stubs: each export calls the Rust server
// at `/rpc/actions/<name>` (see src/lib/server-action.ts).
//
// Usage (repo root): node apps/web/scripts/copy-from-builder.mjs src/features/contacts/contacts-table.tsx ...
// Paths are relative to apps/builder. Prints the copied files and the files
// that still import server-only modules (they need a hand edit).
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { dirname, join, relative, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const webRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const builderRoot = resolve(webRoot, "../builder")
const EXTENSIONS = [".ts", ".tsx", "/index.ts", "/index.tsx", ""]

// Modules that cannot run in the browser. A copied file that imports one at
// run time needs a hand edit.
const SERVER_ONLY =
  /^(server-only|next\/headers|next\/server|next\/cache|next-intl\/server|node:.*|@chatbotx\.io\/(business|database|database\/(client|repositories.*)|redis.*|encryption.*|ai\/server|auth\/server|auth\/tenant|logger.*|mail.*|filesystem.*|event-bus.*|worker-config.*))$/

const IMPORT_RE =
  /(?:import|export)\s+(type\s+)?(?:[^'"]*?\s+from\s+)?["']([^"']+)["']|import\(\s*["']([^"']+)["']\s*\)/g

/** Action names that exist in more than one "use server" file. */
const DUPLICATE_ACTION_NAMES = new Set(["updateHandoverResumeFlowAction"])

const WORKSPACE_CLIENTS =
  /\b(workspaceActionClient\w*|calling\w*ActionClient|callHistoryActionClient)\b/

function resolveImport(fromFile, specifier) {
  let base
  if (specifier.startsWith("@/")) {
    base = join("src", specifier.slice(2))
  } else if (specifier.startsWith(".")) {
    base = join(dirname(fromFile), specifier)
  } else {
    return null
  }
  for (const ext of EXTENSIONS) {
    const candidate = base + ext
    for (const root of [webRoot, builderRoot]) {
      const full = join(root, candidate)
      if (existsSync(full) && !full.endsWith("/")) {
        try {
          readFileSync(full)
          return candidate
        } catch {
          // a directory: try the next extension
        }
      }
    }
  }
  return null
}

function actionName(file, name) {
  if (!DUPLICATE_ACTION_NAMES.has(name)) {
    return name
  }
  const feature = file.split("/")[2]
  return `${feature}.${name}`
}

/** The client stub of a "use server" file. */
function stubFor(file, source) {
  const lines = [
    `// Generated from apps/builder/${file} by apps/web/scripts/copy-from-builder.mjs.`,
    "// The server code runs in the Rust server: POST /rpc/actions/<name>.",
    "",
  ]
  const exportRe =
    /^export\s+(?:(type|interface)\s+(\w+)|(?:async\s+)?function\s+(\w+)|const\s+(\w+))/gm
  const matches = [...source.matchAll(exportRe)]
  matches.forEach((match, index) => {
    const end = matches[index + 1]?.index ?? source.length
    const body = source.slice(match.index, end)
    if (match[1]) {
      lines.push(
        "// biome-ignore lint/suspicious/noExplicitAny: server type, not ported",
        `export type ${match[2]} = any`,
      )
      return
    }
    const name = match[3] ?? match[4]
    const rpcName = actionName(file, name)
    if (/ActionClient\b|\.action\(/.test(body)) {
      const flags = [
        WORKSPACE_CLIENTS.test(body) && "workspace: true",
        !/\.inputSchema\(/.test(body) && "noInput: true",
      ].filter(Boolean)
      const options = flags.length > 0 ? `, { ${flags.join(", ")} }` : ""
      lines.push(`export const ${name} = serverAction("${rpcName}"${options})`)
    } else {
      lines.push(`export const ${name} = serverFunction("${rpcName}")`)
    }
  })
  const used = ["serverAction", "serverFunction"].filter((fn) =>
    lines.some((line) => line.includes(`= ${fn}(`)),
  )
  if (used.length > 0) {
    lines.splice(
      2,
      0,
      `import { ${used.join(", ")} } from "@/lib/server-action"`,
    )
  }
  return `${lines.join("\n")}\n`
}

const copied = []
const stubs = []
const serverOnly = new Map()
const seen = new Set()
const queue = process.argv.slice(2)

while (queue.length > 0) {
  const file = queue.shift()
  if (seen.has(file)) {
    continue
  }
  seen.add(file)
  const dest = join(webRoot, file)
  const fromWeb = existsSync(dest)
  const source = readFileSync(fromWeb ? dest : join(builderRoot, file), "utf8")
  const isServerFile = /^\s*["']use server["']/.test(source)

  if (!fromWeb) {
    mkdirSync(dirname(dest), { recursive: true })
    if (isServerFile) {
      writeFileSync(dest, stubFor(file, source))
      stubs.push(file)
      continue
    }
    writeFileSync(dest, source)
    copied.push(file)
  }
  if (isServerFile || !/\.(ts|tsx)$/.test(file)) {
    continue
  }
  const text = fromWeb ? source : readFileSync(dest, "utf8")
  for (const match of text.matchAll(IMPORT_RE)) {
    const specifier = match[2] ?? match[3]
    const typeOnly = Boolean(match[1])
    if (!typeOnly && SERVER_ONLY.test(specifier)) {
      const list = serverOnly.get(file) ?? new Set()
      list.add(specifier)
      serverOnly.set(file, list)
    }
    const target = resolveImport(file, specifier)
    if (target && !seen.has(target)) {
      queue.push(target)
    }
  }
}

console.log(`copied ${copied.length}, stubs ${stubs.length}`)
for (const file of [...copied, ...stubs]) {
  console.log(`  ${file}`)
}
if (serverOnly.size > 0) {
  console.log("server-only imports (hand edit needed):")
  for (const [file, specifiers] of serverOnly) {
    console.log(`  ${relative(".", file)}: ${[...specifiers].join(", ")}`)
  }
}
