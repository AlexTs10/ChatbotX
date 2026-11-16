// Generate crates/api/src/rpc/table.rs: every procedure of the builder's private
// oRPC router (apps/builder/src/routers/index.ts) with its auth base, HTTP
// method, workspace mapper and input-validation order.
//
// Static scan with the TypeScript compiler API (no module is executed).
// Run from the repo root: TS=<path to typescript> node conversion/tools/rpc-table-gen/gen.mjs
import { createRequire } from "node:module"
import fs from "node:fs"
import path from "node:path"

const require = createRequire(import.meta.url)
const ts = require(process.env.TS ?? "typescript")
const ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../../..")
const SRC = path.join(ROOT, "apps/builder/src")
const OUT = path.join(ROOT, "crates/api/src/rpc/table.rs")

const parsed = new Map()
function parse(file) {
  if (!parsed.has(file)) {
    const text = fs.readFileSync(file, "utf8")
    parsed.set(file, ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true))
  }
  return parsed.get(file)
}

function resolveModule(spec, fromFile) {
  let base
  if (spec.startsWith("@/")) base = path.join(SRC, spec.slice(2))
  else if (spec.startsWith(".")) base = path.resolve(path.dirname(fromFile), spec)
  else if (spec === "@chatbotx.io/analytics-nextjs/routes")
    base = path.join(ROOT, "packages/analytics-nextjs/src/routes/index")
  else return null
  for (const c of [base + ".ts", base + ".tsx", path.join(base, "index.ts")])
    if (fs.existsSync(c)) return c
  throw new Error(`cannot resolve ${spec} from ${fromFile}`)
}

// Finds the declaration of `name` in `file`: a local const initializer, or an import / re-export.
function findSymbol(name, file) {
  const sf = parse(file)
  for (const st of sf.statements) {
    if (ts.isVariableStatement(st)) {
      for (const d of st.declarationList.declarations)
        if (ts.isIdentifier(d.name) && d.name.text === name && d.initializer)
          return { kind: "expr", expr: d.initializer, file }
    }
    if (ts.isImportDeclaration(st) && st.importClause?.name?.text === name)
      return { kind: "import", module: st.moduleSpecifier.text, name: "default", file }
    if (name === "default" && ts.isExportAssignment(st)) return { kind: "expr", expr: st.expression, file }
    if (ts.isImportDeclaration(st) && st.importClause?.namedBindings && ts.isNamedImports(st.importClause.namedBindings)) {
      for (const el of st.importClause.namedBindings.elements)
        if (el.name.text === name)
          return { kind: "import", module: st.moduleSpecifier.text, name: (el.propertyName ?? el.name).text, file }
    }
    if (ts.isExportDeclaration(st) && st.moduleSpecifier) {
      if (st.exportClause && ts.isNamedExports(st.exportClause)) {
        for (const el of st.exportClause.elements)
          if (el.name.text === name)
            return { kind: "import", module: st.moduleSpecifier.text, name: (el.propertyName ?? el.name).text, file }
      } else if (!st.exportClause) {
        const target = resolveModule(st.moduleSpecifier.text, file)
        if (target) {
          const found = findSymbol(name, target)
          if (found) return found
        }
      }
    }
  }
  return null
}

const BASES = {
  authorizedAPI: "Session",
  channelApiTokenAPI: "ChannelToken",
  mobileUserAPI: "MobileUser",
  mobileUserPasswordChangeAPI: "MobileUserPasswordChange",
  os: "Public",
  base: "Public",
}

// Resolves an identifier to { kind: "base", base } or an expression.
function resolveIdent(name, file) {
  const sym = findSymbol(name, file)
  if (!sym) {
    if (BASES[name]) return { base: BASES[name] }
    throw new Error(`unknown identifier ${name} in ${file}`)
  }
  if (sym.kind === "import") {
    if (sym.module === "@/orpc" || sym.module === "@orpc/server") {
      if (!BASES[sym.name]) return { tokenFactory: sym.name }
      return { base: BASES[sym.name] }
    }
    const target = resolveModule(sym.module, sym.file)
    if (!target) throw new Error(`external ${sym.module} for ${name}`)
    return resolveIdent(sym.name, target)
  }
  return { expr: sym.expr, file: sym.file }
}

