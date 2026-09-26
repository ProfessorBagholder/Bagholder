import { test } from '@playwright/test'
import { writeFileSync } from 'node:fs'
const OUT = '/private/tmp/claude-501/-Users-md-dev-Bagholder/4156ee24-aa1e-453b-9468-2ae062980e70/scratchpad/parity'
const widths = ('1200,1340,1440,1680').split(',').map(Number)
const only = ['holding']
const grab = (p) => p.evaluate(() => {
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
const scenes = [
  { name: 'trade', tab: 'trades', row: true },
  { name: 'holding', tab: 'portfolio/rt%3Ademo-0056' },
  { name: 'ticket', tab: 'dashboard', steps: async (p) => { await p.keyboard.press('Control+k'); await p.waitForTimeout(500); if (!(await p.getByRole('textbox', { name: 'Search' }).count())) await p.keyboard.press('Meta+k'); await p.getByRole('textbox', { name: 'Search' }).first().fill('NVDA', { timeout: 5000 }); await p.waitForTimeout(800); await p.getByRole('button', { name: 'Buy NVDA', exact: true }).first().click({ timeout: 5000 }); await p.waitForTimeout(1500) } },
  { name: 'orders', tab: 'dashboard', click: 'Orders' },
  { name: 'notes', tab: 'dashboard', click: 'Notifications' },
  { name: 'menu', tab: 'dashboard', click: 'Menu' },
  { name: 'filter', tab: 'dashboard', click: 'Filters' },
]
test('scenes', async ({ browser }) => {
  const res = {}, log = []
  for (const w of widths) {
    const ctx = await browser.newContext({ viewport: { width: w, height: 1000 }, colorScheme: 'dark', timezoneId: 'America/Toronto' })
    const p = await ctx.newPage()
    for (const sc of scenes.filter((s) => !only || only.includes(s.name))) {
      let hash = sc.tab
      for (const [name, path] of [['svelte', '/'], ['legacy', '/ledger.html']]) {
        await p.goto('http://127.0.0.1:8793' + path + '#' + (name === 'legacy' ? hash : sc.tab))
        await p.waitForTimeout(2000)
        if (sc.row && name === 'svelte') {
          await p.locator('tbody tr').first().click()
          await p.waitForTimeout(1500)
          hash = (await p.evaluate(() => location.hash)).slice(1)
          await p.goto('http://127.0.0.1:8793/#' + hash)
          await p.reload()
          await p.waitForTimeout(2500)
        }
        await p.mouse.move(0, 0)
        if (sc.steps) { try { await sc.steps(p) } catch (e) { log.push(sc.name + ' ' + name + ' steps failed: ' + String(e).slice(0, 200)) } }
        if (sc.steps) { try { await sc.steps(p) } catch (e) { log.push(sc.name + ' ' + name + ' steps failed: ' + String(e).slice(0, 200)) } }
        if (sc.click) {
          const b = typeof sc.click === 'string' ? p.getByRole('button', { name: sc.click, exact: true }) : p.getByRole('button', { name: sc.click })
          if (await b.count()) { await b.first().click(); await p.waitForTimeout(1200) } else log.push(sc.name + ' ' + name + ' no button')
        }
        await p.screenshot({ path: OUT + '/' + sc.name + '-' + w + '-' + name + '.png', fullPage: true })
        res[sc.name + '-' + w + '-' + name] = await grab(p)
      }
      log.push(sc.name + ' ' + hash)
    }
    await ctx.close()
  }
  writeFileSync(OUT + '/scenes.json', JSON.stringify(res))
  console.log(log.join('\n'))
})
