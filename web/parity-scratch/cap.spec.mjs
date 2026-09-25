import { test, chromium } from '@playwright/test'
import { writeFileSync } from 'node:fs'
const OUT = process.env.OUT || '/private/tmp/claude-501/-Users-md-dev-Bagholder/4156ee24-aa1e-453b-9468-2ae062980e70/scratchpad/parity', PORT = process.env.PORT_ || '8793'
const widths = (process.env.WIDTHS || '1200,1340,1440,1680').split(',').map(Number)
const tabs = (process.env.TABS || 'dashboard,trades,portfolio,markets,cashflow').split(',')
const pages = { svelte: '/', legacy: '/ledger.html' }
test('capture', async ({ browser: b }) => {
  const res = {}
  for (const w of widths) for (const [name, path] of Object.entries(pages)) {
    const ctx = await b.newContext({ viewport: { width: w, height: 1000 }, colorScheme: 'dark', timezoneId: 'America/Toronto' })
    const p = await ctx.newPage()
    for (const tab of tabs) {
      await p.goto(`http://127.0.0.1:${PORT}${path}#${tab}`)
      await p.waitForTimeout(2500)
      await p.screenshot({ path: `${OUT}/${tab}-${w}-${name}.png`, fullPage: true })
      res[`${tab}-${w}-${name}`] = await p.evaluate(() => {
        const out = []
        const walk = (el) => {
          for (const c of el.children) walk(c)
          const own = [...el.childNodes].filter(n => n.nodeType === 3).map(n => n.textContent.trim()).join(' ').trim()
          if (!own) return
          const r = el.getBoundingClientRect(); if (!r.width || !r.height) return
          const s = getComputedStyle(el); if (s.visibility === 'hidden' || s.display === 'none') return
          out.push({ t: own.slice(0, 60), x: Math.round(r.x), y: Math.round(r.y + scrollY), w: Math.round(r.width), h: Math.round(r.height), fs: s.fontSize, fw: s.fontWeight, c: s.color, tag: el.tagName })
        }
        walk(document.body)
        return { items: out, docW: document.documentElement.scrollWidth, docH: document.documentElement.scrollHeight }
      })
    }
    await ctx.close()
  }
  writeFileSync(`${OUT}/cap.json`, JSON.stringify(res))
  console.log('ok', Object.keys(res).length)
})