function unwrap(expr) {
  while (ts.isAsExpression(expr) || ts.isSatisfiesExpression?.(expr) || ts.isParenthesizedExpression(expr) || ts.isTypeAssertionExpression?.(expr))
    expr = expr.expression
  return expr
}

function literal(node, file) {
  node = unwrap(node)
  if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) return node.text
  if (ts.isIdentifier(node)) {
    const r = resolveIdent(node.text, file)
    if (r.expr) return literal(r.expr, r.file)
  }
  if (ts.isTemplateExpression(node)) return node.getText()
  return node.getText()
}

// The workspace mapper of `.use(workspaceAuthorizedMidddleware, mapper)`: the input path it reads.
function mapperPath(node, file) {
  node = unwrap(node)
  if (ts.isIdentifier(node)) {
    const r = resolveIdent(node.text, file)
    return mapperPath(r.expr, r.file)
  }
  if (ts.isArrowFunction(node) || ts.isFunctionExpression(node)) {
    let body = node.body
    if (ts.isBlock(body)) {
      const ret = body.statements.find(ts.isReturnStatement)
      body = ret?.expression
    }
    body = unwrap(body)
    const p = node.parameters[0]
    if (ts.isObjectBindingPattern(p.name) && ts.isIdentifier(body)) return [body.text]
    const parts = []
    let e = body
    while (ts.isPropertyAccessExpression(e) || ts.isNonNullExpression(e)) {
      if (ts.isNonNullExpression(e)) { e = e.expression; continue }
      parts.unshift(e.name.text)
      e = e.expression
    }
    if (ts.isIdentifier(e) && parts.length) return parts
  }
  throw new Error(`mapper ${node.getText()} in ${file}`)
}

// A procedure chain: base + middlewares + route config. Returns null when the expression is not one.
function chain(expr, file) {
  expr = unwrap(expr)
  if (ts.isIdentifier(expr)) {
    const r = resolveIdent(expr.text, file)
    if (r.base) return { base: r.base, method: null, path: null, workspace: null, validateFirst: false, hasInput: false, router: false }
    if (r.tokenFactory) return null
    return chain(r.expr, r.file)
  }
  if (ts.isCallExpression(expr)) {
    const callee = unwrap(expr.expression)
    if (ts.isIdentifier(callee) && callee.text === "lazy") return null
    if (ts.isIdentifier(callee)) {
      const r = resolveIdent(callee.text, file)
      if (r.tokenFactory === "workspaceTokenAuthAPIForScope") {
        return { base: "Token", scope: literal(expr.arguments[0], file), method: null, path: null, workspace: null, validateFirst: false, hasInput: false, router: false }
      }
      // A local helper that returns a chain (`const memberAPI = (schema) => authorizedAPI.input(schema)...`).
      if (r.expr && ts.isArrowFunction(unwrap(r.expr)) && !ts.isBlock(unwrap(r.expr).body)) return chain(unwrap(r.expr).body, r.file)
      return null
    }
    if (ts.isPropertyAccessExpression(callee)) {
      const inner = chain(callee.expression, file)
      if (!inner) return null
      const m = callee.name.text
      const c = { ...inner }
      if (m === "route") {
        const cfg = unwrap(expr.arguments[0])
        for (const p of cfg.properties ?? []) {
          if (!p.name) continue
          if (p.name.text === "method") c.method = literal(p.initializer, file)
          if (p.name.text === "path") c.path = literal(p.initializer, file)
        }
      } else if (m === "input") {
        c.hasInput = true
      } else if (m === "use") {
        const mw = unwrap(expr.arguments[0])
        if (ts.isIdentifier(mw) && mw.text === "workspaceAuthorizedMidddleware") {
          c.workspace = mapperPath(expr.arguments[1], file)
          c.validateFirst = c.hasInput
        }
      } else if (m === "router") {
        c.router = true
        c.routerArg = expr.arguments[0]
        c.routerFile = file
      }
      return c
    }
  }
  return null
}

