// Rule 0, held at the DOM: when one holding's price moves, the elements showing
// that holding's figures and the totals that include it change, and nothing else on
// the screen is touched -- no row rebuilt, no element added or removed, the holding
// beside it not so much as looked at. The page is watched with a MutationObserver
// while a change of the shape the server sends is written into the model.

import { describe, it, expect, vi, beforeEach } from 'vitest'
import { render } from '@testing-library/svelte'
import { flushSync } from 'svelte'
// a real document: the server's figures for a month of one account's recorded replies,
// each holding quoted (rust/crates/server/src/wire/build.rs keeps it the server's shape)
import pulled from './fixtures/figures_pulled_month.json'
import Portfolio from './Portfolio.svelte'
import { applyOps, type Op } from './live'
import type { Model } from './model'
import { sign, type Dec } from './dec'

// Every figure on the screen goes through a formatter, so counting their calls counts
// how much of the screen's code ran again -- which the DOM alone does not show: a
// keyed list rewrites only the text that differs even when every row was re-run.
const formatted: unknown[] = []
vi.mock('./fmt', async (orig) => {
  const real = (await orig()) as Record<string, unknown>
  const counted: Record<string, unknown> = { ...real }
  for (const [name, fn] of Object.entries(real)) {
    if (typeof fn === 'function') counted[name] = (...args: unknown[]) => { formatted.push(args[0]); return (fn as (...a: unknown[]) => unknown)(...args) }
  }
  return counted
})
beforeEach(() => { formatted.length = 0 })

// Figures that belong to the other holdings and to nothing else on the screen (a zero,
// or a figure this holding shares, would say nothing about whose code ran).
function figuresOnlyOthersHave(model: Model, one: Model['positions'][number]): Set<string> {
  const of = (p: Model['positions'][number]) => [p.last, p.mv, p.unreal, p.avg, p.cost].filter((v): v is Dec => typeof v === 'string' && sign(v) !== 0)
  const own = new Set(of(one))
  return new Set(model.positions.filter((p) => p !== one).flatMap(of).filter((v) => !own.has(v)))
}

function book(): Model {
  return structuredClone(pulled) as unknown as Model
}

// the holding whose price moves: one the document states a value for, in CAD
const moving = (m: Model) => m.positions.find((p) => p.currency === 'CAD' && typeof p.mv === 'string' && !p.short)!

function watch(root: Node) {
  const seen: MutationRecord[] = []
  const mo = new MutationObserver((r) => seen.push(...r))
  mo.observe(root, { subtree: true, childList: true, characterData: true, attributes: true })
  return () => {
    seen.push(...mo.takeRecords())
    mo.disconnect()
    return seen
  }
}

const rowOf = (n: Node | null): HTMLElement | null => {
  for (let e = n instanceof Element ? n : n?.parentElement ?? null; e; e = e.parentElement) if (e.tagName === 'TR') return e as HTMLElement
  return null
}

describe('one holding\'s price moves', () => {
  it('touches that holding\'s cells and the totals, and nothing else', () => {
    const model = $state(book())
    const { container } = render(Portfolio, { props: { model } })
    flushSync()

    const veqt = moving(model)
    const rows = [...container.querySelectorAll('tbody tr')] as HTMLElement[]
    const veqtRow = rows.find((r) => r.textContent!.includes(veqt.symbol))!
    const others = rows.filter((r) => r !== veqtRow)
    expect(veqtRow).toBeTruthy()
    expect(others.length).toBeGreaterThanOrEqual(3)
    const before = others.map((r) => r.outerHTML)

    const stop = watch(container)
    formatted.length = 0
    const otherFigures = figuresOnlyOthersHave(model, veqt)
    const row = { k: 'id', v: veqt.id }
    // what the server sends for this tick: this holding's figures, and the totals
    applyOps(model, [
      ['set', ['positions', row, 'last'], '63.1'],
      ['set', ['positions', row, 'mv'], '6310'],
      // the P&L the list is sorted by keeps its place, so the row stays where it is
      ['set', ['positions', row, 'unreal'], veqt.unreal],
      ['set', ['positions', row, 'unrealPct'], 0.2681],
      ['set', ['portfolio', 'marketValue', 'total'], '99999.99'],
      ['set', ['portfolio', 'unrealized', 'total'], '4444.44'],
    ] as Op[])
    flushSync()
    const seen = stop()

    // something did change on screen, in the holding's own row
    expect(seen.length).toBeGreaterThan(0)
    expect(veqtRow.textContent).toContain('63.10')
    // no element was added or removed anywhere: values were written, nothing was rebuilt
    // (a tile's figure rolls to its new value: wheels inside that one figure, and nowhere else)
    const rolling = (m: MutationRecord) => m.target instanceof Element && m.target.matches('.kpi .v')
    const structural = seen.filter((m) => m.type === 'childList' && !rolling(m) && [...m.addedNodes, ...m.removedNodes].some((n) => n.nodeType === Node.ELEMENT_NODE))
    expect(structural).toEqual([])
    // and not one mutation landed in any other holding's row
    const strays = seen.filter((m) => { const r = rowOf(m.target); return r !== null && r !== veqtRow })
    expect(strays.map((m) => rowOf(m.target)!.textContent)).toEqual([])
    expect(others.map((r) => r.outerHTML)).toEqual(before)
    // the rows are the same elements they were
    expect([...container.querySelectorAll('tbody tr')]).toEqual(rows)
    // and the code behind the other holdings did not run again: not one of their
    // figures was formatted. (Replacing the list, as the page used to, re-runs them all.)
    expect(formatted.length).toBeGreaterThan(0)
    expect(formatted.filter((v) => otherFigures.has(v as string))).toEqual([])
  })

  it('the old way, for contrast: replacing the list re-runs every holding for the same change', () => {
    const model = $state(book())
    render(Portfolio, { props: { model } })
    flushSync()
    const veqt = moving(model)
    const otherFigures = figuresOnlyOthersHave(model, veqt)
    formatted.length = 0
    const next = book()
    next.positions.find((p) => p.id === veqt.id)!.last = '63.1' as Dec
    model.positions = next.positions // a fresh list of fresh objects, as a whole-model reload gives
    flushSync()
    expect(formatted.filter((v) => otherFigures.has(v as string)).length).toBeGreaterThan(0)
  })

  it('a price that did not move touches nothing at all', () => {
    const model = $state(book())
    const { container } = render(Portfolio, { props: { model } })
    flushSync()
    const veqt = moving(model)
    const stop = watch(container)
    applyOps(model, [['set', ['positions', { k: 'id', v: veqt.id }, 'last'], veqt.last]] as Op[])
    flushSync()
    expect(stop()).toEqual([])
  })
})
