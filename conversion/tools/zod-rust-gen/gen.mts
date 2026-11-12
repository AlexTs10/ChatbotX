// One-off generator (P3.4): walks the zod 4 definitions exported by
// packages/flow-config and writes Rust skeletons that rebuild the same schemas
// with `chatbotx_core::flow_config::z`. Closures (refine, superRefine,
// transform, custom checks) cannot be translated: they become named stubs with
// the TypeScript source in a comment, ported by hand afterwards.
//
// Run (Mac, light):
//   node --import ./register.mjs ../../../node_modules/.bin/tsx gen.ts <outDir>
// or: tsx --import ./register.mjs gen.ts <outDir>
import { readFileSync, readdirSync, statSync, mkdirSync, writeFileSync } from "node:fs"
import { join, relative, dirname } from "node:path"
const { globalRegistry } = await import("/Users/alextoska/Desktop/ChatbotX-lean/packages/flow-config/node_modules/zod/index.js")

const ROOT = "/Users/alextoska/Desktop/ChatbotX-lean/packages/flow-config/src"
const OUT = process.argv[2] ?? "/tmp/fc-gen"

const walkDir = (dir: string): string[] =>
  readdirSync(dir).flatMap((f) => {
    const p = join(dir, f)
    return statSync(p).isDirectory() ? walkDir(p) : p.endsWith(".ts") ? [p] : []
  })

const files = walkDir(ROOT).filter((f) => !f.endsWith("/index.ts") || f.endsWith("/states/index.ts") || f.endsWith("/nodes/index.ts"))

const snake = (s: string) =>
  s
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .replace(/[^A-Za-z0-9_]/g, "_")
    .toLowerCase()
const upper = (s: string) => (/^[A-Z0-9_]+$/.test(s) ? s : snake(s).toUpperCase())
const RUST_KW = new Set(["type", "enum", "match", "fn", "mod", "use", "ref", "move", "loop", "where", "impl", "trait", "struct", "self", "super", "crate", "box", "async", "await", "dyn", "static", "const", "let", "in", "for", "if", "else", "as", "break", "continue", "return", "true", "false", "mut", "pub", "unsafe", "extern", "while", "yield", "try", "macro", "override", "final", "priv", "virtual", "abstract", "become", "do", "typeof", "unsized"])
const ident = (s: string) => (RUST_KW.has(s) ? `r#${s}` : s)

type ExportInfo = { file: string; name: string; rust: string }
const exportBySchema = new Map<any, ExportInfo>()
const exportByDef = new Map<any, ExportInfo>()
const exportByFn = new Map<any, ExportInfo>()
const modules: { file: string; mod: any; order: string[] }[] = []

const exportOrder = (src: string) => {
  const names: string[] = []
  const re = /export\s+(?:const|function|class|let)\s+([A-Za-z0-9_$]+)/g
  let m
  while ((m = re.exec(src))) names.push(m[1])
  return names
}

for (const file of files) {
  const mod = await import(file)
  const src = readFileSync(file, "utf8")
  const order = exportOrder(src).filter((n) => n in mod)
  modules.push({ file, mod, order })
  for (const name of order) {
    const v = mod[name]
    const info = { file, name, rust: snake(name) }
    if (v && v._zod) {
      if (!exportBySchema.has(v)) exportBySchema.set(v, info)
      if (!exportByDef.has(v._zod.def)) exportByDef.set(v._zod.def, info)
    } else if (typeof v === "function") {
      if (!exportByFn.has(v)) exportByFn.set(v, info)
    }
  }
}

// utils schemas referenced from flow-config
const U = "/Users/alextoska/Desktop/ChatbotX-lean/packages/utils/src/"
const utilsMods: Record<string, string> = Object.fromEntries(
  ["index", "meta-capi", "google-click", "custom-field", "channel", "error-log", "custom-field-operation", "variables", "datetime", "id"].map((m) => [`${U}${m}.ts`, "deps"]),
)
const depSchemas = new Map<any, string>()
for (const m of Object.keys(utilsMods)) {
  const mod = await import(m)
  for (const [name, v] of Object.entries(mod)) {
    if (v && (v as any)._zod && !depSchemas.has(v)) depSchemas.set(v, name)
  }
}
const usedDeps = new Map<string, any>()

