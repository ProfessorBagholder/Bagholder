import { describe, it, expect } from 'vitest'
import { applyOps, numbering, reconcile, rowKey, type Op } from './live'

// The page holds each entity as one object for as long as the entity lives, and a
// change is written into it (docs/architecture.md, rule 0). These hold that: after
// a change, everything that did not change is the very same object it was.

const book = () => ({
  kpi: { realized: 1200.5 },
  positions: [
    { id: 'p:QNC', symbol: 'QNC', last: 1.75, mv: 175, tags: ['core'] },
    { id: 'p:CH', symbol: 'CH', last: 0.2, mv: 400, tags: [] },
  ],
  positionsSummary: { mv: 575 },
  markets: { news: [{ id: 'n1', headline: 'one' }, { id: 'n2', headline: 'two' }] },
})

describe('a change is written into the objects already held', () => {
  it('one price moving touches that holding and the totals, and nothing else is a new object', () => {
    const m = book()
    const [qnc, ch] = m.positions
    const { positions, markets, kpi } = m
    const row = { k: 'id', v: 'p:QNC' }
    const touched = applyOps(m, [
      ['set', ['positions', row, 'last'], 1.8],
      ['set', ['positions', row, 'mv'], 180],
      ['set', ['positionsSummary', 'mv'], 580],
    ] as Op[])
    expect(qnc.last).toBe(1.8)
    expect(m.positionsSummary.mv).toBe(580)
    expect(m.positions).toBe(positions) // the list is the same list
    expect(m.positions[0]).toBe(qnc) // the holding is the same object, written into
    expect(m.positions[1]).toBe(ch)
    expect(ch).toEqual({ id: 'p:CH', symbol: 'CH', last: 0.2, mv: 400, tags: [] })
    expect(m.markets).toBe(markets)
    expect(m.kpi).toBe(kpi)
    expect([...touched]).toEqual(['p:QNC'])
  })

  it('a headline arriving is one row inserted; the rows that were there are the same rows', () => {
    const m = book()
    const list = m.markets.news
    const [n1, n2] = list
    applyOps(m, [['rows', ['markets', 'news'], 'id', ['n3', 'n1', 'n2'], { n3: { id: 'n3', headline: 'three' } }]] as Op[])
    expect(m.markets.news).toBe(list)
    expect(list.map((r) => r.id)).toEqual(['n3', 'n1', 'n2'])
    expect(list[1]).toBe(n1)
    expect(list[2]).toBe(n2)
  })

  it('a row leaving takes only itself', () => {
    const m = book()
    const ch = m.positions[1]
    applyOps(m, [['rows', ['positions'], 'id', ['p:CH'], {}]] as Op[])
    expect(m.positions).toEqual([ch])
    expect(m.positions[0]).toBe(ch)
  })

  it('a field gained and a field lost', () => {
    const m = book() as ReturnType<typeof book> & { kpi: Record<string, unknown> }
    applyOps(m, [['set', ['kpi', 'winRate'], 0.5], ['del', ['kpi', 'realized']]] as Op[])
    expect(m.kpi).toEqual({ winRate: 0.5 })
  })

  it('a change to something no longer there is passed over', () => {
    const m = book()
    expect(() => applyOps(m, [['set', ['positions', { k: 'id', v: 'gone' }, 'last'], 9], ['set', ['nowhere', 'x'], 1]] as Op[])).not.toThrow()
  })
})

describe('a whole view arriving over one already shown', () => {
  it('keeps every object that is still there and writes only what differs', () => {
    const m = book()
    const [qnc, ch] = m.positions
    const next = book()
    next.positions = [{ ...next.positions[1], last: 0.21 }, { id: 'p:NEW', symbol: 'NEW', last: 5, mv: 50, tags: [] }]
    next.kpi.realized = 1300
    reconcile(m as never, next as never)
    expect(m.positions.map((p) => p.id)).toEqual(['p:CH', 'p:NEW'])
    expect(m.positions[0]).toBe(ch) // the holding that stayed is the object it was
    expect(ch.last).toBe(0.21)
    expect(m.positions).not.toContain(qnc)
    expect(m.kpi.realized).toBe(1300)
  })

  it('the same view again changes no object', () => {
    const m = book()
    const before = { positions: m.positions, qnc: m.positions[0], news: m.markets.news, tags: m.positions[0].tags }
    reconcile(m as never, book() as never)
    expect(m.positions).toBe(before.positions)
    expect(m.positions[0]).toBe(before.qnc)
    expect(m.markets.news).toBe(before.news)
    expect(m.positions[0].tags).toBe(before.tags)
  })
})

describe('what tells rows apart', () => {
  it('is the first usual field every row has and no two share', () => {
    expect(rowKey([{ id: 'a' }, { id: 'b' }])).toBe('id')
    expect(rowKey([{ symbol: 'CH', v: 1 }, { symbol: 'CH', v: 2 }])).toBe(null) // rows that repeat
    expect(rowKey(['a', 'b'])).toBe(null)
    expect(rowKey([{ d: '2026-01-01' }, { d: '2026-01-02' }])).toBe('d')
  })
})

describe('the numbers of a stream\'s messages', () => {
  it('a number that is not the next asks for the whole state; a new connection counts from 1', () => {
    let gaps = 0
    const seen = numbering(() => gaps++)
    for (const id of ['1', '2', '3']) seen(id)
    expect(gaps).toBe(0)
    seen('5') // 4 was lost
    expect(gaps).toBe(1)
    seen('6')
    expect(gaps).toBe(1)
    seen('1') // the browser connected again: a new stream
    seen('2')
    expect(gaps).toBe(1)
    seen('') // a message without a number (the keep-alive) says nothing
    seen('3')
    expect(gaps).toBe(1)
  })
})
