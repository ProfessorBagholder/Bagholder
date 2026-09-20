import { beforeEach, describe, expect, it, vi } from 'vitest'

const made = { charts: 0, removed: 0, setData: 0, setMarkers: 0, applied: 0 }

vi.mock('lightweight-charts', () => {
  const timeScale = { fitContent: vi.fn(), setVisibleLogicalRange: vi.fn(), getVisibleLogicalRange: () => ({ from: 0, to: 10 }), subscribeVisibleLogicalRangeChange: vi.fn() }
  return {
    CandlestickSeries: {},
    CrosshairMode: { Normal: 0 },
    createChart: () => {
      made.charts++
      return {
        addSeries: () => ({ setData: () => made.setData++, applyOptions: () => made.applied++ }),
        applyOptions: () => made.applied++,
        timeScale: () => timeScale,
        remove: () => made.removed++,
      }
    },
    createSeriesMarkers: () => ({ setMarkers: () => made.setMarkers++ }),
  }
})

import { tradeChart, type TradeChartParams } from './tradeChart'
import type { Fill } from '../model'

const colors = { text: '#aaa', grid: '#222', up: '#0a0', down: '#a00' } as TradeChartParams['colors']
const bars = [
  { date: '2026-09-14', open: 1, high: 2, low: 1, close: 2 },
  { date: '2026-09-15', open: 2, high: 3, low: 2, close: 3 },
] as TradeChartParams['bars']
const fill = (id: string, when: string): Fill => ({ id, when, date: when.slice(0, 10), time: '', side: 'BUY', sub: '', qty: 5, price: 1.5, amount: -7.5, fees: 0, currency: 'CAD', flags: [] })

beforeEach(() => Object.keys(made).forEach((k) => ((made as Record<string, number>)[k] = 0)))

describe('the trade chart', () => {
  it('is made once and an update redoes only what changed', () => {
    vi.stubGlobal('requestAnimationFrame', (f: () => void) => f())
    const node = document.createElement('div')
    const first: TradeChartParams = { bars, fills: [fill('a', '2026-09-14T14:00:00Z')], tf: '1d', colors, rangeKey: 't1|1d' }
    const chart = tradeChart(node, first)
    expect(made).toMatchObject({ charts: 1, setData: 1 })
    const markersAtStart = made.setMarkers

    // the card around it was given a new price: the same bars and fills in a new object
    chart.update({ ...first, fills: [fill('a', '2026-09-14T14:00:00Z')] })
    expect(made).toMatchObject({ charts: 1, removed: 0, setData: 1, setMarkers: markersAtStart, applied: 0 })

    // another execution: new markers on the same chart and the same bars
    chart.update({ ...first, fills: [fill('a', '2026-09-14T14:00:00Z'), fill('b', '2026-09-15T15:00:00Z')] })
    expect(made).toMatchObject({ charts: 1, removed: 0, setData: 1 })
    expect(made.setMarkers).toBeGreaterThan(markersAtStart)

    // another timeframe: new data, still the same chart
    chart.update({ ...first, bars: [...bars], tf: '1h', rangeKey: 't1|1h' })
    expect(made).toMatchObject({ charts: 1, removed: 0, setData: 2 })

    // the theme changed: options, not a new chart
    chart.update({ ...first, bars: [...bars], tf: '1h', rangeKey: 't1|1h', colors: { ...colors, text: '#111' } })
    expect(made.charts).toBe(1)
    expect(made.applied).toBeGreaterThan(0)

    chart.destroy()
    expect(made.removed).toBe(1)
  })
})