// ---------------------------------------------------------------- emitter
let currentFile = ""
let stubs: string[] = []
const fnNames = new Map<any, string>()
let stubCounter = 0
const fileStubNames = new Set<string>()

const jsonLit = (v: unknown): string => `json!(${JSON.stringify(v)})`
const strLit = (s: string) => JSON.stringify(s)
const rawRe = (s: string) => {
  let hashes = "#"
  while (s.includes(`"${hashes}`)) hashes += "#"
  return `r${hashes}"${s}"${hashes}`
}

const errMsg = (def: any): string | undefined => {
  if (!def?.error) return undefined
  try {
    const src = def.error.toString()
    const r = def.error({})
    if (typeof r === "string" && /^\(\)\s*=>/.test(src)) return r
    if (r && typeof r.message === "string" && /^\(\)\s*=>/.test(src)) return r.message
  } catch {}
  return `__ERRFN__${def.error.toString()}`
}

const stubFn = (fn: Function, kind: string, owner: string, sig: string): string => {
  const known = exportByFn.get(fn)
  if (known && known.file !== currentFile) return `${known.rust}`
  if (fnNames.has(fn)) return fnNames.get(fn)!
  let base = fn.name && !["fn", "refinement", "check", "transform", "anonymous", ""].includes(fn.name) ? snake(fn.name) : `${owner}_${kind}`
  let name = base
  let i = 2
  while (fileStubNames.has(name)) name = `${base}_${i++}`
  fileStubNames.add(name)
  fnNames.set(fn, name)
  const src = fn.toString().split("\n").map((l) => `// ${l}`).join("\n")
  stubs.push(`${src}\n${sig.replace("NAME", name)} {\n    todo!("P3.4")\n}\n`)
  return name
}

const SIG_REFINE = "fn NAME(value: &Value) -> bool"
const SIG_SUPER = "fn NAME(value: &Value, ctx: &mut RefineCtx)"
const SIG_TRANSFORM = "fn NAME(value: Value, ctx: &mut RefineCtx) -> Value"
const SIG_PRE = "fn NAME(value: Option<Value>) -> Option<Value>"

const onStack = new Map<any, string>()
const hoisted = new Map<any, string>()
let newHoist = false

function walk(s: any, owner: string, root = false): string {
  if (!root && hoisted.has(s)) return `z::lazy(${hoisted.get(s)})`
  if (!root) {
    const ex = exportBySchema.get(s)
    if (ex) return ex.file === currentFile ? `${ex.rust}()` : `${ex.rust}()`
    const dep = depSchemas.get(s)
    if (dep) {
      usedDeps.set(dep, s)
      return `deps::${snake(dep)}()`
    }
  }
  const def = s._zod.def
  let meta = globalRegistry.get(s) as any
  // describe()/meta() clone: same def as an export
  if (!root && !exportBySchema.has(s)) {
    const ex = exportByDef.get(def)
    if (ex) return `${ex.rust}()${metaSuffix(meta)}`
  }
  if (onStack.has(s)) {
    if (!hoisted.has(s)) {
      hoisted.set(s, `${onStack.get(s)}_rec`)
      newHoist = true
    }
    return `z::lazy(${hoisted.get(s)})`
  }
  onStack.set(s, owner)
  try {
    let out = base(s, def, owner)
    out += checksSuffix(def, owner, s)
    const em = errMsg(def)
    if (em && !["string_format"].includes(def.check)) out += `.error(${strLit(em)})`
    out += metaSuffix(meta)
    return out
  } finally {
    onStack.delete(s)
  }
}

