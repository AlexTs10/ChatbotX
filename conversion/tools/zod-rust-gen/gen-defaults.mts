// Emits Rust for `*DefaultFn(props)` factories that are `{ ...defaults, ...props, ...fixed }`
// (detected by probing); prints COMPLEX for the rest (hand-ported).
const fc = await import("/Users/alextoska/Desktop/ChatbotX-lean/packages/flow-config/src/index.ts")
const snake = (s: string) =>
  s.replace(/([a-z0-9])([A-Z])/g, "$1_$2").replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2").toLowerCase()
const isId = (a: unknown, b: unknown) => typeof a === "string" && typeof b === "string" && a !== b && /^\d{10,}$/.test(a)
const lit = (a: any, b: any): string => {
  if (isId(a, b)) return "create_id()"
  if (Array.isArray(a)) return `[${a.map((x, i) => lit(x, b?.[i])).join(", ")}]`
  if (a && typeof a === "object")
    return `{${Object.entries(a).filter(([, v]) => v !== undefined).map(([k, v]) => `${JSON.stringify(k)}: ${lit(v, b?.[k])}`).join(", ")}}`
  return JSON.stringify(a) ?? "null"
}
for (const [name, fn] of Object.entries(fc) as [string, any][]) {
  if (typeof fn !== "function" || !/DefaultFn$/.test(name)) continue
  const src = fn.toString()
  if (!/^\(?\s*props/.test(src)) continue
  let a: any, b: any
  try { a = fn({}); b = fn({}) } catch { console.log(`// COMPLEX ${name} (throws)`); continue }
  const keys = Object.keys(a)
  const before: string[] = []
  const after: string[] = []
  let complex = false
  for (const k of keys) {
    let out: any
    try { out = fn({ [k]: "__PROBE__" }) } catch { complex = true; break }
    if (out[k] === "__PROBE__") before.push(k)
    else after.push(k)
    for (const k2 of keys) {
      if (k2 === k) continue
      const x = JSON.stringify(out[k2]), y = JSON.stringify(a[k2])
      if (x !== y && !(typeof out[k2] === "object") && !isId(out[k2], a[k2])) complex = true
    }
  }
  const extra = fn({ __extra__: 1 })
  if (extra.__extra__ !== 1) complex = true
  if (complex) { console.log(`// COMPLEX ${name}`); continue }
  const pick = (ks: string[]) => Object.fromEntries(ks.map((k) => [k, a[k]]))
  const pickB = (ks: string[]) => Object.fromEntries(ks.map((k) => [k, b[k]]))
  const base = lit(pick(before), pickB(before))
  const fixed = lit(pick(after), pickB(after))
  const r = snake(name)
  const body = after.length ? `spread3(json!(${base}), props, json!(${fixed}))` : `spread_props(json!(${base}), props)`
  console.log(`/// \`${name}(props)\`.\npub fn ${r}(props: &Value) -> Value {\n    ${body}\n}\n`)
}
