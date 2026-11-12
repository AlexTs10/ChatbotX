// Lists vitest cases of the given test files: file \t line \t "describe > name" (one row per test/it, .each rows once).
import ts from "/Users/alextoska/Desktop/ChatbotX-lean/node_modules/.pnpm/typescript@5.9.3/node_modules/typescript/lib/typescript.js"
import { readFileSync } from "node:fs"
for (const file of process.argv.slice(2)) {
  const src = readFileSync(file, "utf8")
  const sf = ts.createSourceFile(file, src, ts.ScriptTarget.Latest, true)
  const visit = (node: ts.Node, stack: string[]) => {
    if (ts.isCallExpression(node)) {
      let callee = node.expression
      let isEach = false
      if (ts.isCallExpression(callee)) {
        const inner = callee.expression
        if (ts.isPropertyAccessExpression(inner) && inner.name.text === "each") {
          isEach = true
          callee = inner.expression
        }
      }
      const name = ts.isIdentifier(callee) ? callee.text : ts.isPropertyAccessExpression(callee) ? callee.getText() : ""
      const arg = node.arguments[0]
      const title = arg && (ts.isStringLiteral(arg) || ts.isNoSubstitutionTemplateLiteral(arg)) ? arg.text : arg ? arg.getText() : ""
      if (name === "describe") {
        const body = node.arguments[1]
        if (body) ts.forEachChild(body, (c) => visit(c, [...stack, title]))
        return
      }
      if (name === "test" || name === "it" || name === "test.skip" || name === "it.skip") {
        const line = sf.getLineAndCharacterOfPosition(node.getStart()).line + 1
        console.log(`${file}\t${line}\t${[...stack, title].join(" > ")}\t${isEach ? "each" : ""}`)
        return
      }
    }
    ts.forEachChild(node, (c) => visit(c, stack))
  }
  visit(sf, [])
}