function metaSuffix(meta: any): string {
  if (!meta) return ""
  let out = ""
  const { description, ...rest } = meta
  if (description !== undefined) out += `.describe(${strLit(description)})`
  if (Object.keys(rest).length) out += `.meta(${jsonLit(rest)})`
  return out
}

function formatCall(def: any): string {
  switch (def.format) {
    case "email":
      return `.email()`
    case "url": {
      let o = `.url()`
      if (def.hostname) o = `.url_with(Some(${rawRe(def.hostname.source)}), ${def.protocol ? `Some(${rawRe(def.protocol.source)})` : "None"}, ${!!def.normalize})`
      else if (def.protocol) o = `.url_with(None, Some(${rawRe(def.protocol.source)}), ${!!def.normalize})`
      return o
    }
    case "regex":
      return `.regex(${rawRe(def.pattern.source)}, ${strLit(def.pattern.flags)})`
    case "starts_with":
      return `.starts_with(${strLit(def.prefix)})`
    case "ends_with":
      return `.ends_with(${strLit(def.suffix)})`
    case "includes":
      return `.includes(${strLit(def.includes)})`
    case "lowercase":
      return `.lowercase()`
    case "uppercase":
      return `.uppercase()`
    case "datetime":
      return `.iso_datetime(${def.offset ? "true" : "false"}, ${def.local ? "true" : "false"}, ${def.precision ?? "None"})`
    case "date":
      return `.iso_date()`
    case "time":
      return `.iso_time()`
    case "uuid":
      return `.uuid()`
    default:
      return `.format(${strLit(def.format)}, ${def.pattern ? rawRe(def.pattern.source) : '""'})`
  }
}

