// The squarified treemap (Bruls, Huizing & van Wijk) and the heatmap's colour,
// ported verbatim from the legacy page's algorithm so the tiles lay out and colour
// identically. The blocks, their tiles, their values and changes and the folded
// `Other (N)` tiles are the server's (`HeatmapDoc`); only the geometry is worked
// out here, each value a plotted size (`plot`).

import { plot } from '../dec'
import type { HeatBlock, HeatTile } from '../model'

interface Rect { x: number; y: number; w: number; h: number }
interface Sized { v: number }
type Cell<T> = T & Rect
type WithV<T> = T & Sized

function squarify<T extends Sized>(items: T[], rect: Rect): Cell<T>[] {
  const total = items.reduce((s, i) => s + i.v, 0)
  if (!total || rect.w <= 0 || rect.h <= 0) return []
  const scale = (rect.w * rect.h) / total
  const list = items.map((i) => ({ ...i, a: i.v * scale })).sort((a, b) => b.a - a.a)
  const r = { ...rect }
  let res: Cell<T>[] = []
  let row: (T & { a: number })[] = []
  const worst = (rw: { a: number }[], len: number) => {
    const s = rw.reduce((a, b) => a + b.a, 0)
    const mx = Math.max(...rw.map((b) => b.a))
    const mn = Math.min(...rw.map((b) => b.a))
    return Math.max((len * len * mx) / (s * s), (s * s) / (len * len * mn))
  }
  const layoutRow = (rw: (T & { a: number })[]): Cell<T>[] => {
    const s = rw.reduce((a, b) => a + b.a, 0)
    const out: Cell<T>[] = []
    if (r.w >= r.h) {
      const thick = s / r.h
      let y = r.y
      rw.forEach((it) => { const hh = it.a / thick; out.push({ ...(it as T), x: r.x, y, w: thick, h: hh }); y += hh })
      r.x += thick; r.w -= thick
    } else {
      const thick = s / r.w
      let x = r.x
      rw.forEach((it) => { const ww = it.a / thick; out.push({ ...(it as T), x, y: r.y, w: ww, h: thick }); x += ww })
      r.y += thick; r.h -= thick
    }
    return out
  }
  while (list.length) {
    const len = Math.min(r.w, r.h)
    const next = list[0]
    if (!row.length || worst(row.concat([next]), len) <= worst(row, len)) row.push(list.shift()!)
    else { res = res.concat(layoutRow(row)); row = [] }
  }
  if (row.length) res = res.concat(layoutRow(row))
  return res
}

export interface Block { label: string; x: number; y: number; w: number; h: number; chg: number | null }
export interface HeatCell extends HeatTile { v: number; x: number; y: number; w: number; h: number; big: boolean; mid: boolean }

export function heatmapLayout(blocks: HeatBlock[], W: number, H: number): { blocks: Block[]; cells: HeatCell[] } {
  const GAP = 6, HEAD = 19
  let groups = blocks.map((b) => ({ label: b.label, v: plot(b.value), chg: b.percentChange, tiles: b.tiles }))
  const gv = groups.reduce((a, b) => a + b.v, 0)
  const gArea = W * H
  const gMinArea = Math.min(52 * Math.min(W, H), gArea * 0.22)
  const gMinV = Math.min(gv / groups.length, (gv * gMinArea) / gArea)
  groups = groups.map((g) => ({ ...g, v: Math.max(g.v, gMinV) }))
  const cells: HeatCell[] = []
  const placed: Block[] = squarify(groups as WithV<(typeof groups)[number]>[], { x: 0, y: 0, w: W, h: H }).map((g) => {
    const inner = { x: g.x + GAP / 2, y: g.y + GAP / 2 + HEAD, w: Math.max(1, g.w - GAP), h: Math.max(1, g.h - GAP - HEAD) }
    let tiles: (HeatTile & { v: number })[] = g.tiles.map((t) => ({ ...t, v: plot(t.value) }))
    const tv = tiles.reduce((a, b) => a + b.v, 0)
    const area = inner.w * inner.h
    const minArea = Math.min(52 * Math.min(inner.w, inner.h), area * 0.22)
    const minV = Math.min(tv / tiles.length, (tv * minArea) / area)
    tiles = tiles.map((x) => ({ ...x, v: Math.max(x.v, minV) }))
    squarify(tiles, inner).forEach((c) => {
      const big = c.w > 128 && c.h > 62
      const mid = c.w > 78 && c.h > 40
      cells.push({ ...(c as HeatCell), big, mid })
    })
    return { label: g.label, x: g.x, y: g.y, w: g.w, h: g.h, chg: g.chg }
  })
  return { blocks: placed, cells }
}

// ledger's heatColor: the design-system heat tokens, never hardcoded hex, so the
// map reads correctly in every theme. Absent change is the faint ink wash.
// A day's change is a fraction: the steps are 0.35%, 1% and 2.5%.
export function heatColor(chg: number | null | undefined): string {
  if (chg == null || !isFinite(chg)) return 'rgba(var(--ink-rgb),.10)'
  const a = Math.abs(chg)
  const step = a < 0.0035 ? 1 : a < 0.01 ? 2 : a < 0.025 ? 3 : 4
  return 'var(--heat-' + (chg >= 0 ? 'p' : 'n') + step + ')'
}
