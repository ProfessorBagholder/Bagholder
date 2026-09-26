import { beforeEach, describe, expect, it, vi } from 'vitest'

const made = { charts: 0, removed: 0, setData: 0, setMarkers: 0, applied: 0 }

vi.mock('lightweight-charts', () => {
  const timeScale = {
    fitContent: vi.fn(),
    setVisibleLogicalRange: vi.fn(),
    getVisibleLogicalRange: () => ({ from: 0, to: 10 }),
    subscribeVisibleLogicalRangeChange: vi.fn(),
    subscribeSizeChange: vi.fn(),
    width: vi.fn(() => 800),
  }
  ;(globalThis as Record<string, unknown>).__timeScale = timeScale
  return {
    CandlestickSeries: {},
    CrosshairMode: { Normal: 0 },
    createChart: () => {
      made.charts++
      return {
        addSeries: () => ({
          setData: () => {
            made.setData++
            // the library reports its own default span as the data goes in, as it does
            const onRange = timeScale.subscribeVisibleLogicalRangeChange.mock.calls.at(-1)?.[0]
            if (typeof onRange === 'function') onRange({ from: -133, to: 19 })
          },
          applyOptions: () => made.applied++,
        }),
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
import type { Dec } from '../dec'

const colors = { text: '#aaa', grid: '#222', up: '#0a0', down: '#a00' } as TradeChartParams['colors']
const bars = [
  { date: '2026-09-14', open: 1, high: 2, low: 1, close: 2 },
  { date: '2026-09-15', open: 2, high: 3, low: 2, close: 3 },
] as TradeChartParams['bars']
const fill = (id: string, when: string): Fill => ({ id, when, date: when.slice(0, 10), side: 'BUY', sub: '', qty: '5' as Dec, price: '1.5' as Dec, amount: '-7.5' as Dec, currency: 'CAD', flags: [] })

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

  it('frames the trade once the chart has its width, and keeps no range from before', () => {
    const ts = (globalThis as Record<string, any>).__timeScale
    ts.setVisibleLogicalRange.mockClear()
    ts.width.mockReturnValue(0) // laid out later: autoSize has not measured the element yet
    const days = Array.from({ length: 20 }, (_, i) => ({ date: `2026-08-${String(i + 1).padStart(2, '0')}`, open: 1, high: 2, low: 1, close: 2 })) as TradeChartParams['bars']
    const node = document.createElement('div')
    tradeChart(node, { bars: days, fills: [fill('a', '2026-08-06T14:00:00Z'), fill('b', '2026-08-16T14:00:00Z')], tf: '1d', colors, rangeKey: 'framed|1d' })
    const onRange = ts.subscribeVisibleLogicalRangeChange.mock.calls.at(-1)[0]
    const onSize = ts.subscribeSizeChange.mock.calls.at(-1)[0]
    onRange({ from: -150, to: 19 }) // the chart's own default at no width: not the person's range
    ts.setVisibleLogicalRange.mockClear()
    ts.width.mockReturnValue(1300)
    onSize(1300, 300)
    // the executions at bars 5 and 15, four bars either side
    expect(ts.setVisibleLogicalRange).toHaveBeenCalledWith({ from: 1, to: 19 })
    // framed now: a later size change leaves the range to the person
    ts.setVisibleLogicalRange.mockClear()
    onSize(1200, 300)
    expect(ts.setVisibleLogicalRange).not.toHaveBeenCalled()
    ts.width.mockReturnValue(800)
  })

  it('frames the trade, not the span the library reports while the bars go in', () => {
    const ts = (globalThis as Record<string, any>).__timeScale
    ts.setVisibleLogicalRange.mockClear()
    const days = Array.from({ length: 20 }, (_, i) => ({ date: `2026-07-${String(i + 1).padStart(2, '0')}`, open: 1, high: 2, low: 1, close: 2 })) as TradeChartParams['bars']
    tradeChart(document.createElement('div'), { bars: days, fills: [fill('a', '2026-07-06T14:00:00Z'), fill('b', '2026-07-16T14:00:00Z')], tf: '1d', colors, rangeKey: 'settling|1d' })
    expect(ts.setVisibleLogicalRange).toHaveBeenLastCalledWith({ from: 1, to: 19 })
  })
})