function base(s: any, def: any, owner: string): string {
  switch (def.type) {
    case "string": {
      let o = "z::string()"
      if (def.coerce) o += ".coerce()"
      if (def.format) {
        o += formatCall(def)
        const em = errMsg(def)
        if (em) o += `.msg(${strLit(em)})`
      }
      return o
    }
    case "number": {
      let o = "z::number()"
      if (def.coerce) o += ".coerce()"
      if (def.format) o += def.format.includes("int") ? `.int()` : `.number_format(${strLit(def.format)})`
      return o
    }
    case "boolean":
      return def.coerce ? "z::boolean().coerce()" : "z::boolean()"
    case "null":
      return "z::null()"
    case "undefined":
      return "z::undefined()"
    case "void":
      return "z::undefined()"
    case "never":
      return "z::never()"
    case "any":
      return "z::any()"
    case "unknown":
      return "z::unknown()"
    case "date":
      return def.coerce ? "z::date().coerce()" : "z::date()"
    case "nan":
      return "z::never()"
    case "literal":
      return def.values.length === 1 ? `z::literal(${jsonLit(def.values[0])})` : `z::literals(vec![${def.values.map(jsonLit).join(", ")}])`
    case "enum": {
      const vals = Object.values(def.entries)
      if (vals.every((v) => typeof v === "string")) return `z::enum_(&[${vals.map((v) => strLit(v as string)).join(", ")}])`
      return `z::literals(vec![${vals.map(jsonLit).join(", ")}])`
    }
    case "array":
      return `z::array(${walk(def.element, owner)})`
    case "tuple": {
      const items = def.items.map((i: any, n: number) => walk(i, `${owner}_${n}`)).join(",\n")
      return def.rest ? `z::tuple_rest(vec![${items}], ${walk(def.rest, owner)})` : `z::tuple(vec![${items}])`
    }
    case "object": {
      const shape = def.shape
      const fields = Object.entries(shape)
        .map(([k, v]) => `(${strLit(k)}, ${walk(v, `${owner}_${snake(k)}`)})`)
        .join(",\n")
      let o = `z::object(vec![${fields}])`
      if (def.catchall) {
        const t = def.catchall._zod.def.type
        if (t === "never") o += ".strict()"
        else if (t === "unknown") o += ".loose()"
        else o += `.catchall(${walk(def.catchall, owner)})`
      }
      return o
    }
    case "record":
      return `z::record(${walk(def.keyType, owner)}, ${walk(def.valueType, owner)})`
    case "union": {
      const opts = def.options.map((o: any, n: number) => walk(o, `${owner}_${n}`)).join(",\n")
      if (def.discriminator) return `z::discriminated_union(${strLit(def.discriminator)}, vec![${opts}])`
      return `z::union(vec![${opts}])`
    }
    case "intersection":
      return `z::intersection(${walk(def.left, owner)}, ${walk(def.right, owner)})`
    case "optional":
      return `${walk(def.innerType, owner)}.optional()`
    case "nullable":
      return `${walk(def.innerType, owner)}.nullable()`
    case "nonoptional":
      return `${walk(def.innerType, owner)}.nonoptional()`
    case "readonly":
      return walk(def.innerType, owner)
    case "default":
    case "prefault": {
      const m = def.type === "default" ? "default" : "prefault"
      const raw = def.rawDefault
      if (typeof raw === "function") {
        const a = raw()
        const b = raw()
        const same = JSON.stringify(a) === JSON.stringify(b)
        const note = same ? `/* fn: ${raw.toString().replace(/\*\//g, "* /").slice(0, 200)} */ ` : `/* FN-DIFFERS: ${raw.toString().replace(/\*\//g, "* /")} */ `
        return `${walk(def.innerType, owner)}.${m}(${note}${jsonLit(a ?? null)})`
      }
      return `${walk(def.innerType, owner)}.${m}(${jsonLit(raw ?? null)})`
    }
    case "catch": {
      let v
      try {
        v = def.catchValue({ error: { issues: [] }, input: undefined, value: undefined, issues: [] })
      } catch (e) {
        v = "__CATCH_THROWS__"
      }
      return `${walk(def.innerType, owner)}.catch_value(/* ${def.catchValue.toString().replace(/\*\//g, "* /").slice(0, 200)} */ ${jsonLit(v ?? null)})`
    }
    case "pipe": {
      const inn = def.in
      const out = def.out
      if (out._zod.def.type === "transform") {
        const name = stubFn(out._zod.def.transform, "transform", owner, SIG_TRANSFORM)
        return `${walk(inn, owner)}.transform(${name})`
      }
      if (inn._zod.def.type === "transform") {
        const name = stubFn(inn._zod.def.transform, "preprocess", owner, SIG_PRE)
        return `z::preprocess(${name}, ${walk(out, owner)})`
      }
      return `${walk(inn, owner)}.pipe(${walk(out, owner)})`
    }
    case "transform": {
      const name = stubFn(def.transform, "transform", owner, SIG_TRANSFORM)
      return `z::transform(${name})`
    }
    case "lazy": {
      const target = def.getter()
      const ex = exportBySchema.get(target)
      if (ex) return `z::lazy(${ex.rust})`
      if (hoisted.has(target)) return `z::lazy(${hoisted.get(target)})`
      return `z::lazy(|| ${walk(target, owner)})`
    }
    case "custom": {
      const name = stubFn(def.fn, "custom", owner, SIG_REFINE)
      const em = errMsg(def)
      return `z::custom(${name}${em ? `, Some(${strLit(em)})` : ", None"})`
    }
    default:
      return `/* UNSUPPORTED ${def.type} */ todo!()`
  }
}

