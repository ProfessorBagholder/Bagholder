import { render } from '@testing-library/svelte'
import { describe, it, expect, vi, beforeEach } from 'vitest'

// Record calls to the charting library so we can assert the island's lifecycle
// without a real canvas.
const createChartSpy = vi.fn()
const setDataSpy = vi.fn()
const removeSpy = vi.fn()
const fitContentSpy = vi.fn()

vi.mock('lightweight-charts', () => ({
  AreaSeries: 'AreaSeries',
  createChart: (...args: unknown[]) => {
    createChartSpy(...args)
    return {
      addSeries: () => ({ setData: setDataSpy, priceScale: () => ({ applyOptions: () => {} }) }),
      timeScale: () => ({ fitContent: fitContentSpy }),
      applyOptions: () => {},
      remove: removeSpy,
    }
  },
}))

import ChartHarness from './ChartHarness.svelte'

describe('equityChart island lifecycle', () => {
  beforeEach(() => {
    createChartSpy.mockClear()
    setDataSpy.mockClear()
    removeSpy.mockClear()
    fitContentSpy.mockClear()
  })

  it('creates the chart once, updates in place on a data change, and tears down on unmount', async () => {
    const p1 = [{ d: '2024-01-01', v: 100, dep: 0 }]
    const p2 = [
      { d: '2024-01-01', v: 100, dep: 0 },
      { d: '2024-01-02', v: 110, dep: 0 },
    ]

    const { rerender, unmount } = render(ChartHarness, { props: { points: p1 } })
    expect(createChartSpy).toHaveBeenCalledTimes(1)
    expect(setDataSpy).toHaveBeenCalledTimes(1)

    // A model update flows through the action's update() — the chart is NOT
    // recreated. This is the exact guarantee the legacy positional reconciler
    // could not give: it stranded/rebuilt externally-mounted nodes across a
    // redraw (the v1.46.3 heatmap and v1.46.2 tab-underline regressions).
    await rerender({ points: p2 })
    expect(createChartSpy).toHaveBeenCalledTimes(1) // still one — no recreate
    expect(setDataSpy).toHaveBeenCalledTimes(2) // updated in place
    expect(removeSpy).not.toHaveBeenCalled()

    unmount()
    expect(removeSpy).toHaveBeenCalledTimes(1) // cleaned up exactly once
  })
})
