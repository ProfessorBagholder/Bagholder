import { describe, expect, it } from 'vitest'
import ts from 'typescript'

// No failure is dropped on the page (docs/plans/stage-5-interface-and-running.md, B):
// every call to the server is answered with its route's answer or a failure
// (`api.ts`), and the caller that made it says the failure where the app says one.
// The type checker refuses a caller that reads an answer without meeting its failure;
// this refuses the two ways past it: a call whose answer is never looked at (the call
// as a statement, `void`ed, awaited and let go, or followed by a `.then` that does not
// read what it was given), and a `catch` that swallows what it caught. A `catch` that
// has nothing to do says why in a comment in its block, and "ignore" is not a why.
// Read with TypeScript's own parser: the page's .ts files and its .svelte script blocks.
const sources = import.meta.glob(['/src/**/*.svelte', '/src/**/*.ts', '!/src/**/*.test.ts', '!/src/lib/generated/**'], {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

/** The script of a file, each piece with the line it starts on: a .ts whole, a .svelte's <script> blocks. */
function scripts(path: string, text: string): { code: string; line: number }[] {
  if (!path.endsWith('.svelte')) return [{ code: text, line: 1 }]
  const out: { code: string; line: number }[] = []
  for (const m of text.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)) {
    const start = m.index + m[0].indexOf('>') + 1
    out.push({ code: m[1], line: text.slice(0, start).split('\n').length })
  }
  return out
}

const API_MODULE = /(^|\/)api$/
const API_FUNCTIONS = ['request', 'get', 'post', 'call', 'lookup', 'searchSymbols'] // api.ts's own, for api.ts itself
const SAYS_NOTHING = /^(ignored?|noop|no-op|nothing|empty|swallow(ed)?)\.?$/i

/** Why each dropped failure in `code` is one, as `line: what`. */
function dropped(code: string, fileName = 'x.ts', firstLine = 1): string[] {
  const sf = ts.createSourceFile(fileName, code, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
  const found: string[] = []
  const at = (n: ts.Node, what: string) => found.push(`${sf.getLineAndCharacterOfPosition(n.getStart(sf)).line + firstLine}: ${what}: ${n.getText(sf).split('\n')[0].trim()}`)

  // the names that reach the server here: the api module's functions, as imported, and its lookups
  const api = new Set<string>(/(^|\/)api\.ts$/.test(fileName) ? API_FUNCTIONS : [])
  for (const s of sf.statements) {
    if (!ts.isImportDeclaration(s) || !ts.isStringLiteral(s.moduleSpecifier) || !API_MODULE.test(s.moduleSpecifier.text)) continue
    const named = s.importClause?.namedBindings
    if (named && ts.isNamedImports(named)) for (const e of named.elements) if (!e.isTypeOnly) api.add(e.name.text)
  }
  const lookups = new Set<string>()
  const findLookups = (n: ts.Node): void => {
    if (ts.isVariableDeclaration(n) && ts.isIdentifier(n.name) && n.initializer && ts.isCallExpression(n.initializer) && ts.isIdentifier(n.initializer.expression) && n.initializer.expression.text === 'lookup' && api.has('lookup'))
      lookups.add(n.name.text)
    ts.forEachChild(n, findLookups)
  }
  findLookups(sf)

  const isApiCall = (e: ts.Expression): boolean => {
    if (!ts.isCallExpression(e)) return false
    const f = e.expression
    if (ts.isIdentifier(f)) return api.has(f.text)
    return ts.isPropertyAccessExpression(f) && f.name.text === 'read' && ts.isIdentifier(f.expression) && lookups.has(f.expression.text)
  }
  const unwrap = (e: ts.Expression): ts.Expression => {
    for (;;) {
      if (ts.isParenthesizedExpression(e) || ts.isVoidExpression(e) || ts.isAwaitExpression(e)) e = e.expression
      else return e
    }
  }
  const method = (e: ts.Expression): { on: ts.Expression; name: string; args: readonly ts.Expression[] } | null =>
    ts.isCallExpression(e) && ts.isPropertyAccessExpression(e.expression) ? { on: e.expression.expression, name: e.expression.name.text, args: e.arguments } : null
  // a function that does nothing: an empty block with no comment saying why, or `undefined`, `null`, `void 0`
  const doesNothing = (f: ts.Expression | undefined): boolean => {
    if (!f || !(ts.isArrowFunction(f) || ts.isFunctionExpression(f))) return false
    const body = f.body
    if (ts.isBlock(body)) return body.statements.length === 0 && !reason(body)
    const b = unwrap(body)
    return b.kind === ts.SyntaxKind.NullKeyword || (ts.isIdentifier(b) && b.text === 'undefined') || (ts.isVoidExpression(body) && ts.isNumericLiteral(body.expression))
  }
  // the comment an empty block carries, as its reason; one that says nothing is none
  const reason = (b: ts.Block): boolean => {
    const inner = b.getText(sf).slice(1, -1)
    const words = inner.replace(/\/\*|\*\/|\/\//g, ' ').replace(/\s+/g, ' ').trim()
    return words !== '' && !SAYS_NOTHING.test(words)
  }
  // a callback reads what it was given: its first parameter is named and used
  const reads = (f: ts.Expression | undefined): boolean => {
    if (!f) return false
    if (ts.isIdentifier(f) || ts.isPropertyAccessExpression(f)) return true // a named handler, given the answer
    if (!(ts.isArrowFunction(f) || ts.isFunctionExpression(f))) return false
    const p = f.parameters[0]
    if (!p) return false
    if (!ts.isIdentifier(p.name)) return true // destructured: read by the pattern
    const name = p.name.text
    let used = false
    const look = (n: ts.Node): void => {
      if (used) return
      if (ts.isIdentifier(n) && n.text === name && n !== p.name) used = true
      else ts.forEachChild(n, look)
    }
    look(f.body)
    return used
  }

  const visit = (n: ts.Node): void => {
    if (ts.isCatchClause(n) && n.block.statements.length === 0 && !reason(n.block)) at(n, 'an empty catch')
    const m = ts.isCallExpression(n) ? method(n) : null
    if (m && m.name === 'catch' && doesNothing(m.args[0])) at(n, 'a .catch that does nothing')
    if (m && m.name === 'then' && doesNothing(m.args[1])) at(n, 'a .then whose failure handler does nothing')
    if (ts.isExpressionStatement(n)) {
      // down the chain to the call that reached the server, noting the handler hung directly on it
      let e = unwrap(n.expression)
      let handled = false
      for (;;) {
        const c = method(e)
        if (isApiCall(e)) break
        if (!c) {
          e = unwrap(e)
          break
        }
        const on = unwrap(c.on)
        if (isApiCall(on)) handled = c.name === 'then' && reads(c.args[0])
        e = on
      }
      if (isApiCall(e) && !handled) at(n, 'a call whose answer is dropped')
    }
    ts.forEachChild(n, visit)
  }
  visit(sf)
  return found
}

describe('the page drops no failure', () => {
  it('reads the page source', () => {
    expect(Object.keys(sources).length).toBeGreaterThan(20)
  })

  it('has no empty catch and no call whose answer is dropped', () => {
    const found: string[] = []
    for (const [path, text] of Object.entries(sources))
      for (const s of scripts(path, text)) for (const f of dropped(s.code, path, s.line)) found.push(`${path}:${f}`)
    expect(found).toEqual([])
  })

  it('reads a .svelte file\'s script blocks, on their own lines', () => {
    const file = '<script lang="ts">\n  import { call } from \'./api\'\n  call(\'POST /api/sync\')\n</script>\n\n<p>x</p>\n'
    const [s] = scripts('/src/X.svelte', file)
    expect(dropped(s.code, '/src/X.svelte', s.line)).toEqual(["3: a call whose answer is dropped: call('POST /api/sync')"])
  })

  it('catches each form it looks for', () => {
    const head = "import { call, post, request, lookup } from '../api'\nconst bars = lookup('GET /api/history')\n"
    const forms = [
      'try { f() } catch {}',
      'try { f() } catch (e) {}',
      'try { f() } catch { /* ignore */ }',
      'p.catch(() => {})',
      'p.catch(() => undefined)',
      'p.then((a) => use(a), () => {})',
      "call('POST /api/sync')",
      "void call('POST /api/sync')",
      "async function f() { await call('POST /api/sync') }",
      "call('POST /api/sync').then(() => done())",
      "call('POST /api/sync').then((r) => done())",
      "call('POST /api/sync').finally(() => done())",
      "post('/api/events/resync', { id: 1 })",
      "request('GET', '/api/status')",
      'bars.read({ query: q })',
    ]
    for (const f of forms) expect(dropped(head + f), f).toHaveLength(1)
  })

  it('lets a failure met, or a catch that says why, stand', () => {
    const head = "import { call, lookup } from '../api'\nconst bars = lookup('GET /api/history')\n"
    const forms = [
      'try { f() } catch { /* the choice holds for this visit; the browser keeps nothing */ }',
      "try { f() } catch { return 'all' }",
      "async function f() { const r = await call('POST /api/sync'); if (!r.ok) say(r.error) }",
      "call('POST /api/sync').then((r) => { if (!r.ok) say(r.error) })",
      "call('POST /api/sync').then(scanned)",
      "async function f() { return call('POST /api/sync') }",
      'async function f() { const a = await bars.read({ query: q }); use(a) }',
      "call2('x')", // not the api's
    ]
    for (const f of forms) expect(dropped(head + f), f).toEqual([])
  })
})
