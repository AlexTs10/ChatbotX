// Node load hook: expose user callbacks that zod keeps only in closures, so
// the generator can name and print them (superRefine fn, default factory).
export async function load(url, context, nextLoad) {
  const result = await nextLoad(url, context)
  if (!/zod@4[^/]*\/node_modules\/zod\/v4\/(core\/api|classic\/schemas)\.js$/.test(url)) return result
  let src = typeof result.source === "string" ? result.source : new TextDecoder().decode(result.source)
  src = src.replace(
    /\}, params\);\n    return ch;\n\}\n\/\/ @__NO_SIDE_EFFECTS__\nexport function _check/,
    "}, params);\n    ch._zod.def.userFn = fn;\n    return ch;\n}\n// @__NO_SIDE_EFFECTS__\nexport function _check",
  )
  src = src.replaceAll("get defaultValue() {", "rawDefault: defaultValue,\n        get defaultValue() {")
  return { ...result, source: src }
}