function checksSuffix(def: any, owner: string, s: any): string {
  const checks: any[] = def.checks ?? []
  let out = ""
  for (const ch of checks) {
    const cd = ch._zod.def
    if (ch === s) continue
    const em = errMsg(cd)
    const msg = em ? `.msg(${strLit(em)})` : ""
    switch (cd.check) {
      case "min_length":
        out += `.min(${cd.minimum})${msg}`
        break
      case "max_length":
        out += `.max(${cd.maximum})${msg}`
        break
      case "length_equals":
        out += `.length(${cd.length})${msg}`
        break
      case "greater_than":
        out += `.${cd.inclusive ? "gte" : "gt"}(${cd.value} as f64)${msg}`
        break
      case "less_than":
        out += `.${cd.inclusive ? "lte" : "lt"}(${cd.value} as f64)${msg}`
        break
      case "multiple_of":
        out += `.multiple_of(${cd.value} as f64)${msg}`
        break
      case "number_format":
        out += cd.format.includes("int") ? `.int()${msg}` : `.number_format(${strLit(cd.format)})${msg}`
        break
      case "string_format":
        out += formatCall(cd) + msg
        break
      case "overwrite": {
        const src = cd.tx.toString()
        if (src.includes(".trim()")) out += ".trim()"
        else if (src.includes(".toLowerCase()")) out += ".to_lower_case()"
        else if (src.includes(".toUpperCase()")) out += ".to_upper_case()"
        else out += `.overwrite(${stubFn(cd.tx, "overwrite", owner, SIG_TRANSFORM)})`
        break
      }
      case "custom": {
        const path = cd.path ? `, Some(&[${cd.path.map((p: any) => strLit(String(p))).join(", ")}])` : ", None"
        const abort = cd.abort ? ".abort()" : ""
        const when = cd.when ? `/* WHEN: ${cd.when.toString()} */` : ""
        if (cd.userFn) {
          out += `.super_refine(${stubFn(cd.userFn, "super_refine", owner, SIG_SUPER)})${abort}${when}`
        } else if (cd.fn) {
          out += `.refine(${stubFn(cd.fn, "refine", owner, SIG_REFINE)}, ${em ? `Some(${strLit(em)})` : "None"}${path})${abort}${when}`
        } else if (ch._zod.check) {
          out += `.check(${stubFn(ch._zod.check, "check", owner, "fn NAME(payload: &mut Payload)")})${when}`
        }
        break
      }
      case "describe":
        break
      default:
        out += `/* UNSUPPORTED CHECK ${cd.check} */`
    }
  }
  return out
}

// ------------------------------------------------------------- data values
const isPlainData = (v: unknown, depth = 0): boolean => {
  if (depth > 12) return false
  if (v === null || ["string", "number", "boolean"].includes(typeof v)) return true
  if (Array.isArray(v)) return v.every((x) => isPlainData(x, depth + 1))
  if (typeof v === "object") return Object.getPrototypeOf(v) === Object.prototype && Object.values(v as object).every((x) => x === undefined || isPlainData(x, depth + 1))
  return false
}

const diffIds = (a: any, b: any): string => {
  // json literal with create_id() where two calls differ on digit strings
  if (typeof a === "string" && typeof b === "string" && a !== b && /^\d{10,}$/.test(a)) return "__ID__"
  if (Array.isArray(a) && Array.isArray(b)) return `[${a.map((x, i) => diffIds(x, b[i])).join(", ")}]`
  if (a && typeof a === "object" && b && typeof b === "object") {
    return `{${Object.entries(a)
      .filter(([, v]) => v !== undefined)
      .map(([k, v]) => `${JSON.stringify(k)}: ${diffIds(v, b[k])}`)
      .join(", ")}}`
  }
  if (JSON.stringify(a) !== JSON.stringify(b)) return `__DIFF__${JSON.stringify(a)}`
  return JSON.stringify(a) ?? "null"
}

