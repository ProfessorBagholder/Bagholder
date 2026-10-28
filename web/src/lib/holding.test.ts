// A holding no round trip of the journal is open in (brief 79: a holding is what
// Wealthsimple states, the journal linked where it has one) has no days held: its
// Hold is a dash, never 0d.

import { describe, expect, it } from 'vitest'
import { holdingAsTrade, holdText } from './holding'
import { hold } from './fmt'
import type { Position } from './model'
import month from './fixtures/figures_pulled_month.json'

const positions = (month as unknown as { positions: { positions: Position[] } }).positions.positions

describe('a holding on the page a trade opens', () => {
  it('shows no days held where the journal holds no lot of it', () => {
    const untraded = positions.filter((p) => !p.trade)
    expect(untraded.length).toBeGreaterThan(0)
    for (const p of untraded) expect(holdText(holdingAsTrade(p))).toBe('—')
  })

  it('shows the days held of the journal\'s lots where it holds them', () => {
    const traded = positions.filter((p) => p.trade && typeof p.held === 'number')
    expect(traded.length).toBeGreaterThan(0)
    for (const p of traded) expect(holdText(holdingAsTrade(p))).toBe(hold(p.held as number))
  })
})