const out = []
function walk(expr, file, prefix, inherited) {
  expr = unwrap(expr)
  if (ts.isIdentifier(expr)) {
    const r = resolveIdent(expr.text, file)
    if (r.expr) return walk(r.expr, r.file, prefix, inherited)
    throw new Error(`bare base at ${prefix.join(".")}`)
  }
  if (ts.isObjectLiteralExpression(expr)) {
    for (const p of expr.properties) {
      if (ts.isSpreadAssignment(p)) walk(p.expression, file, prefix, inherited)
      else if (ts.isPropertyAssignment(p)) walk(p.initializer, file, [...prefix, p.name.getText().replace(/^["']|["']$/g, "")], inherited)
      else if (ts.isShorthandPropertyAssignment(p)) walk(p.name, file, [...prefix, p.name.text], inherited)
      else throw new Error(`property kind ${ts.SyntaxKind[p.kind]} at ${prefix.join(".")}`)
    }
    return
  }
  const c = chain(expr, file)
  if (!c) throw new Error(`not a router or procedure at ${prefix.join(".")}: ${expr.getText().slice(0, 80)}`)
  if (c.router) {
    const inh = inherited ?? { base: c.base, scope: c.scope, workspace: c.workspace }
    return walk(c.routerArg, c.routerFile, prefix, c.base === "Public" && inherited ? inherited : inh)
  }
  let proc = c
  if (inherited && c.base === "Public") {
    // A procedure of a router that gets its auth from the parent (`authorizedAPI.use(...).router(...)`):
    // router-level middlewares run before the procedure's input validation.
    proc = { ...c, base: inherited.base, scope: inherited.scope, workspace: inherited.workspace ?? c.workspace, validateFirst: inherited.workspace ? false : c.validateFirst }
  }
  out.push({ path: prefix.join("."), base: proc.base, scope: proc.scope ?? null, method: proc.method, httpPath: proc.path, workspace: proc.workspace, validateFirst: proc.validateFirst })
}

const indexFile = path.join(SRC, "routers/index.ts")
const routerExpr = findSymbol("router", indexFile)
for (const p of unwrap(routerExpr.expr).properties) {
  const key = p.name.text
  let init = unwrap(p.initializer)
  if (ts.isCallExpression(init) && ts.isIdentifier(init.expression) && init.expression.text === "lazy") {
    // lazy(() => import("x").then((m) => ({ default: m.Y })))
    const text = init.getText()
    const mod = /import\("([^"]+)"\)/.exec(text)[1]
    const name = /default:\s*m\.([A-Za-z0-9_]+)/.exec(text)[1]
    const file = resolveModule(mod, indexFile)
    const r = resolveIdent(name, file)
    walk(r.expr, r.file, [key], null)
  } else {
    walk(init, indexFile, [key], null)
  }
}

const esc = (s) => JSON.stringify(s)
const opt = (s) => (s == null ? "None" : `Some(${esc(s)})`)
let rs = `//! The procedures of the builder's private oRPC router (\`routers/index.ts\`).
//! Generated by \`conversion/tools/rpc-table-gen/gen.mjs\`; do not edit.

use super::{RpcAuth, RpcProcedure};

/// Every procedure, in router order.
pub const RPC_PROCEDURES: &[RpcProcedure] = &[
`
for (const p of out) {
  const auth = p.base === "Token" ? `RpcAuth::Token(${esc(p.scope)})` : `RpcAuth::${p.base}`
  const ws = p.workspace ? `Some(&[${p.workspace.map(esc).join(", ")}])` : "None"
  rs += `    RpcProcedure { path: ${esc(p.path)}, auth: ${auth}, method: ${opt(p.method)}, workspace: ${ws}, validate_first: ${p.validateFirst} },\n`
}
rs += "];\n"
fs.mkdirSync(path.dirname(OUT), { recursive: true })
fs.writeFileSync(OUT, rs)
const counts = {}
for (const p of out) counts[p.base] = (counts[p.base] ?? 0) + 1
console.log(out.length, "procedures", JSON.stringify(counts), "workspace:", out.filter((p) => p.workspace).length, "keys:", new Set(out.map((p) => p.path.split(".")[0])).size)
