// The squarified treemap (Bruls, Huizing & van Wijk) and the heatmap's sector
// grouping and colour, ported verbatim from the legacy page's algorithm so the
// tiles lay out and colour identically.

export interface Tile {
  id?: string | null
  symbol: string
  exchange?: string
  currency?: string
  name?: string
  value: number
  percentChange: number | null
  sector: string
  other?: boolean
}

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

const wmean = (xs: Tile[]): number | null => {
  const q = xs.filter((x) => x.percentChange != null)
  const v = q.reduce((s, x) => s + x.value, 0)
  return v ? q.reduce((s, x) => s + x.value * (x.percentChange as number), 0) / v : null
}

export interface Block { label: string; x: number; y: number; w: number; h: number; chg: number | null }
export interface HeatCell extends Tile { v: number; x: number; y: number; w: number; h: number; big: boolean; mid: boolean }

export function heatmapLayout(tiles: Tile[], W: number, H: number): { blocks: Block[]; cells: HeatCell[] } {
  const GAP = 6, HEAD = 19
  const bySector: Record<string, Tile[]> = {}
  tiles.forEach((t) => { (bySector[t.sector] = bySector[t.sector] || []).push(t) })
  let groups = Object.keys(bySector).map((label) => ({
    label,
    v: bySector[label].reduce((s, x) => s + x.value, 0),
    chg: wmean(bySector[label]),
    syms: bySector[label],
  }))
  const gv = groups.reduce((a, b) => a + b.v, 0)
  const gArea = W * H
  const gMinArea = Math.min(52 * Math.min(W, H), gArea * 0.22)
  const gMinV = Math.min(gv / groups.length, (gv * gMinArea) / gArea)
  groups = groups.map((g) => ({ ...g, v: Math.max(g.v, gMinV) }))
  const cells: HeatCell[] = []
  const blocks: Block[] = squarify(groups as WithV<(typeof groups)[number]>[], { x: 0, y: 0, w: W, h: H }).map((g) => {
    const inner = { x: g.x + GAP / 2, y: g.y + GAP / 2 + HEAD, w: Math.max(1, g.w - GAP), h: Math.max(1, g.h - GAP - HEAD) }
    let syms: (Tile & { v: number })[] = g.syms.map((t) => ({ ...t, v: t.value }))
    const floor = g.v * 0.015
    const small = syms.filter((x) => x.v < floor)
    if (small.length > 1) {
      syms = syms.filter((x) => x.v >= floor)
      const sv = small.reduce((a, b) => a + b.v, 0)
      syms.push({ id: null, symbol: 'Other (' + small.length + ')', value: sv, v: sv, percentChange: wmean(small), sector: g.label, other: true })
    }
    const tv = syms.reduce((a, b) => a + b.v, 0)
    const area = inner.w * inner.h
    const minArea = Math.min(52 * Math.min(inner.w, inner.h), area * 0.22)
    const minV = Math.min(tv / syms.length, (tv * minArea) / area)
    syms = syms.map((x) => ({ ...x, v: Math.max(x.v, minV) }))
    squarify(syms, inner).forEach((c) => {
      const big = c.w > 128 && c.h > 62
      const mid = c.w > 78 && c.h > 40
      cells.push({ ...(c as HeatCell), big, mid })
    })
    return { label: g.label, x: g.x, y: g.y, w: g.w, h: g.h, chg: g.chg }
  })
  return { blocks, cells }
}

const HEAT: Record<string, string> = {
  p1: '#2b3a36', p2: '#2c6b51', p3: '#2f9e6b', p4: '#4fc98d',
  n1: '#3a2b31', n2: '#6e2f3d', n3: '#a8455a', n4: '#d4586f',
}
export function heatColor(chg: number | null): string {
  if (chg == null || !isFinite(chg)) return 'rgba(139,147,167,0.10)'
  const a = Math.abs(chg)
  const step = a < 0.35 ? 1 : a < 1 ? 2 : a < 2.5 ? 3 : 4
  return HEAT[(chg >= 0 ? 'p' : 'n') + step]
}