// ------------------------------------------------------------------- main
for (const { file, mod, order } of modules) {
 hoisted.clear()
 let parts: string[] = []
 for (let pass = 0; pass < 3; pass++) {
  currentFile = file
  stubs = []
  fnNames.clear()
  fileStubNames.clear()
  stubCounter = 0
  newHoist = false
  parts = []
  for (const name of order) {
    const v = mod[name]
    const rust = snake(name)
    try {
      if (v && v._zod) {
        parts.push(`schema!(pub fn ${ident(rust)} = ${walk(v, rust, true)});`)
      } else if (typeof v === "function") {
        const src = v.toString()
        if (v.length === 0 && /DefaultFn$|DefaultButton$|^createEmpty/.test(name) && !/^(?:function\s*\w*)?\(\s*\w/.test(src)) {
          let a, b
          try {
            a = v()
            b = v()
          } catch {}
          if (a !== undefined && isPlainData(JSON.parse(JSON.stringify(a)))) {
            const lit = diffIds(a, b).replaceAll('"__ID__"', "create_id()")
            parts.push(`default_fn!(pub fn ${ident(rust)} = json!(${lit.replaceAll("__ID__", "create_id()")}));`)
            continue
          }
        }
        parts.push(`// HAND-PORT ${name}:\n// ${src.split("\n").join("\n// ")}`)
      } else if (typeof v === "number") {
        parts.push(`pub const ${upper(name)}: ${Number.isInteger(v) && v >= 0 ? "usize" : "f64"} = ${Number.isInteger(v) && v >= 0 ? v : v.toString().includes(".") ? v : `${v}.0`};`)
      } else if (typeof v === "string") {
        parts.push(`pub const ${upper(name)}: &str = ${strLit(v)};`)
      } else if (Array.isArray(v) && v.every((x) => typeof x === "string")) {
        parts.push(`pub const ${upper(name)}: &[&str] = &[${v.map(strLit).join(", ")}];`)
      } else if (Array.isArray(v) && v.every((x) => x && x._zod)) {
        parts.push(`pub fn ${ident(rust)}() -> Vec<Schema> {\n    vec![${v.map((x: any) => walk(x, rust)).join(", ")}]\n}`)
      } else if (isPlainData(v)) {
        parts.push(`pub fn ${ident(rust)}() -> &'static Value {\n    static V: LazyLock<Value> = LazyLock::new(|| json!(${JSON.stringify(v)}));\n    &V\n}`)
      } else {
        let shown = ""
        try {
          shown = JSON.stringify(v, (_k, x) => (typeof x === "function" ? `<fn ${x.name}>` : x && x._zod ? `<schema ${exportBySchema.get(x)?.name ?? "?"}>` : x)).slice(0, 1500)
        } catch {}
        parts.push(`// HAND-PORT ${name} (object): ${shown}`)
      }
    } catch (e) {
      parts.push(`// GEN-ERROR ${name}: ${(e as Error).message}`)
    }
  }
  for (const [hs, hn] of hoisted) {
    parts.push(`schema!(fn ${hn} = ${walk(hs, hn, true)});`)
  }
  if (!newHoist) break
 }
  const rel = relative(ROOT, file).replace(/\.ts$/, ".rs").replace(/-/g, "_")
  const outPath = join(OUT, rel)
  mkdirSync(dirname(outPath), { recursive: true })
  const header = `//! Port of \`packages/flow-config/src/${relative(ROOT, file)}\`.\n\n#[allow(unused_imports)]\nuse crate::flow_config::prelude::*;\n\n`
  writeFileSync(outPath, header + parts.join("\n\n") + "\n\n" + stubs.join("\n"))
}

// deps
{
  currentFile = "deps"
  stubs = []
  fnNames.clear()
  fileStubNames.clear()
  const parts: string[] = []
  const seen = new Set<string>()
  let changed = true
  while (changed) {
    changed = false
    for (const [name, v] of usedDeps) {
      if (seen.has(name)) continue
      seen.add(name)
      changed = true
      parts.push(`schema!(pub fn ${snake(name)} = ${walk(v, snake(name), true)});`)
    }
  }
  writeFileSync(join(OUT, "deps.rs"), `//! Schemas from \`@chatbotx.io/utils\` used by flow-config.\n\n#[allow(unused_imports)]\nuse crate::flow_config::prelude::*;\n\n` + parts.join("\n\n") + "\n\n" + stubs.join("\n"))
}
console.log("done", modules.length, "files ->", OUT)
