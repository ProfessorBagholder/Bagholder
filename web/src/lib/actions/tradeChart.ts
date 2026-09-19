import {
  createChart,
  CandlestickSeries,
  createSeriesMarkers,
  type IChartApi,
  type ISeriesApi,
  type ISeriesMarkersPluginApi,
  type Time,
} from 'lightweight-charts'

export interface Bar { date: string; open: number; high: number; low: number; close: number; volume: number }
export interface Fill { date: string; qty: number; price: number | null }

// The trade chart as a Svelte `use:` action — candlesticks with each execution
// marked (up arrow below for a buy, down arrow above for a sell). Created once,
// updated in place when the bars or fills change (a timeframe switch), torn down
// on destroy. The same island-lifecycle guarantee as the equity chart.
export function tradeChart(node: HTMLElement, params: { bars: Bar[]; fills: Fill[] }) {
  const chart: IChartApi = createChart(node, {
    layout: { background: { color: 'transparent' }, textColor: '#8b93a7', attributionLogo: true },
    grid: { horzLines: { color: '#1c2230' }, vertLines: { color: '#141924' } },
    rightPriceScale: { borderColor: '#1c2230' },
    timeScale: { borderColor: '#1c2230', timeVisible: true },
    height: node.clientHeight || 320,
    width: node.clientWidth || 700,
  })
  const series: ISeriesApi<'Candlestick'> = chart.addSeries(CandlestickSeries, {
    upColor: '#3ecf8e',
    downColor: '#f0616d',
    borderVisible: false,
    wickUpColor: '#3ecf8e',
    wickDownColor: '#f0616d',
  })
  let markerApi: ISeriesMarkersPluginApi<Time> | null = null

  function draw(p: { bars: Bar[]; fills: Fill[] }) {
    series.setData(
      p.bars.map((b) => ({ time: b.date as Time, open: b.open, high: b.high, low: b.low, close: b.close })),
    )
    const markers = p.fills
      .filter((f) => f.price != null)
      .map((f) => ({
        time: f.date as Time,
        position: (f.qty >= 0 ? 'belowBar' : 'aboveBar') as 'belowBar' | 'aboveBar',
        color: f.qty >= 0 ? '#3ecf8e' : '#f0616d',
        shape: (f.qty >= 0 ? 'arrowUp' : 'arrowDown') as 'arrowUp' | 'arrowDown',
        text: `${f.qty >= 0 ? '+' : ''}${f.qty} @ ${f.price}`,
      }))
      .sort((a, b) => String(a.time).localeCompare(String(b.time)))
    if (markerApi) markerApi.setMarkers(markers)
    else markerApi = createSeriesMarkers(series, markers)
    chart.timeScale().fitContent()
  }
  draw(params)

  const ro = new ResizeObserver(() => chart.applyOptions({ width: node.clientWidth, height: node.clientHeight }))
  ro.observe(node)

  return {
    update(next: { bars: Bar[]; fills: Fill[] }) {
      draw(next)
    },
    destroy() {
      ro.disconnect()
      chart.remove()
    },
  }
}
