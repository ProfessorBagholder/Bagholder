import { test } from '@playwright/test'
import { readFileSync, writeFileSync } from 'node:fs'
const SP='/private/tmp/claude-501/-Users-md-dev-Bagholder/4156ee24-aa1e-453b-9468-2ae062980e70/scratchpad/parity'
test('probe', async ({ browser }) => {
  const q = JSON.parse(readFileSync(SP + '/probe.json', 'utf8'))
  const out = {}
  for (const [name, path] of Object.entries({ svelte: '/', legacy: '/ledger.html' })) {
    const ctx = await browser.newContext({ viewport: { width: q.width, height: 1000 }, colorScheme: 'dark', timezoneId: 'America/Toronto' })
    const p = await ctx.newPage()
    await p.goto('http://127.0.0.1:8793' + path + '#' + q.tab)
    await p.waitForTimeout(2500)
    if (q.shot) await p.screenshot({ path: SP + '/probe-' + name + '.png', clip: q.shot })
    out[name] = await p.evaluate((q) => q.texts.map((t) => {
      const el = [...document.querySelectorAll('body *')].find((e) => [...e.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim() === t))
      if (!el) return { t, missing: true }
      const chain = []
      let e = el
      for (let i = 0; i < (q.depth || 3) && e; i++, e = e.parentElement) {
        const s = getComputedStyle(e), r = e.getBoundingClientRect()
        const o = { tag: e.tagName, cls: e.className && e.className.baseVal === undefined ? e.className : '', rect: [r.x, r.y, r.width, r.height].map(Math.round) }
        for (const k of q.props) o[k] = s[k]
        chain.push(o)
      }
      return { t, chain }
    }), q)
    await ctx.close()
  }
  writeFileSync(SP + '/probe-out.json', JSON.stringify(out, null, 1))
})
