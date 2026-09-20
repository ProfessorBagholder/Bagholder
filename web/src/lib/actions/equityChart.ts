import { createChart, AreaSeries, type IChartApi, type ISeriesApi, type Time } from 'lightweight-charts'
import type { EquityPoint } from '../model'

// The equity curve as a Svelte `use:` action — the pattern that replaces the
// legacy MORPH_SKIP set and the keepChart/keepHeat/keepLogin/keepTab dance.
// The framework never reconciles inside an action-owned node: the chart is
// created ONCE on mount, updated in place when the data changes, and torn down
// on destroy. There is no way for a redraw to strand or recreate it — which is
// exactly the bug class that produced the v1.46.3 heatmap and v1.46.2 tab-underline
// regressions.
export function equityChart(node: HTMLElement, points: EquityPoint[]) {
  const chart: IChartApi = createChart(node, {
    layout: { background: { color: 'transparent' }, textColor: '#8b93a7', attributionLogo: false },
    grid: { horzLines: { color: '#1c2230' }, vertLines: { color: 'transparent' } },
    rightPriceScale: { borderColor: '#1c2230' },
    timeScale: { borderColor: '#1c2230' },
    height: node.clientHeight || 260,
    width: node.clientWidth || 600,
    handleScale: false,
    handleScroll: false,
  })
  const series: ISeriesApi<'Area'> = chart.addSeries(AreaSeries, {
    lineColor: '#3ecf8e',
    topColor: 'rgba(62,207,142,0.25)',
    bottomColor: 'rgba(62,207,142,0.02)',
    lineWidth: 2,
    priceLineVisible: false,
  })
  const set = (pts: EquityPoint[]) => series.setData(pts.map((p) => ({ time: p.d as Time, value: p.v })))
  set(points)
  chart.timeScale().fitContent()

  const ro = new ResizeObserver(() => chart.applyOptions({ width: node.clientWidth, height: node.clientHeight }))
  ro.observe(node)

  return {
    update(next: EquityPoint[]) {
      set(next)
      chart.timeScale().fitContent()
    },
    destroy() {
      ro.disconnect()
      chart.remove()
    },
  }
}
